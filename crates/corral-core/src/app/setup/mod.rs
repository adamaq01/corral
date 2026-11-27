mod adoption;
mod compose;
mod registry;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use secrecy::{ExposeSecret, SecretSlice};
use tracing::{debug, info};

use compose::{Composed, amount, compose, template_of, unresolved};
pub use compose::{FILE_LIMIT, FILES_LIMIT, VARIABLE_LIMIT, VARIABLES_LIMIT};
use registry::{Registry, Scope, by_target};

use super::inside::SandboxLinks;
use crate::domain::{
    Admin, Attachment, Entry, EntryId, Glimpse, Handle, HomePath, Kept, Owner, Repository, Sandbox,
    SandboxId, SandboxName, SandboxToken, SecretId, SecretRef, Shared, StoredValue, Target,
    Template, User, UserId, VariableName,
};
use crate::ports::{Boot, Changed, SandboxRepo, SecretStore, SetupRepo, UserRepo};
use crate::{Error, Result};

/// The directory in the home where Corral keeps the sandbox's own files.
const OWN_DIR: &str = ".config/corral";
/// What a preview shows in place of a secret variable's value.
const MASK: &str = "••••••••";

/// Each member's variables and files, those the admin shares with everyone, and the sandboxes
/// they are attached to.
#[derive(Clone)]
pub struct Setup {
    repo: Arc<dyn SetupRepo>,
    secrets: Arc<dyn SecretStore>,
    sandboxes: Arc<dyn SandboxRepo>,
    users: Arc<dyn UserRepo>,
    links: SandboxLinks,
}

/// An entry as its owner first gives it.
#[allow(clippy::struct_excessive_bools)]
pub struct NewEntry {
    pub target: Target,
    pub secret: bool,
    pub template: bool,
    pub executable: bool,
    pub apply_to_new: bool,
    pub value: SecretSlice<u8>,
}

/// What an entry becomes: its target, kind and flags as its owner sets them, and its value
/// unless `None`. Its attachments and overrides stay.
#[allow(clippy::struct_excessive_bools)]
pub struct EntryChange {
    pub target: Target,
    pub secret: bool,
    pub template: bool,
    pub executable: bool,
    pub apply_to_new: bool,
    pub value: Option<SecretSlice<u8>>,
}

/// An entry the admin shares with everyone, as they first give it.
pub struct NewSharedEntry {
    pub shared: Shared,
    pub entry: NewEntry,
}

/// What a shared entry becomes, how it is shared included.
pub struct SharedEntryChange {
    pub shared: Shared,
    pub change: EntryChange,
}

/// An entry with the member's sandboxes it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryView {
    pub entry: Entry,
    /// Those attached to it, or every one for a required entry.
    pub sandboxes: Vec<Use>,
    /// For a template: the variables its own value refers to.
    pub references: BTreeSet<VariableName>,
    /// Whether an entry made earlier has the same target, which only entries made before
    /// targets were unique can have.
    pub duplicate: bool,
    /// For the member's own entry: whether it takes the place of an offered one.
    pub shadows: bool,
    /// For an offered entry: whether an entry of the member's takes its place.
    pub shadowed: bool,
}

/// A shared entry as the admin sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedEntryView {
    pub entry: Entry,
    pub shared: Shared,
    /// For a template: the variables its own value refers to.
    pub references: BTreeSet<VariableName>,
    /// How many sandboxes are attached to it.
    pub attached: usize,
    /// The members with an entry of their own in its place, by handle.
    pub shadowed_by: Vec<Handle>,
}

/// An entry's attachment to one sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub sandbox: SandboxName,
    pub overridden: bool,
    /// For a template: the variables it refers to there that are not attached to the sandbox.
    pub unresolved: BTreeSet<VariableName>,
}

/// An entry attached to a sandbox, with any value it has there in place of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub view: EntryView,
    pub overriding: Option<StoredValue>,
}

/// A template as one sandbox would get it, with each secret variable masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub text: String,
    /// The variables it refers to that are not attached to the sandbox, left as written.
    pub unresolved: BTreeSet<VariableName>,
}

/// Where a change lands: some of a member's sandboxes, and their new sandboxes.
#[derive(Default)]
struct Reach {
    sandboxes: BTreeSet<SandboxId>,
    new: bool,
}

impl Setup {
    #[must_use]
    pub fn new(
        repo: Arc<dyn SetupRepo>,
        secrets: Arc<dyn SecretStore>,
        sandboxes: Arc<dyn SandboxRepo>,
        users: Arc<dyn UserRepo>,
        links: SandboxLinks,
    ) -> Self {
        Self {
            repo,
            secrets,
            sandboxes,
            users,
            links,
        }
    }

    /// Every shared entry, then the member's own.
    pub async fn list(&self, owner: &User) -> Result<Vec<EntryView>> {
        let registry = self.registry(owner.id).await?;
        let ids = registry
            .shared
            .iter()
            .chain(&registry.entries)
            .map(|e| e.id);
        self.views(owner.id, &registry, ids).await
    }

    pub async fn get(&self, owner: &User, id: EntryId) -> Result<EntryView> {
        let registry = self.registry(owner.id).await?;
        self.view(owner.id, &registry, id).await
    }

    /// Attached to no sandbox yet: with `apply_to_new`, to each one created from now on.
    pub async fn create(&self, owner: &User, new: NewEntry) -> Result<EntryView> {
        let registry = self.registry(owner.id).await?;
        check_target(&registry, &new.target, None)?;
        check_shape(&new.target, new.template, new.executable)?;
        check_content(&new.target, new.template, new.value.expose_secret())?;
        let entry = Entry {
            id: EntryId::generate(),
            owner: Owner::Member(owner.id),
            template: new.template,
            executable: new.executable,
            apply_to_new: new.apply_to_new,
            value: self
                .keep(owner.id, &new.target, new.secret, &new.value)
                .await?,
            target: new.target,
            created_at: Utc::now(),
        };
        let mut proposed = registry.clone();
        proposed.put_entry(entry.clone());
        let reach = Reach {
            new: entry.apply_to_new,
            ..Reach::default()
        };
        self.settle(owner.id, [&entry.value], async {
            self.validate(owner.id, None, &registry, &proposed, &reach)
                .await?;
            self.repo.insert(&entry).await
        })
        .await?;
        self.view(owner.id, &proposed, entry.id).await
    }

    /// Changes the entry in place. Each sandbox attached to it, and new sandboxes when it
    /// applies to them, must still start; each override must suit what the entry becomes, and
    /// moves to or from the secret store with it.
    pub async fn change(
        &self,
        owner: &User,
        id: EntryId,
        change: EntryChange,
    ) -> Result<EntryView> {
        let registry = self.registry(owner.id).await?;
        let before = registry.own(id)?.clone();
        check_target(&registry, &change.target, Some(id))?;
        check_shape(&change.target, change.template, change.executable)?;
        let attachments: Vec<Attachment> = registry.attachments_of(id).cloned().collect();
        let labels = self
            .sandbox_names(owner.id)
            .await?
            .into_iter()
            .map(|(id, name)| (id, name.to_string()))
            .collect();
        let (after, overrides, rekept) = self
            .refitted(&before, &change, &attachments, &labels)
            .await?;
        let mut proposed = registry.clone();
        proposed.put_entry(after.clone());
        for attachment in &overrides {
            proposed.put_attachment(attachment.clone());
        }
        let affected = affected(&before, &after, change.value.is_some(), &attachments);
        let reach = Reach {
            sandboxes: affected.clone(),
            new: before.apply_to_new || after.apply_to_new,
        };
        self.settle(owner.id, &rekept.kept, async {
            self.validate(owner.id, None, &registry, &proposed, &reach)
                .await?;
            self.update(&after, &overrides).await
        })
        .await?;
        self.forget_all(owner.id, rekept.gone).await;
        self.changed(&affected).await?;
        self.view(owner.id, &proposed, id).await
    }

    /// Detaches the entry from every sandbox, which loses it at its next start. Refused while a
    /// template that stays would refer to it.
    pub async fn delete(&self, owner: &User, id: EntryId) -> Result<()> {
        let registry = self.registry(owner.id).await?;
        let entry = registry.own(id)?.clone();
        let attachments: Vec<Attachment> = registry.attachments_of(id).cloned().collect();
        let mut proposed = registry.clone();
        proposed.remove_entry(id);
        let reach = Reach {
            sandboxes: attachments.iter().map(|a| a.sandbox.clone()).collect(),
            new: entry.apply_to_new,
        };
        self.validate(owner.id, None, &registry, &proposed, &reach)
            .await?;
        if !self.repo.delete(&entry.owner, id).await? {
            return Err(not_found(id));
        }
        self.forget(owner.id, entry.value.secret_id()).await;
        self.forget_all(owner.id, attachments.iter().map(overriding_secret))
            .await;
        self.changed(&reach.sandboxes).await
    }

    /// What the owner's sandbox `name` gets at its next start: every required entry, then the
    /// entries attached to it.
    pub async fn of_sandbox(&self, owner: &User, name: &SandboxName) -> Result<Vec<Attached>> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let held: Vec<(EntryId, Option<StoredValue>)> = registry
            .required()
            .map(|entry| (entry.id, None))
            .chain(
                registry
                    .attached(&sandbox.id)
                    .into_iter()
                    .map(|(entry, attachment)| (entry.id, attachment.overriding.clone())),
            )
            .collect();
        let views = self
            .views(owner.id, &registry, held.iter().map(|(id, _)| *id))
            .await?;
        Ok(views
            .into_iter()
            .zip(held)
            .map(|(view, (_, overriding))| Attached { view, overriding })
            .collect())
    }

    /// Does nothing when the entry is attached already.
    pub async fn attach(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let target = registry.attachable(id)?.target.to_string();
        if registry.attachment(&sandbox.id, id).is_some() {
            return Ok(());
        }
        let attachment = Attachment {
            sandbox: sandbox.id.clone(),
            entry: id,
            overriding: None,
        };
        let mut proposed = registry.clone();
        proposed.put_attachment(attachment.clone());
        self.validate(owner.id, None, &registry, &proposed, &Reach::of(&sandbox))
            .await?;
        self.repo.attach(&attachment).await?;
        debug!(sandbox = %sandbox.id.0, entry = target, "attached a setup entry");
        self.changed(&[sandbox.id]).await
    }

    /// Refused while a template attached to the sandbox would refer to it.
    pub async fn detach(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let entry = registry.attachable(id)?;
        let (keeper, target) = (entry.owner.secrets_owner(), entry.target.to_string());
        let attachment = attached(&registry, &sandbox, id)?.clone();
        let mut proposed = registry.clone();
        proposed.remove_attachment(&sandbox.id, id);
        self.validate(owner.id, None, &registry, &proposed, &Reach::of(&sandbox))
            .await?;
        self.repo.detach(&sandbox.id, id).await?;
        debug!(sandbox = %sandbox.id.0, entry = target, "detached a setup entry");
        self.forget(keeper, overriding_secret(&attachment)).await;
        self.changed(&[sandbox.id]).await
    }

    /// Gives the attached entry `value` in this sandbox alone, kept as secret as the entry, or
    /// with `None` its own value again.
    pub async fn set_override(
        &self,
        owner: &User,
        name: &SandboxName,
        id: EntryId,
        value: Option<SecretSlice<u8>>,
    ) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let entry = registry.attachable(id)?;
        let keeper = entry.owner.secrets_owner();
        let before = attached(&registry, &sandbox, id)?.clone();
        if value.is_none() && before.overriding.is_none() {
            return Ok(());
        }
        let overriding = match &value {
            Some(value) => {
                check_content(&entry.target, entry.template, value.expose_secret())?;
                Some(
                    self.keep(keeper, &entry.target, entry.is_secret(), value)
                        .await?,
                )
            }
            None => None,
        };
        let after = Attachment {
            overriding,
            ..before.clone()
        };
        let mut proposed = registry.clone();
        proposed.put_attachment(after.clone());
        self.settle(keeper, after.overriding.iter(), async {
            self.validate(owner.id, None, &registry, &proposed, &Reach::of(&sandbox))
                .await?;
            self.repo.attach(&after).await
        })
        .await?;
        self.forget(keeper, overriding_secret(&before)).await;
        self.changed(&[sandbox.id]).await
    }

    /// The attached template as the sandbox would get it at a start, each secret variable it
    /// takes masked. A secret template is never shown.
    pub async fn preview(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<Preview> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let entry = registry.entry(id)?;
        let effective = registry.effective(Scope::Sandbox(&sandbox.id));
        let Some((_, value)) = effective.iter().find(|(held, _)| held.id == id) else {
            return Err(Error::NotFound(format!(
                "{} is not attached to sandbox {}",
                entry.target, sandbox.name
            )));
        };
        let text = match (&entry.target, &value.kept) {
            (Target::File(_), Kept::Plain(text)) if entry.template => {
                String::from_utf8_lossy(text).into_owned()
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "only a template that is not secret has a preview, and {} is not one",
                    entry.target
                )));
            }
        };
        let variables: BTreeMap<&VariableName, &StoredValue> = effective
            .iter()
            .filter_map(|&(entry, value)| match &entry.target {
                Target::Variable(name) => Some((name, value)),
                Target::File(_) => None,
            })
            .collect();
        let mut unresolved = BTreeSet::new();
        let text = Template::parse(&text).render_with(|name| match variables.get(name) {
            Some(StoredValue {
                kept: Kept::Plain(value),
                ..
            }) => String::from_utf8_lossy(value).into_owned(),
            Some(_) => MASK.to_owned(),
            None => {
                unresolved.insert(name.clone());
                format!("{{{{ {name} }}}}")
            }
        });
        Ok(Preview { text, unresolved })
    }

    /// Every shared entry, variables before files, each kind by name.
    pub async fn list_shared(&self, _: &Admin) -> Result<Vec<SharedEntryView>> {
        let mut shared = self.repo.shared_entries().await?;
        by_target(&mut shared);
        self.shared_views(shared).await
    }

    pub async fn get_shared(&self, _: &Admin, id: EntryId) -> Result<SharedEntryView> {
        let entry = self.shared_entry(id).await?;
        self.shared_view(entry).await
    }

    /// A required entry reaches every sandbox at its next start; an offered one, new sandboxes
    /// when it applies to them.
    pub async fn create_shared(
        &self,
        admin: &Admin,
        new: NewSharedEntry,
    ) -> Result<SharedEntryView> {
        let NewSharedEntry { shared, entry: new } = new;
        let mut proposed = self.repo.shared_entries().await?;
        check_shared_target(&proposed, &new.target, None)?;
        check_shape(&new.target, new.template, new.executable)?;
        check_content(&new.target, new.template, new.value.expose_secret())?;
        if shared == Shared::Required {
            self.check_members_lack(&new.target).await?;
        }
        let owner = Owner::Everyone(shared);
        let entry = Entry {
            id: EntryId::generate(),
            owner,
            template: new.template,
            executable: new.executable,
            apply_to_new: new.apply_to_new,
            value: self
                .keep(owner.secrets_owner(), &new.target, new.secret, &new.value)
                .await?,
            target: new.target,
            created_at: Utc::now(),
        };
        proposed.push(entry.clone());
        self.settle(UserId::SHARED, [&entry.value], async {
            self.check_required(&proposed).await?;
            self.validate_everyone(None, Some(&entry), &[]).await?;
            self.repo.insert(&entry).await
        })
        .await?;
        if shared == Shared::Required {
            self.changed_everywhere().await?;
        }
        changed_shared(admin, &entry);
        self.shared_view(entry).await
    }

    /// Changes the shared entry in place, as [`Self::change`] does a member's. An offered entry
    /// made required reaches every sandbox, so it is detached from those attached to it and
    /// their overrides are forgotten.
    pub async fn change_shared(
        &self,
        admin: &Admin,
        id: EntryId,
        change: SharedEntryChange,
    ) -> Result<SharedEntryView> {
        let SharedEntryChange { shared, change } = change;
        let mut proposed = self.repo.shared_entries().await?;
        let before = proposed
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| not_found(id))?;
        check_shared_target(&proposed, &change.target, Some(id))?;
        check_shape(&change.target, change.template, change.executable)?;
        let required = shared == Shared::Required;
        if required {
            self.check_members_lack(&change.target).await?;
        }
        let attachments = self.repo.attachments_of(id).await?;
        let refitting = if required { &[][..] } else { &attachments };
        let labels = self.every_sandbox_label().await?;
        let (after, overrides, rekept) =
            self.refitted(&before, &change, refitting, &labels).await?;
        let after = Entry {
            owner: Owner::Everyone(shared),
            ..after
        };
        proposed.retain(|entry| entry.id != id);
        proposed.push(after.clone());
        self.settle(UserId::SHARED, &rekept.kept, async {
            self.check_required(&proposed).await?;
            self.validate_everyone(Some(&before), Some(&after), &overrides)
                .await?;
            self.update(&after, &overrides).await
        })
        .await?;
        self.forget_all(UserId::SHARED, rekept.gone).await;
        if required {
            for attachment in &attachments {
                self.repo.detach(&attachment.sandbox, id).await?;
                self.forget(UserId::SHARED, overriding_secret(attachment))
                    .await;
            }
        }
        if required || before.owner.shared() == Some(Shared::Required) {
            self.changed_everywhere().await?;
        } else {
            let valued = change.value.is_some();
            self.changed(&affected(&before, &after, valued, &attachments))
                .await?;
        }
        changed_shared(admin, &after);
        self.shared_view(after).await
    }

    /// Detaches the shared entry from every sandbox, which loses it at its next start, and
    /// forgets their overrides.
    pub async fn delete_shared(&self, admin: &Admin, id: EntryId) -> Result<()> {
        let mut proposed = self.repo.shared_entries().await?;
        let entry = proposed
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| not_found(id))?;
        proposed.retain(|entry| entry.id != id);
        self.check_required(&proposed).await?;
        self.validate_everyone(Some(&entry), None, &[]).await?;
        let attachments = self.repo.attachments_of(id).await?;
        if !self.repo.delete(&entry.owner, id).await? {
            return Err(not_found(id));
        }
        self.forget(UserId::SHARED, entry.value.secret_id()).await;
        self.forget_all(UserId::SHARED, attachments.iter().map(overriding_secret))
            .await;
        if entry.owner.shared() == Some(Shared::Required) {
            self.changed_everywhere().await?;
        } else {
            self.changed(attachments.iter().map(|attachment| &attachment.sandbox))
                .await?;
        }
        changed_shared(admin, &entry);
        Ok(())
    }

    /// What the sandbox gets at a start that issues it `token`. Refuses when a template file
    /// attached to it names a variable that is not.
    pub(super) async fn boot(&self, sandbox: &Sandbox, token: &SandboxToken) -> Result<Boot> {
        let registry = self.registry(sandbox.owner).await?;
        let composed = self
            .compose(&registry, Scope::Sandbox(&sandbox.id), sandbox, token)
            .await?;
        if let Some((path, names)) = composed.unknown.first_key_value() {
            return Err(Error::Invalid(format!(
                "sandbox {} cannot start: template ~/{path} refers to {}, not attached to it",
                sandbox.name,
                listed(names)
            )));
        }
        Ok(composed.boot)
    }

    /// Attaches a new sandbox to the entries `chosen` for it, or without a choice to each entry
    /// that applies to new sandboxes, refusing what would keep it from starting.
    pub(super) async fn attach_new(
        &self,
        sandbox: &Sandbox,
        chosen: Option<&[EntryId]>,
    ) -> Result<()> {
        let registry = self.registry(sandbox.owner).await?;
        for &id in chosen.unwrap_or_default() {
            registry
                .entry(id)
                .map_err(|_| Error::Invalid(format!("there is no setup entry {}", id.0)))?;
        }
        let scope = chosen.map_or(Scope::New, Scope::Chosen);
        let composed = self
            .compose(&registry, scope, sandbox, &SandboxToken::generate()?)
            .await?;
        if let Some((path, names)) = composed.unknown.first_key_value() {
            let one = names.len() == 1;
            let names = listed(names);
            return Err(Error::Invalid(match scope {
                Scope::Chosen(_) => format!(
                    "template ~/{path} refers to {names}: choose {} too, or leave the template out",
                    if one { "it" } else { "them" }
                ),
                Scope::New | Scope::Sandbox(_) => format!(
                    "template ~/{path} applies to new sandboxes but refers to {names}, which {} not",
                    if one { "does" } else { "do" }
                ),
            }));
        }
        for (entry, _) in registry.effective(scope) {
            if entry.owner.shared() == Some(Shared::Required) {
                continue;
            }
            self.repo
                .attach(&Attachment {
                    sandbox: sandbox.id.clone(),
                    entry: entry.id,
                    overriding: None,
                })
                .await?;
            let target = entry.target.to_string();
            debug!(sandbox = %sandbox.id.0, entry = target, "attached a setup entry");
        }
        Ok(())
    }

    /// Detaches every entry from a sandbox that is going away.
    pub(super) async fn forget_sandbox(&self, sandbox: &Sandbox) -> Result<()> {
        let registry = self.registry(sandbox.owner).await?;
        for (entry, attachment) in registry.attached(&sandbox.id) {
            self.repo.detach(&sandbox.id, entry.id).await?;
            self.forget(entry.owner.secrets_owner(), overriding_secret(attachment))
                .await;
        }
        Ok(())
    }

    pub(super) async fn remove_all(&self, owner: UserId) -> Result<()> {
        for entry in self.repo.entries(owner).await? {
            self.repo.delete(&entry.owner, entry.id).await?;
        }
        self.secrets.remove_all(owner).await
    }

    async fn registry(&self, owner: UserId) -> Result<Registry> {
        let shared = self.repo.shared_entries().await?;
        let mut attachments = self.repo.attachments(owner).await?;
        let offered: Vec<EntryId> = shared
            .iter()
            .filter(|entry| entry.owner.shared() == Some(Shared::Offered))
            .map(|entry| entry.id)
            .collect();
        if !offered.is_empty() {
            let theirs = self.sandbox_names(owner).await?;
            for id in offered {
                let attached = self.repo.attachments_of(id).await?;
                attachments.extend(
                    attached
                        .into_iter()
                        .filter(|attachment| theirs.contains_key(&attachment.sandbox)),
                );
            }
        }
        Ok(Registry::new(
            self.repo.entries(owner).await?,
            shared,
            attachments,
        ))
    }

    async fn sandbox(&self, owner: &User, name: &SandboxName) -> Result<Sandbox> {
        self.sandboxes
            .by_name(owner.id, name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("sandbox {name}")))
    }

    async fn sandbox_names(&self, owner: UserId) -> Result<BTreeMap<SandboxId, SandboxName>> {
        Ok(self
            .sandboxes
            .list(owner)
            .await?
            .into_iter()
            .map(|sandbox| (sandbox.id, sandbox.name))
            .collect())
    }

    async fn shared_entry(&self, id: EntryId) -> Result<Entry> {
        self.repo
            .shared_entries()
            .await?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| not_found(id))
    }

    async fn shared_view(&self, entry: Entry) -> Result<SharedEntryView> {
        let id = entry.id;
        self.shared_views(vec![entry])
            .await?
            .pop()
            .ok_or_else(|| not_found(id))
    }

    /// The views of the shared entries among `entries`.
    async fn shared_views(&self, entries: Vec<Entry>) -> Result<Vec<SharedEntryView>> {
        let members = self.repo.every_member_entry().await?;
        let handles = self.handles().await?;
        let wanted = secret_ids(
            entries
                .iter()
                .filter(|entry| entry.template)
                .map(|entry| (entry, &entry.value)),
        );
        let secrets = self.secrets_of(wanted).await?;
        let mut views = Vec::new();
        for entry in entries {
            let Owner::Everyone(shared) = entry.owner else {
                continue;
            };
            let shadowing: BTreeSet<UserId> = members
                .iter()
                .filter(|member| member.target.collides(&entry.target))
                .filter_map(|member| member.owner.member())
                .collect();
            views.push(SharedEntryView {
                shared,
                references: references(&entry, &secrets),
                attached: self.repo.attachments_of(entry.id).await?.len(),
                shadowed_by: handles_of(&handles, &shadowing),
                entry,
            });
        }
        Ok(views)
    }

    async fn handles(&self) -> Result<BTreeMap<UserId, Handle>> {
        Ok(self
            .users
            .list()
            .await?
            .into_iter()
            .map(|user| (user.id, user.handle))
            .collect())
    }

    /// Each sandbox by its name and its member's handle.
    async fn every_sandbox_label(&self) -> Result<BTreeMap<SandboxId, String>> {
        let handles = self.handles().await?;
        Ok(self
            .sandboxes
            .list_all()
            .await?
            .into_iter()
            .map(|sandbox| {
                let label = match handles.get(&sandbox.owner) {
                    Some(handle) => format!("{} of {handle}", sandbox.name),
                    None => sandbox.name.to_string(),
                };
                (sandbox.id, label)
            })
            .collect())
    }

    /// Refuses a target for a required entry that an entry of a member collides with, naming
    /// the members.
    async fn check_members_lack(&self, target: &Target) -> Result<()> {
        let members: BTreeSet<UserId> = self
            .repo
            .every_member_entry()
            .await?
            .into_iter()
            .filter(|entry| entry.target.collides(target))
            .filter_map(|entry| entry.owner.member())
            .collect();
        if members.is_empty() {
            return Ok(());
        }
        let handles = handles_of(&self.handles().await?, &members);
        Err(Error::Conflict(format!(
            "{target} is also set by {}; ask them to rename theirs first",
            and_listed(&handles)
        )))
    }

    /// Refuses shared entries among which a required template refers to a variable that is not
    /// required.
    async fn check_required(&self, shared: &[Entry]) -> Result<()> {
        let required: Vec<(&Entry, &StoredValue)> = shared
            .iter()
            .filter(|entry| entry.owner.shared() == Some(Shared::Required))
            .map(|entry| (entry, &entry.value))
            .collect();
        let wanted = secret_ids(required.iter().copied().filter(|(entry, _)| entry.template));
        let secrets = self.secrets_of(wanted).await?;
        if let Some((path, names)) = unresolved(&required, &secrets).first_key_value() {
            return Err(Error::Invalid(format!(
                "template ~/{path} applies to every sandbox and refers to {}, which {} not \
                 required",
                listed(names),
                if names.len() == 1 { "is" } else { "are" }
            )));
        }
        Ok(())
    }

    /// Refuses a change of a shared entry from `before` to `after`, with `overrides` kept anew,
    /// that would keep a member's sandbox it reaches, or their new sandboxes, from starting.
    async fn validate_everyone(
        &self,
        before: Option<&Entry>,
        after: Option<&Entry>,
        overrides: &[Attachment],
    ) -> Result<()> {
        let Some(id) = after.or(before).map(|entry| entry.id) else {
            return Ok(());
        };
        let either = || before.into_iter().chain(after);
        let required = either().any(|entry| entry.owner.shared() == Some(Shared::Required));
        let new = required || either().any(|entry| entry.apply_to_new);
        if !new && self.repo.attachments_of(id).await?.is_empty() {
            return Ok(());
        }
        for user in self.users.list().await? {
            let current = self.registry(user.id).await?;
            let mut proposed = current.clone();
            match after {
                Some(after) => proposed.put_entry(after.clone()),
                None => proposed.remove_entry(id),
            }
            for attachment in overrides {
                if current
                    .attachment(&attachment.sandbox, attachment.entry)
                    .is_some()
                {
                    proposed.put_attachment(attachment.clone());
                }
            }
            let sandboxes = if required {
                self.sandbox_names(user.id).await?.into_keys().collect()
            } else {
                current
                    .attachments_of(id)
                    .map(|attachment| attachment.sandbox.clone())
                    .collect()
            };
            let reach = Reach { sandboxes, new };
            self.validate(user.id, Some(&user.handle), &current, &proposed, &reach)
                .await?;
        }
        Ok(())
    }

    async fn view(&self, owner: UserId, registry: &Registry, id: EntryId) -> Result<EntryView> {
        self.views(owner, registry, [id])
            .await?
            .pop()
            .ok_or_else(|| not_found(id))
    }

    /// The views of `ids`, in their order. Templates read their secret values to tell what they
    /// refer to.
    async fn views(
        &self,
        owner: UserId,
        registry: &Registry,
        ids: impl IntoIterator<Item = EntryId>,
    ) -> Result<Vec<EntryView>> {
        let ids: Vec<EntryId> = ids.into_iter().collect();
        let names = self.sandbox_names(owner).await?;
        let templates = registry
            .shared
            .iter()
            .chain(&registry.entries)
            .filter(|entry| entry.template);
        let wanted = secret_ids(templates.flat_map(|entry| {
            std::iter::once((entry, &entry.value)).chain(
                registry
                    .attachments_of(entry.id)
                    .filter_map(move |a| Some((entry, a.overriding.as_ref()?))),
            )
        }));
        let secrets = self.secrets_of(wanted).await?;
        let mut unresolved_in = BTreeMap::new();
        ids.into_iter()
            .map(|id| {
                let entry = registry.entry(id)?;
                let path = match &entry.target {
                    Target::File(path) if entry.template => Some(path),
                    _ => None,
                };
                let reached: Vec<(&SandboxId, bool)> =
                    if entry.owner.shared() == Some(Shared::Required) {
                        names.keys().map(|sandbox| (sandbox, false)).collect()
                    } else {
                        registry
                            .attachments_of(id)
                            .map(|a| (&a.sandbox, a.overriding.is_some()))
                            .collect()
                    };
                let mut sandboxes: Vec<Use> = reached
                    .into_iter()
                    .filter_map(|(id, overridden)| {
                        let sandbox = names.get(id)?;
                        let missing = match path {
                            Some(path) => unresolved_in
                                .entry(id.clone())
                                .or_insert_with(|| {
                                    unresolved(&registry.effective(Scope::Sandbox(id)), &secrets)
                                })
                                .get(path)
                                .cloned()
                                .unwrap_or_default(),
                            None => BTreeSet::new(),
                        };
                        Some(Use {
                            sandbox: sandbox.clone(),
                            overridden,
                            unresolved: missing,
                        })
                    })
                    .collect();
                sandboxes.sort_by(|a, b| a.sandbox.cmp(&b.sandbox));
                Ok(EntryView {
                    references: references(entry, &secrets),
                    duplicate: registry.is_duplicate(entry),
                    shadows: registry.shadows(entry),
                    shadowed: registry.shadowed(entry),
                    entry: entry.clone(),
                    sandboxes,
                })
            })
            .collect()
    }

    async fn compose(
        &self,
        registry: &Registry,
        scope: Scope<'_>,
        sandbox: &Sandbox,
        token: &SandboxToken,
    ) -> Result<Composed> {
        let effective = registry.effective(scope);
        let secrets = self
            .secrets_of(secret_ids(effective.iter().copied()))
            .await?;
        compose(
            &whose(scope, &sandbox.name, None),
            &effective,
            &secrets,
            self.links
                .sandbox_file(&sandbox.name, sandbox.repository.as_ref(), token),
        )
    }

    /// Refuses a change that would leave a sandbox it reaches, or new sandboxes, with colliding
    /// entries, beyond a limit, or with a template referring to a variable they do not get,
    /// which the current state did not already do. Refusals name the member `of` when given.
    async fn validate(
        &self,
        owner: UserId,
        of: Option<&Handle>,
        current: &Registry,
        proposed: &Registry,
        reach: &Reach,
    ) -> Result<()> {
        if reach.sandboxes.is_empty() && !reach.new {
            return Ok(());
        }
        // The token a start would issue is as long as any other, so the files it sizes are too.
        let token = SandboxToken::generate()?;
        let theirs: BTreeMap<SandboxId, Sandbox> = self
            .sandboxes
            .list(owner)
            .await?
            .into_iter()
            .map(|sandbox| (sandbox.id.clone(), sandbox))
            .collect();
        let longest = SandboxName::parse(&"x".repeat(32)).expect("a valid sandbox name");
        let mut scopes: Vec<(Scope<'_>, &SandboxName, Option<&Repository>)> = reach
            .sandboxes
            .iter()
            .filter_map(|id| {
                let sandbox = theirs.get(id)?;
                Some((
                    Scope::Sandbox(id),
                    &sandbox.name,
                    sandbox.repository.as_ref(),
                ))
            })
            .collect();
        if reach.new {
            scopes.push((Scope::New, &longest, None));
        }
        let wanted = secret_ids(scopes.iter().flat_map(|(scope, _, _)| {
            current
                .effective(*scope)
                .into_iter()
                .chain(proposed.effective(*scope))
        }));
        let secrets = self.secrets_of(wanted).await?;
        for (scope, name, repository) in scopes {
            let whose = whose(scope, name, of);
            let composed = compose(
                &whose,
                &proposed.effective(scope),
                &secrets,
                self.links.sandbox_file(name, repository, &token),
            )?;
            let before = unresolved(&current.effective(scope), &secrets);
            for (path, names) in composed.unknown {
                let new: BTreeSet<VariableName> = names
                    .into_iter()
                    .filter(|name| !before.get(&path).is_some_and(|old| old.contains(name)))
                    .collect();
                if new.is_empty() {
                    continue;
                }
                let names = listed(&new);
                return Err(Error::Invalid(match scope {
                    Scope::Sandbox(_) | Scope::Chosen(_) => format!(
                        "{whose} could not start: template ~/{path} would refer to {names}, not \
                         attached to it"
                    ),
                    Scope::New => format!(
                        "{whose} could not start: template ~/{path} applies to them and would \
                         refer to {names}, which would not"
                    ),
                }));
            }
        }
        Ok(())
    }

    /// The value as the changed entry keeps it, checked against what the entry becomes: kept
    /// anew when it changes kind or secrecy, or `None` when it stays as it is.
    async fn refit(
        &self,
        owner: UserId,
        change: &EntryChange,
        shift: &Shift,
        value: &StoredValue,
        rekept: &mut Rekept,
    ) -> Result<Option<StoredValue>> {
        if !(shift.kind || shift.secrecy || shift.template) {
            check_size(&change.target, value.size())?;
            return Ok(None);
        }
        let content = self.content(owner, value).await?;
        check_content(&change.target, change.template, &content)?;
        if !(shift.kind || shift.secrecy) {
            return Ok(None);
        }
        let kept = self
            .keep(
                owner,
                &change.target,
                change.secret,
                &SecretSlice::from(content),
            )
            .await?;
        rekept.record(&kept, value);
        Ok(Some(kept))
    }

    /// The entry as `change` makes it, each override among `attachments` it keeps anew, and
    /// what was kept anew; leaves nothing kept anew behind when it fails. `labels` name the
    /// sandboxes to refuse an override in.
    async fn refitted(
        &self,
        before: &Entry,
        change: &EntryChange,
        attachments: &[Attachment],
        labels: &BTreeMap<SandboxId, String>,
    ) -> Result<(Entry, Vec<Attachment>, Rekept)> {
        let keeper = before.owner.secrets_owner();
        let shift = Shift {
            kind: change.target.is_file() != before.target.is_file(),
            secrecy: change.secret != before.is_secret(),
            template: change.template && !before.template,
        };
        let mut rekept = Rekept::default();
        let refitted = async {
            let own = match &change.value {
                Some(value) => {
                    check_content(&change.target, change.template, value.expose_secret())?;
                    let kept = self
                        .keep(keeper, &change.target, change.secret, value)
                        .await?;
                    rekept.record(&kept, &before.value);
                    kept
                }
                None => self
                    .refit(keeper, change, &shift, &before.value, &mut rekept)
                    .await?
                    .unwrap_or_else(|| before.value.clone()),
            };
            let overrides = self
                .refit_overrides(keeper, attachments, labels, change, &shift, &mut rekept)
                .await?;
            Ok((own, overrides))
        }
        .await;
        match refitted {
            Ok((value, overrides)) => {
                let after = Entry {
                    target: change.target.clone(),
                    template: change.template,
                    executable: change.executable,
                    apply_to_new: change.apply_to_new,
                    value,
                    ..before.clone()
                };
                Ok((after, overrides, rekept))
            }
            Err(error) => {
                self.forget_all(keeper, rekept.kept.iter().map(StoredValue::secret_id))
                    .await;
                Err(error)
            }
        }
    }

    /// The overrides among `attachments` as [`Self::refit`] keeps them, each that changes.
    async fn refit_overrides(
        &self,
        owner: UserId,
        attachments: &[Attachment],
        labels: &BTreeMap<SandboxId, String>,
        change: &EntryChange,
        shift: &Shift,
        rekept: &mut Rekept,
    ) -> Result<Vec<Attachment>> {
        let mut overrides = Vec::new();
        for attachment in attachments {
            let Some(overriding) = &attachment.overriding else {
                continue;
            };
            let sandbox = labels.get(&attachment.sandbox).map_or("", String::as_str);
            let refitted = self
                .refit(owner, change, shift, overriding, rekept)
                .await
                .map_err(|error| {
                    reworded(error, |problem| {
                        format!("the override in sandbox {sandbox} does not suit it: {problem}")
                    })
                })?;
            if let Some(value) = refitted {
                overrides.push(Attachment {
                    overriding: Some(value),
                    ..attachment.clone()
                });
            }
        }
        Ok(overrides)
    }

    /// The values the store holds for `ids`.
    async fn secrets_of(
        &self,
        ids: BTreeMap<UserId, BTreeSet<SecretId>>,
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        let mut secrets = BTreeMap::new();
        for (owner, ids) in ids {
            let ids: Vec<SecretId> = ids.into_iter().collect();
            secrets.extend(self.secrets.get(owner, &ids).await?);
        }
        Ok(secrets)
    }

    async fn keep(
        &self,
        owner: UserId,
        target: &Target,
        secret: bool,
        value: &SecretSlice<u8>,
    ) -> Result<StoredValue> {
        let bytes = value.expose_secret();
        let kept = if secret {
            let id = SecretId::generate();
            self.secrets.put(owner, id, value).await?;
            Kept::Secret(SecretRef {
                id,
                size: bytes.len(),
                glimpse: match target {
                    Target::Variable(_) => Some(Glimpse::of(&String::from_utf8_lossy(bytes))),
                    Target::File(_) => None,
                },
            })
        } else {
            Kept::Plain(bytes.to_vec())
        };
        Ok(StoredValue {
            kept,
            set_at: Utc::now(),
        })
    }

    /// Runs `work`, removing the secrets `kept` holds when it fails.
    async fn settle<'a, T>(
        &self,
        owner: UserId,
        kept: impl IntoIterator<Item = &'a StoredValue>,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let done = work.await;
        if done.is_err() {
            self.forget_all(owner, kept.into_iter().map(StoredValue::secret_id))
                .await;
        }
        done
    }

    // A value left behind only takes room in the store, and the owner's deletion removes it.
    async fn forget(&self, owner: UserId, id: Option<SecretId>) {
        if let Some(id) = id {
            let _ = self.secrets.remove(owner, id).await;
        }
    }

    async fn forget_all(&self, owner: UserId, ids: impl IntoIterator<Item = Option<SecretId>>) {
        for id in ids {
            self.forget(owner, id).await;
        }
    }

    async fn content(&self, owner: UserId, value: &StoredValue) -> Result<Vec<u8>> {
        match &value.kept {
            Kept::Plain(content) => Ok(content.clone()),
            Kept::Secret(secret) => self
                .secrets
                .get(owner, &[secret.id])
                .await?
                .remove(&secret.id)
                .map(|value| value.expose_secret().to_vec())
                .ok_or_else(|| Error::internal("setup", "a secret value is missing")),
        }
    }

    async fn update(&self, entry: &Entry, overrides: &[Attachment]) -> Result<()> {
        if self.repo.update(entry, overrides).await? {
            Ok(())
        } else {
            Err(not_found(entry.id))
        }
    }

    // Recorded after the write, so a start that loaded the earlier values began before it.
    async fn changed<'a>(&self, sandboxes: impl IntoIterator<Item = &'a SandboxId>) -> Result<()> {
        let sandboxes: Vec<SandboxId> = sandboxes.into_iter().cloned().collect();
        if sandboxes.is_empty() {
            return Ok(());
        }
        self.sandboxes
            .record_setup_change(Changed::Sandboxes(&sandboxes), Utc::now())
            .await
    }

    async fn changed_everywhere(&self) -> Result<()> {
        self.sandboxes
            .record_setup_change(Changed::Every, Utc::now())
            .await
    }
}

/// What a change does to an entry beyond its target and value.
struct Shift {
    /// From a variable to a file, or back.
    kind: bool,
    /// To or from the secret store.
    secrecy: bool,
    /// Into a template.
    template: bool,
}

/// The values a change keeps anew, and the secrets it then leaves behind.
#[derive(Default)]
struct Rekept {
    kept: Vec<StoredValue>,
    gone: Vec<Option<SecretId>>,
}

impl Rekept {
    fn record(&mut self, kept: &StoredValue, replaced: &StoredValue) {
        self.kept.push(kept.clone());
        self.gone.push(replaced.secret_id());
    }
}

impl Reach {
    fn of(sandbox: &Sandbox) -> Self {
        Self {
            sandboxes: BTreeSet::from([sandbox.id.clone()]),
            new: false,
        }
    }
}

/// The ids of the secret values among `values`, by whom the store keeps each under: its entry's
/// owner.
fn secret_ids<'a>(
    values: impl IntoIterator<Item = (&'a Entry, &'a StoredValue)>,
) -> BTreeMap<UserId, BTreeSet<SecretId>> {
    let mut ids: BTreeMap<UserId, BTreeSet<SecretId>> = BTreeMap::new();
    for (entry, value) in values {
        if let Some(id) = value.secret_id() {
            ids.entry(entry.owner.secrets_owner())
                .or_default()
                .insert(id);
        }
    }
    ids
}

fn whose(scope: Scope<'_>, name: &SandboxName, of: Option<&Handle>) -> String {
    let whose = match scope {
        Scope::Sandbox(_) | Scope::Chosen(_) => format!("sandbox {name}"),
        Scope::New => "new sandboxes".to_owned(),
    };
    match of {
        Some(handle) => format!("{whose} of {handle}"),
        None => whose,
    }
}

fn not_found(id: EntryId) -> Error {
    Error::NotFound(format!("setup entry {}", id.0))
}

/// For a template: the variables its own value refers to.
fn references(
    entry: &Entry,
    secrets: &BTreeMap<SecretId, SecretSlice<u8>>,
) -> BTreeSet<VariableName> {
    match &entry.target {
        Target::File(_) if entry.template => template_of(&entry.value, secrets)
            .names()
            .into_iter()
            .cloned()
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// The sandboxes among `attachments` whose next start the change from `before` to `after`
/// alters, `valued` when it gives the entry another value.
fn affected(
    before: &Entry,
    after: &Entry,
    valued: bool,
    attachments: &[Attachment],
) -> BTreeSet<SandboxId> {
    let lands = after.target != before.target
        || after.template != before.template
        || after.executable != before.executable
        || after.is_secret() != before.is_secret();
    attachments
        .iter()
        .filter(|attachment| lands || (valued && attachment.overriding.is_none()))
        .map(|attachment| attachment.sandbox.clone())
        .collect()
}

fn attached<'a>(registry: &'a Registry, sandbox: &Sandbox, id: EntryId) -> Result<&'a Attachment> {
    let entry = registry.entry(id)?;
    registry.attachment(&sandbox.id, id).ok_or_else(|| {
        Error::NotFound(format!(
            "{} is not attached to sandbox {}",
            entry.target, sandbox.name
        ))
    })
}

fn overriding_secret(attachment: &Attachment) -> Option<SecretId> {
    attachment
        .overriding
        .as_ref()
        .and_then(StoredValue::secret_id)
}

/// Refuses a target Corral keeps for itself.
fn check_own_dir(target: &Target) -> Result<()> {
    if let Target::File(path) = target
        && path.overlaps(&HomePath::parse(OWN_DIR).expect("a valid home path"))
    {
        return Err(Error::Invalid(format!(
            "~/{path} is where Corral keeps the sandbox's own files, in ~/{OWN_DIR}"
        )));
    }
    Ok(())
}

/// Refuses a target Corral keeps for itself, one a required entry has, and one another entry of
/// the member already has.
fn check_target(registry: &Registry, target: &Target, except: Option<EntryId>) -> Result<()> {
    check_own_dir(target)?;
    if let Some(required) = registry
        .required()
        .find(|required| required.target.collides(target))
    {
        return Err(Error::Conflict(format!(
            "{} is set for every sandbox by the admin",
            required.target
        )));
    }
    if registry
        .entries
        .iter()
        .any(|entry| Some(entry.id) != except && entry.target == *target)
    {
        return Err(Error::Conflict(format!(
            "you already have {target}; to give one sandbox another value, override it there"
        )));
    }
    Ok(())
}

/// Refuses flags only a file takes on a variable.
fn check_shape(target: &Target, template: bool, executable: bool) -> Result<()> {
    if let Target::Variable(name) = target {
        if template {
            return Err(Error::Invalid(format!(
                "variable {name} cannot be a template; only a file can"
            )));
        }
        if executable {
            return Err(Error::Invalid(format!(
                "variable {name} cannot be executable; only a file can"
            )));
        }
    }
    Ok(())
}

/// Refuses a value larger than its target takes: a variable [`VARIABLE_LIMIT`], a file
/// [`FILE_LIMIT`].
fn check_size(target: &Target, size: usize) -> Result<()> {
    match target {
        Target::Variable(name) => {
            if name.as_str().len() + size + 2 > VARIABLE_LIMIT {
                return Err(Error::LimitReached(format!(
                    "variable {name} takes {size} bytes, more than the {} one variable may take \
                     with its name; make it a file",
                    amount(VARIABLE_LIMIT)
                )));
            }
        }
        Target::File(path) => {
            if size > FILE_LIMIT {
                return Err(Error::LimitReached(format!(
                    "file ~/{path} is {size} bytes, more than the {} a file may hold",
                    amount(FILE_LIMIT)
                )));
            }
        }
    }
    Ok(())
}

/// Refuses a value its target cannot take: a variable takes one line of text, and a template
/// text.
fn check_content(target: &Target, template: bool, bytes: &[u8]) -> Result<()> {
    check_size(target, bytes.len())?;
    match target {
        Target::Variable(name) => {
            let one_line =
                std::str::from_utf8(bytes).is_ok_and(|text| !text.contains(['\n', '\r', '\0']));
            if !one_line {
                return Err(Error::Invalid(format!(
                    "variable {name} must be one line of text without NUL bytes; \
                     base64-encode other values, or make them a file"
                )));
            }
        }
        Target::File(path) => {
            if template && std::str::from_utf8(bytes).is_err() {
                return Err(Error::Invalid(format!(
                    "template ~/{path} must be text, in UTF-8"
                )));
            }
        }
    }
    Ok(())
}

/// The error with its message rewritten by `with`.
fn reworded(error: Error, with: impl FnOnce(String) -> String) -> Error {
    match error {
        Error::Invalid(problem) => Error::Invalid(with(problem)),
        Error::LimitReached(problem) => Error::LimitReached(with(problem)),
        other => other,
    }
}

/// Refuses a target Corral keeps for itself, and one colliding with another shared entry.
fn check_shared_target(shared: &[Entry], target: &Target, except: Option<EntryId>) -> Result<()> {
    check_own_dir(target)?;
    if let Some(other) = shared
        .iter()
        .find(|other| Some(other.id) != except && other.target.collides(target))
    {
        return Err(Error::Conflict(format!(
            "the shared setup already has {}",
            other.target
        )));
    }
    Ok(())
}

fn listed(names: &BTreeSet<VariableName>) -> String {
    names
        .iter()
        .map(VariableName::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The handles of `ids`, in order.
fn handles_of(handles: &BTreeMap<UserId, Handle>, ids: &BTreeSet<UserId>) -> Vec<Handle> {
    let mut handles: Vec<Handle> = ids
        .iter()
        .filter_map(|id| handles.get(id))
        .cloned()
        .collect();
    handles.sort();
    handles
}

/// The handles, joined with commas and the last with "and".
fn and_listed(handles: &[Handle]) -> String {
    match handles {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(ToString::to_string).collect();
            format!("{} and {last}", rest.join(", "))
        }
    }
}

fn changed_shared(admin: &Admin, entry: &Entry) {
    let (admin, entry) = (admin.handle(), entry.target.to_string());
    info!(%admin, kind = %"shared_setup", entry, "changed the settings");
}
