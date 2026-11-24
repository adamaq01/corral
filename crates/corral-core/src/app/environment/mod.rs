mod adoption;
mod compose;
mod registry;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use secrecy::{ExposeSecret, SecretSlice};

use compose::{Composed, amount, compose, template_of, unresolved};
pub use compose::{FILE_LIMIT, FILES_LIMIT, VARIABLE_LIMIT, VARIABLES_LIMIT};
use registry::{Registry, Scope};

use super::inside::SandboxLinks;
use crate::domain::{
    Attachment, Entry, EntryId, Glimpse, HomePath, Kept, Sandbox, SandboxId, SandboxName,
    SandboxToken, SecretId, SecretRef, StoredValue, Target, Template, User, UserId, VariableName,
};
use crate::ports::{Boot, EnvironmentRepo, SandboxRepo, SecretStore};
use crate::{Error, Result};

/// The directory in the home where Corral keeps the sandbox's own files.
const OWN_DIR: &str = ".config/corral";
/// What a preview shows in place of a secret variable's value.
const MASK: &str = "••••••••";

/// Each user's variables and files, and the sandboxes they are attached to.
#[derive(Clone)]
pub struct Environment {
    repo: Arc<dyn EnvironmentRepo>,
    secrets: Arc<dyn SecretStore>,
    sandboxes: Arc<dyn SandboxRepo>,
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

/// An entry with the sandboxes it is attached to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryView {
    pub entry: Entry,
    pub sandboxes: Vec<Use>,
    /// For a template: the variables its own value refers to.
    pub references: BTreeSet<VariableName>,
    /// Whether an entry made earlier has the same target, which only entries made before
    /// targets were unique can have.
    pub duplicate: bool,
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

/// Where a change lands: some of the owner's sandboxes, and new sandboxes.
#[derive(Default)]
struct Reach {
    sandboxes: BTreeSet<SandboxId>,
    new: bool,
}

impl Environment {
    #[must_use]
    pub fn new(
        repo: Arc<dyn EnvironmentRepo>,
        secrets: Arc<dyn SecretStore>,
        sandboxes: Arc<dyn SandboxRepo>,
        links: SandboxLinks,
    ) -> Self {
        Self {
            repo,
            secrets,
            sandboxes,
            links,
        }
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<EntryView>> {
        let registry = self.registry(owner.id).await?;
        self.views(owner.id, &registry, registry.entries.iter().map(|e| e.id))
            .await
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
            owner: owner.id,
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
            self.validate(owner.id, &registry, &proposed, &reach)
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
        let before = registry.entry(id)?.clone();
        check_target(&registry, &change.target, Some(id))?;
        check_shape(&change.target, change.template, change.executable)?;
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
                        .keep(owner.id, &change.target, change.secret, value)
                        .await?;
                    rekept.record(&kept, &before.value);
                    kept
                }
                None => self
                    .refit(owner.id, &change, &shift, &before.value, &mut rekept)
                    .await?
                    .unwrap_or_else(|| before.value.clone()),
            };
            let overrides = self
                .refit_overrides(owner.id, &registry, id, &change, &shift, &mut rekept)
                .await?;
            Ok((own, overrides))
        }
        .await;
        let (own, overrides) = match refitted {
            Ok(refitted) => refitted,
            Err(error) => {
                self.forget_all(owner.id, rekept.kept.iter().map(StoredValue::secret_id))
                    .await;
                return Err(error);
            }
        };

        let after = Entry {
            target: change.target,
            template: change.template,
            executable: change.executable,
            apply_to_new: change.apply_to_new,
            value: own,
            ..before.clone()
        };
        let mut proposed = registry.clone();
        proposed.put_entry(after.clone());
        for attachment in &overrides {
            proposed.put_attachment(attachment.clone());
        }
        let lands = after.target != before.target
            || after.template != before.template
            || after.executable != before.executable
            || shift.secrecy;
        let affected: BTreeSet<SandboxId> = registry
            .attachments_of(id)
            .filter(|attachment| {
                lands || (change.value.is_some() && attachment.overriding.is_none())
            })
            .map(|attachment| attachment.sandbox.clone())
            .collect();
        let reach = Reach {
            sandboxes: affected.clone(),
            new: before.apply_to_new || after.apply_to_new,
        };
        self.settle(owner.id, &rekept.kept, async {
            self.validate(owner.id, &registry, &proposed, &reach)
                .await?;
            if self.repo.update(&after, &overrides).await? {
                Ok(())
            } else {
                Err(Error::NotFound(format!("environment entry {}", id.0)))
            }
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
        let entry = registry.entry(id)?.clone();
        let attachments: Vec<Attachment> = registry.attachments_of(id).cloned().collect();
        let mut proposed = registry.clone();
        proposed.remove_entry(id);
        let reach = Reach {
            sandboxes: attachments.iter().map(|a| a.sandbox.clone()).collect(),
            new: entry.apply_to_new,
        };
        self.validate(owner.id, &registry, &proposed, &reach)
            .await?;
        if !self.repo.delete(owner.id, id).await? {
            return Err(Error::NotFound(format!("environment entry {}", id.0)));
        }
        self.forget(owner.id, entry.value.secret_id()).await;
        self.forget_all(owner.id, attachments.iter().map(overriding_secret))
            .await;
        self.changed(&reach.sandboxes).await
    }

    /// The entries attached to the owner's sandbox `name`.
    pub async fn of_sandbox(&self, owner: &User, name: &SandboxName) -> Result<Vec<Attached>> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let attached = registry.attached(&sandbox.id);
        let views = self
            .views(owner.id, &registry, attached.iter().map(|(e, _)| e.id))
            .await?;
        Ok(views
            .into_iter()
            .zip(attached)
            .map(|(view, (_, attachment))| Attached {
                view,
                overriding: attachment.overriding.clone(),
            })
            .collect())
    }

    /// Does nothing when the entry is attached already.
    pub async fn attach(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        registry.entry(id)?;
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
        self.validate(owner.id, &registry, &proposed, &Reach::of(&sandbox))
            .await?;
        self.repo.attach(&attachment).await?;
        self.changed(&[sandbox.id]).await
    }

    /// Refused while a template attached to the sandbox would refer to it.
    pub async fn detach(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let attachment = attached(&registry, &sandbox, id)?.clone();
        let mut proposed = registry.clone();
        proposed.remove_attachment(&sandbox.id, id);
        self.validate(owner.id, &registry, &proposed, &Reach::of(&sandbox))
            .await?;
        self.repo.detach(&sandbox.id, id).await?;
        self.forget(owner.id, overriding_secret(&attachment)).await;
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
        let entry = registry.entry(id)?;
        let before = attached(&registry, &sandbox, id)?.clone();
        if value.is_none() && before.overriding.is_none() {
            return Ok(());
        }
        let overriding = match &value {
            Some(value) => {
                check_content(&entry.target, entry.template, value.expose_secret())?;
                Some(
                    self.keep(owner.id, &entry.target, entry.is_secret(), value)
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
        self.settle(owner.id, after.overriding.iter(), async {
            self.validate(owner.id, &registry, &proposed, &Reach::of(&sandbox))
                .await?;
            self.repo.attach(&after).await
        })
        .await?;
        self.forget(owner.id, overriding_secret(&before)).await;
        self.changed(&[sandbox.id]).await
    }

    /// The attached template as the sandbox would get it at a start, each secret variable it
    /// takes masked. A secret template is never shown.
    pub async fn preview(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<Preview> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let entry = registry.entry(id)?;
        let attachment = attached(&registry, &sandbox, id)?;
        let value = attachment.overriding.as_ref().unwrap_or(&entry.value);
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
        let variables: BTreeMap<&VariableName, &StoredValue> = registry
            .effective(Scope::Sandbox(&sandbox.id))
            .into_iter()
            .filter_map(|(entry, value)| match &entry.target {
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

    /// What the sandbox gets at a start that issues it `token`. Refuses when a template file
    /// attached to it names a variable that is not.
    pub(super) async fn boot(&self, sandbox: &Sandbox, token: &SandboxToken) -> Result<Boot> {
        let registry = self.registry(sandbox.owner).await?;
        let composed = self
            .compose(
                sandbox.owner,
                &registry,
                Scope::Sandbox(&sandbox.id),
                &sandbox.name,
                token,
            )
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

    /// Attaches a new sandbox to each entry that applies to new sandboxes, refusing what would
    /// keep it from starting.
    pub(super) async fn attach_new(&self, sandbox: &Sandbox) -> Result<()> {
        let registry = self.registry(sandbox.owner).await?;
        let composed = self
            .compose(
                sandbox.owner,
                &registry,
                Scope::New,
                &sandbox.name,
                &SandboxToken::generate()?,
            )
            .await?;
        if let Some((path, names)) = composed.unknown.first_key_value() {
            return Err(Error::Invalid(format!(
                "template ~/{path} applies to new sandboxes but refers to {}, which {} not",
                listed(names),
                if names.len() == 1 { "does" } else { "do" }
            )));
        }
        for (entry, _) in registry.effective(Scope::New) {
            self.repo
                .attach(&Attachment {
                    sandbox: sandbox.id.clone(),
                    entry: entry.id,
                    overriding: None,
                })
                .await?;
        }
        Ok(())
    }

    /// Detaches every entry from a sandbox that is going away.
    pub(super) async fn forget_sandbox(&self, sandbox: &Sandbox) -> Result<()> {
        for attachment in self.repo.attachments(sandbox.owner).await? {
            if attachment.sandbox == sandbox.id {
                self.repo.detach(&sandbox.id, attachment.entry).await?;
                self.forget(sandbox.owner, overriding_secret(&attachment))
                    .await;
            }
        }
        Ok(())
    }

    pub(super) async fn remove_all(&self, owner: UserId) -> Result<()> {
        for entry in self.repo.entries(owner).await? {
            self.repo.delete(owner, entry.id).await?;
        }
        self.secrets.remove_all(owner).await
    }

    async fn registry(&self, owner: UserId) -> Result<Registry> {
        Ok(Registry::new(
            self.repo.entries(owner).await?,
            self.repo.attachments(owner).await?,
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

    async fn view(&self, owner: UserId, registry: &Registry, id: EntryId) -> Result<EntryView> {
        self.views(owner, registry, [id])
            .await?
            .pop()
            .ok_or_else(|| Error::NotFound(format!("environment entry {}", id.0)))
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
        let templates = registry.entries.iter().filter(|entry| entry.template);
        let wanted = secret_ids(templates.flat_map(|entry| {
            std::iter::once(&entry.value).chain(
                registry
                    .attachments_of(entry.id)
                    .filter_map(|a| a.overriding.as_ref()),
            )
        }));
        let secrets = self.secrets_of(owner, wanted).await?;
        let mut unresolved_in = BTreeMap::new();
        ids.into_iter()
            .map(|id| {
                let entry = registry.entry(id)?;
                let path = match &entry.target {
                    Target::File(path) if entry.template => Some(path),
                    _ => None,
                };
                let mut sandboxes: Vec<Use> = registry
                    .attachments_of(id)
                    .filter_map(|attachment| {
                        let sandbox = names.get(&attachment.sandbox)?;
                        let missing = match path {
                            Some(path) => unresolved_in
                                .entry(attachment.sandbox.clone())
                                .or_insert_with(|| {
                                    unresolved(
                                        &registry.effective(Scope::Sandbox(&attachment.sandbox)),
                                        &secrets,
                                    )
                                })
                                .get(path)
                                .cloned()
                                .unwrap_or_default(),
                            None => BTreeSet::new(),
                        };
                        Some(Use {
                            sandbox: sandbox.clone(),
                            overridden: attachment.overriding.is_some(),
                            unresolved: missing,
                        })
                    })
                    .collect();
                sandboxes.sort_by(|a, b| a.sandbox.cmp(&b.sandbox));
                Ok(EntryView {
                    references: match path {
                        Some(_) => template_of(&entry.value, &secrets)
                            .names()
                            .into_iter()
                            .cloned()
                            .collect(),
                        None => BTreeSet::new(),
                    },
                    duplicate: registry.is_duplicate(entry),
                    entry: entry.clone(),
                    sandboxes,
                })
            })
            .collect()
    }

    async fn compose(
        &self,
        owner: UserId,
        registry: &Registry,
        scope: Scope<'_>,
        name: &SandboxName,
        token: &SandboxToken,
    ) -> Result<Composed> {
        let ids = secret_ids(registry.effective(scope).into_iter().map(|(_, v)| v));
        let secrets = self.secrets_of(owner, ids).await?;
        let effective = registry.effective(scope);
        compose(
            &whose(scope, name),
            &effective,
            &secrets,
            self.links.sandbox_file(name, token),
        )
    }

    /// Refuses a change that would leave a sandbox it reaches, or new sandboxes, with colliding
    /// entries, beyond a limit, or with a template referring to a variable they do not get,
    /// which the current state did not already do.
    async fn validate(
        &self,
        owner: UserId,
        current: &Registry,
        proposed: &Registry,
        reach: &Reach,
    ) -> Result<()> {
        if reach.sandboxes.is_empty() && !reach.new {
            return Ok(());
        }
        // The token a start would issue is as long as any other, so the files it sizes are too.
        let token = SandboxToken::generate()?;
        let names = self.sandbox_names(owner).await?;
        let longest = SandboxName::parse(&"x".repeat(32)).expect("a valid sandbox name");
        let mut scopes: Vec<(Scope<'_>, &SandboxName)> = reach
            .sandboxes
            .iter()
            .filter_map(|id| Some((Scope::Sandbox(id), names.get(id)?)))
            .collect();
        if reach.new {
            scopes.push((Scope::New, &longest));
        }
        let ids = secret_ids(scopes.iter().flat_map(|(scope, _)| {
            current
                .effective(*scope)
                .into_iter()
                .chain(proposed.effective(*scope))
                .map(|(_, value)| value)
        }));
        let secrets = self.secrets_of(owner, ids).await?;
        for (scope, name) in scopes {
            let composed = compose(
                &whose(scope, name),
                &proposed.effective(scope),
                &secrets,
                self.links.sandbox_file(name, &token),
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
                    Scope::Sandbox(_) => format!(
                        "sandbox {name} could not start: template ~/{path} would refer to \
                         {names}, not attached to it"
                    ),
                    Scope::New => format!(
                        "new sandboxes could not start: template ~/{path} applies to them and \
                         would refer to {names}, which would not"
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

    /// The overrides of the entry `id` as [`Self::refit`] keeps them, each that changes.
    async fn refit_overrides(
        &self,
        owner: UserId,
        registry: &Registry,
        id: EntryId,
        change: &EntryChange,
        shift: &Shift,
        rekept: &mut Rekept,
    ) -> Result<Vec<Attachment>> {
        let names = self.sandbox_names(owner).await?;
        let mut overrides = Vec::new();
        for attachment in registry.attachments_of(id) {
            let Some(overriding) = &attachment.overriding else {
                continue;
            };
            let sandbox = names.get(&attachment.sandbox).map_or("", |n| n.as_str());
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
        owner: UserId,
        ids: BTreeSet<SecretId>,
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let ids: Vec<SecretId> = ids.into_iter().collect();
        self.secrets.get(owner, &ids).await
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
                .ok_or_else(|| Error::internal("environment", "a secret value is missing")),
        }
    }

    // Recorded after the write, so a start that loaded the earlier values began before it.
    async fn changed<'a>(&self, sandboxes: impl IntoIterator<Item = &'a SandboxId>) -> Result<()> {
        let sandboxes: Vec<SandboxId> = sandboxes.into_iter().cloned().collect();
        if sandboxes.is_empty() {
            return Ok(());
        }
        self.sandboxes
            .record_environment_change(&sandboxes, Utc::now())
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

fn secret_ids<'a>(values: impl IntoIterator<Item = &'a StoredValue>) -> BTreeSet<SecretId> {
    values
        .into_iter()
        .filter_map(StoredValue::secret_id)
        .collect()
}

fn whose(scope: Scope<'_>, name: &SandboxName) -> String {
    match scope {
        Scope::Sandbox(_) => format!("sandbox {name}"),
        Scope::New => "new sandboxes".to_owned(),
    }
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

/// Refuses a target Corral keeps for itself, and one another entry of the owner already has.
fn check_target(registry: &Registry, target: &Target, except: Option<EntryId>) -> Result<()> {
    if let Target::File(path) = target
        && path.overlaps(&HomePath::parse(OWN_DIR).expect("a valid home path"))
    {
        return Err(Error::Invalid(format!(
            "~/{path} is where Corral keeps the sandbox's own files, in ~/{OWN_DIR}"
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

fn listed(names: &BTreeSet<VariableName>) -> String {
    names
        .iter()
        .map(VariableName::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
