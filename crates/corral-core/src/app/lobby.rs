use std::sync::Arc;

use uuid::Uuid;

use super::ssh_access::active_login;
use super::{Catalog, CreateSandbox, Environment, SandboxView, Sandboxes};
use crate::domain::{
    Entry, EntryId, Handle, HomePath, ImageKey, SandboxDefaults, SandboxName, SizeKey, Target,
};
use crate::ports::UserRepo;
use crate::{Error, Result};

/// Where a connection to a sandbox that does not exist yet chooses how it is made, and makes it.
pub struct Lobby {
    users: Arc<dyn UserRepo>,
    sandboxes: Sandboxes,
    environment: Environment,
}

/// Where a login with a terminal lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entrance {
    /// It names a sandbox that exists: arriving goes on from there.
    Sandbox,
    /// It names none yet, or names only its user, who has none.
    Lobby(Offer),
    /// Why the login reaches no sandbox, for the person who made it.
    Refused(String),
}

/// What the lobby starts from: the owner, the name the login gives the new sandbox if any, the
/// owner's defaults and their environment entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub owner: Handle,
    pub name: Option<SandboxName>,
    pub defaults: SandboxDefaults,
    pub entries: Vec<Entry>,
}

/// How a new sandbox is made. Without an image or a size, the owner's default; without a list
/// of entries, those that apply to new sandboxes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    /// A catalog image's key, or else an image reference.
    pub image: Option<String>,
    pub size: Option<String>,
    /// Each entry by its id, or by the variable or file it sets.
    pub environment: Option<Vec<String>>,
}

/// A sandbox the lobby made, and the entries attached to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    pub view: SandboxView,
    pub attached: Vec<Entry>,
}

impl Lobby {
    #[must_use]
    pub fn new(users: Arc<dyn UserRepo>, sandboxes: Sandboxes, environment: Environment) -> Self {
        Self {
            users,
            sandboxes,
            environment,
        }
    }

    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        self.sandboxes.catalog()
    }

    /// Where the login lands: its sandbox when it has one by that name, or by no name any sandbox
    /// at all; else the lobby, or why neither.
    pub async fn enter(&self, username: &str) -> Result<Entrance> {
        let (owner, name) = match active_login(self.users.as_ref(), username).await? {
            Ok(login) => login,
            Err(why) => return Ok(Entrance::Refused(why)),
        };
        let name = match name.map(SandboxName::parse).transpose() {
            Ok(name) => name,
            Err(Error::Invalid(why)) => return Ok(Entrance::Refused(why)),
            Err(error) => return Err(error),
        };
        let exists = match &name {
            Some(name) => self.sandboxes.repo.by_name(owner.id, name).await?.is_some(),
            None => !self.sandboxes.repo.list(owner.id).await?.is_empty(),
        };
        if exists {
            return Ok(Entrance::Sandbox);
        }
        let entries = self.environment.list(&owner).await?;
        Ok(Entrance::Lobby(Offer {
            defaults: self.sandboxes.defaults(&owner).await?.defaults,
            owner: owner.handle,
            name,
            entries: entries.into_iter().map(|view| view.entry).collect(),
        }))
    }

    /// Refuses an image reference the catalog would not make a sandbox from.
    pub async fn check_image(&self, reference: &str) -> Result<()> {
        self.sandboxes.check_named_image(reference).await
    }

    /// Makes the sandbox the login names, which must not exist yet, and starts it. Refuses with
    /// `Invalid` and the reason to show, or `Conflict` when it exists.
    pub async fn create(&self, username: &str, choice: Choice) -> Result<Made> {
        let (owner, name) = active_login(self.users.as_ref(), username)
            .await?
            .map_err(Error::Invalid)?;
        let name = SandboxName::parse(name.ok_or_else(|| {
            Error::Invalid(format!(
                "a new sandbox needs a name: connect as <name>.{}",
                owner.handle
            ))
        })?)?;
        if self
            .sandboxes
            .repo
            .by_name(owner.id, &name)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!("sandbox {name} exists already")));
        }
        let (image, image_reference) = match choice.image {
            None => (None, None),
            Some(text) => match self.catalog_key(&text) {
                Some(key) => (Some(key), None),
                None => (None, Some(text)),
            },
        };
        let environment = match choice.environment {
            None => None,
            Some(wanted) => Some(chosen(&self.environment.list(&owner).await?, &wanted)?),
        };
        let request = CreateSandbox {
            name: name.clone(),
            image,
            image_reference,
            size: choice.size.as_deref().map(SizeKey::parse).transpose()?,
            environment,
        };
        let view = self
            .sandboxes
            .create(&owner, request)
            .await
            .map_err(|error| match error {
                Error::LimitReached(why) => {
                    Error::Invalid(format!("cannot make sandbox {name}: {why}"))
                }
                Error::Conflict(_) => Error::Conflict(format!("sandbox {name} exists already")),
                error => error,
            })?;
        let attached = self.environment.of_sandbox(&owner, &name).await?;
        Ok(Made {
            view,
            attached: attached
                .into_iter()
                .map(|attached| attached.view.entry)
                .collect(),
        })
    }

    fn catalog_key(&self, text: &str) -> Option<ImageKey> {
        let key = ImageKey::parse(text).ok()?;
        self.catalog().image(Some(&key)).ok().map(|_| key)
    }
}

/// The entries `wanted` names, each by its id or by its target.
fn chosen(entries: &[crate::app::EntryView], wanted: &[String]) -> Result<Vec<EntryId>> {
    let mut ids = Vec::new();
    for named in wanted {
        let id = Uuid::parse_str(named).ok();
        let matching: Vec<&Entry> = entries
            .iter()
            .map(|view| &view.entry)
            .filter(|entry| Some(entry.id.0) == id || names(&entry.target, named))
            .collect();
        let entry = match matching.as_slice() {
            [entry] => entry,
            [] => {
                return Err(Error::Invalid(format!(
                    "there is no environment entry {named:?}"
                )));
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "{named:?} names more than one environment entry: give its id"
                )));
            }
        };
        if !ids.contains(&entry.id) {
            ids.push(entry.id);
        }
    }
    Ok(ids)
}

fn names(target: &Target, named: &str) -> bool {
    match target {
        Target::Variable(variable) => variable.as_str() == named,
        Target::File(path) => HomePath::parse(named).is_ok_and(|named| named == *path),
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, Entrance, Offer};
    use crate::Error;
    use crate::app::{EntryChange, NewEntry};
    use crate::domain::{Entry, SandboxName, User};
    use crate::fakes::{
        MAX_SANDBOXES, World, admin_proof, defaults, entry, exposed, file, unchanged, variable,
    };

    /// Adam, with `main`, a secret variable for new sandboxes and a plain one that is not.
    async fn adam() -> (World, User, Entry) {
        let w = World::new();
        let adam = w.member("adam").await;
        w.sandbox(&adam, "main").await;
        w.entry(&adam, variable("GH_TOKEN"), "shared", true).await;
        let extra = w.entry(&adam, variable("EXTRA"), "extra", false).await;
        let change = EntryChange {
            apply_to_new: false,
            ..unchanged(&extra)
        };
        let extra = w
            .environment
            .change(&adam, extra.id, change)
            .await
            .unwrap()
            .entry;
        (w, adam, extra)
    }

    fn offer(entrance: Entrance) -> Offer {
        match entrance {
            Entrance::Lobby(offer) => offer,
            other => panic!("not the lobby: {other:?}"),
        }
    }

    fn refusal(error: Error) -> String {
        match error {
            Error::Invalid(why) | Error::Conflict(why) => why,
            other => panic!("{other:?}"),
        }
    }

    fn targets(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| entry.target.to_string())
            .collect()
    }

    fn choice(image: Option<&str>, size: Option<&str>, entries: Option<&[&str]>) -> Choice {
        Choice {
            image: image.map(str::to_owned),
            size: size.map(str::to_owned),
            environment: entries.map(|entries| entries.iter().map(|&e| e.to_owned()).collect()),
        }
    }

    #[tokio::test]
    async fn a_login_naming_a_sandbox_that_exists_goes_on_and_one_naming_none_opens_the_lobby() {
        let (w, adam, _) = adam().await;
        w.sandboxes
            .set_defaults(&adam, &defaults("tools", "large"))
            .await
            .unwrap();
        let calls = w.engine.calls().len();
        for login in ["main.adam", "adam"] {
            assert_eq!(w.lobby.enter(login).await.unwrap(), Entrance::Sandbox);
        }
        let offer = offer(w.lobby.enter("new.adam").await.unwrap());
        assert_eq!(
            (
                offer.owner.as_str(),
                offer.name.as_ref().map(SandboxName::as_str)
            ),
            ("adam", Some("new"))
        );
        assert_eq!(offer.defaults, defaults("tools", "large"));
        assert_eq!(
            targets(&offer.entries),
            ["variable EXTRA", "variable GH_TOKEN"]
        );
        assert_eq!(w.engine.calls().len(), calls, "the engine was not asked");
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_bare_login_of_a_user_with_no_sandbox_opens_the_lobby_without_a_name() {
        let w = World::new();
        w.member("cain").await;
        let offer = offer(w.lobby.enter("cain").await.unwrap());
        assert_eq!((offer.owner.as_str(), offer.name), ("cain", None));
    }

    #[tokio::test]
    async fn the_lobby_refuses_as_arriving_does() {
        let (w, ..) = adam().await;
        let root = w.admin("root").await;
        let eve = w.member("eve").await;
        w.accounts
            .set_enabled(&admin_proof(&root), eve.id, false)
            .await
            .unwrap();
        for (login, why) in [
            ("new.eve", "this account is disabled"),
            (
                "Main.adam",
                "sandbox name \"Main\" must match [a-z0-9-]{1,32}",
            ),
            ("new.nobody", "there is no user nobody"),
            ("main+adam", "\"main+adam\" is not a Corral login"),
        ] {
            assert_eq!(
                w.lobby.enter(login).await.unwrap(),
                Entrance::Refused(why.to_owned()),
                "{login}"
            );
        }
    }

    #[tokio::test]
    async fn the_lobby_makes_a_sandbox_from_the_choice_with_exactly_the_entries_chosen() {
        let (w, _, extra) = adam().await;
        let id = extra.id.0.to_string();
        let made = w
            .lobby
            .create(
                "new.adam",
                choice(Some("tools"), Some("large"), Some(&["EXTRA", &id])),
            )
            .await
            .unwrap();
        let sandbox = &made.view.sandbox;
        assert_eq!(
            (sandbox.name.as_str(), sandbox.size.as_str()),
            ("new", "large")
        );
        assert_eq!(sandbox.image.to_string(), "tools");
        assert_eq!(targets(&made.attached), ["variable EXTRA"]);
        assert_eq!(
            exposed(&w.engine.env_of(&sandbox.id).unwrap()),
            ["EXTRA=extra"]
        );
    }

    #[tokio::test]
    async fn an_empty_choice_takes_the_defaults_and_the_entries_for_new_sandboxes() {
        let (w, adam, _) = adam().await;
        let made = w.lobby.create("new.adam", Choice::default()).await.unwrap();
        let sandbox = &made.view.sandbox;
        assert_eq!(
            (sandbox.image.to_string().as_str(), sandbox.size.as_str()),
            ("base", "small")
        );
        assert_eq!(targets(&made.attached), ["variable GH_TOKEN"]);
        let none = w
            .lobby
            .create("bare.adam", choice(None, None, Some(&[])))
            .await
            .unwrap();
        assert_eq!(targets(&none.attached), [] as [String; 0]);
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn the_lobby_refuses_a_bad_choice_and_makes_nothing() {
        let (w, adam, _) = adam().await;
        let template = NewEntry {
            template: true,
            apply_to_new: false,
            ..entry(file(".config/tool.toml"), "token = \"{{ EXTRA }}\"")
        };
        w.environment.create(&adam, template).await.unwrap();
        let calls = w.engine.calls().len();
        for (username, choice, why) in [
            (
                "new.adam",
                choice(None, Some("huge"), None),
                "unknown size \"huge\"",
            ),
            (
                "new.adam",
                choice(Some("ghcr.io/acme/tools:1"), None, None),
                "sandboxes run only the catalog's images here",
            ),
            (
                "new.adam",
                choice(None, None, Some(&["NOPE"])),
                "there is no environment entry \"NOPE\"",
            ),
            (
                "new.adam",
                choice(None, None, Some(&["~/.config/tool.toml"])),
                "template ~/.config/tool.toml refers to EXTRA: choose it too, or leave the \
                 template out",
            ),
            (
                "main.adam",
                choice(None, None, None),
                "sandbox main exists already",
            ),
            (
                "adam",
                choice(None, None, None),
                "a new sandbox needs a name: connect as <name>.adam",
            ),
        ] {
            let error = w.lobby.create(username, choice).await.unwrap_err();
            assert_eq!(refusal(error), why, "{username}");
        }
        assert_eq!(w.engine.calls().len(), calls, "the engine was not asked");
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 1);

        let made = w
            .lobby
            .create(
                "new.adam",
                choice(None, None, Some(&[".config/tool.toml", "EXTRA"])),
            )
            .await
            .unwrap();
        assert_eq!(
            targets(&made.attached),
            ["variable EXTRA", "file .config/tool.toml"]
        );
    }

    #[tokio::test]
    async fn a_name_two_entries_share_is_refused_for_its_id() {
        let (w, adam, _) = adam().await;
        let same_name = NewEntry {
            apply_to_new: false,
            ..entry(file("EXTRA"), "a file")
        };
        w.environment.create(&adam, same_name).await.unwrap();
        let error = w
            .lobby
            .create("new.adam", choice(None, None, Some(&["EXTRA"])))
            .await
            .unwrap_err();
        assert_eq!(
            refusal(error),
            "\"EXTRA\" names more than one environment entry: give its id"
        );
    }

    #[tokio::test]
    async fn past_the_owners_limit_the_lobby_says_why_and_makes_nothing() {
        let (w, adam, _) = adam().await;
        for n in 1..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        let error = w
            .lobby
            .create("new.adam", Choice::default())
            .await
            .unwrap_err();
        assert_eq!(
            refusal(error),
            format!("cannot make sandbox new: at most {MAX_SANDBOXES} sandboxes per user")
        );
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), MAX_SANDBOXES);
    }
}
