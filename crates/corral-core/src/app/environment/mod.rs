mod adoption;
mod compose;
mod registry;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use secrecy::{ExposeSecret, SecretSlice};

use compose::{Composed, amount, compose};
pub use compose::{FILE_LIMIT, FILES_LIMIT, VARIABLES_LIMIT};
use registry::Registry;

use super::inside::{SANDBOX_FILE, SandboxLinks};
use crate::domain::{
    Attachment, CredentialName, Entry, EntryId, Glimpse, HomePath, Kept, Sandbox, SandboxId,
    SandboxName, SandboxToken, SecretId, SecretRef, StoredValue, Target, Template, User, UserId,
};
use crate::ports::{Boot, EnvironmentRepo, SandboxRepo, SecretStore};
use crate::{Error, Result};

/// Each user's variables and files, and the sandboxes they are attached to.
#[derive(Clone)]
pub struct Environment {
    repo: Arc<dyn EnvironmentRepo>,
    secrets: Arc<dyn SecretStore>,
    sandboxes: Arc<dyn SandboxRepo>,
    links: SandboxLinks,
}

/// An entry as its owner first gives it.
pub struct NewEntry {
    pub target: Target,
    pub secret: bool,
    pub template: bool,
    pub apply_to_new: bool,
    pub value: SecretSlice<u8>,
}

/// An entry's flags as its owner sets them, and its value unless `None`.
pub struct EntryChange {
    pub value: Option<SecretSlice<u8>>,
    pub template: bool,
    pub apply_to_new: bool,
}

/// An entry with the sandboxes it is attached to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryView {
    pub entry: Entry,
    pub sandboxes: Vec<SandboxName>,
}

/// An entry attached to a sandbox, with any value it has there in place of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub view: EntryView,
    pub overriding: Option<StoredValue>,
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
        let sandboxes = self.sandboxes.list(owner.id).await?;
        Ok(registry
            .entries
            .iter()
            .map(|entry| view(&registry, &sandboxes, entry))
            .collect())
    }

    pub async fn get(&self, owner: &User, id: EntryId) -> Result<EntryView> {
        let registry = self.registry(owner.id).await?;
        let sandboxes = self.sandboxes.list(owner.id).await?;
        Ok(view(&registry, &sandboxes, registry.entry(id)?))
    }

    /// Attached to no sandbox yet: with `apply_to_new`, to each one created from now on.
    pub async fn create(&self, owner: &User, new: NewEntry) -> Result<EntryView> {
        check_value(&new.target, new.template, &new.value)?;
        if let Target::File(path) = &new.target
            && path.overlaps(&own_path())
        {
            return Err(Error::Invalid(format!(
                "{path} is where Corral writes the sandbox's own settings, {SANDBOX_FILE}"
            )));
        }
        let registry = self.registry(owner.id).await?;
        if new.template {
            known_in_registry(&registry, &new.target, &new.value)?;
        }
        if new.apply_to_new {
            registry.check_applies_to_new(&new.target, None)?;
        }
        let entry = Entry {
            id: EntryId::generate(),
            owner: owner.id,
            template: new.template,
            apply_to_new: new.apply_to_new,
            value: self
                .keep(owner.id, &new.target, new.secret, &new.value)
                .await?,
            target: new.target,
            created_at: Utc::now(),
        };
        self.settle(owner.id, &entry.value, self.repo.insert(&entry))
            .await?;
        Ok(EntryView {
            entry,
            sandboxes: Vec::new(),
        })
    }

    /// A new value reaches each sandbox attached without an override at its next start, and a
    /// new template flag each one attached.
    pub async fn change(
        &self,
        owner: &User,
        id: EntryId,
        change: EntryChange,
    ) -> Result<EntryView> {
        let registry = self.registry(owner.id).await?;
        let before = registry.entry(id)?.clone();
        let content = match &change.value {
            Some(value) => {
                check_value(&before.target, change.template, value)?;
                Some(SecretSlice::from(value.expose_secret().to_vec()))
            }
            None if change.template && !before.template => {
                let current = self.content(owner.id, &before.value).await?;
                check_value(&before.target, true, &current)?;
                Some(current)
            }
            None => None,
        };
        if change.template
            && let Some(content) = &content
        {
            known_in_registry(&registry, &before.target, content)?;
        }
        if change.apply_to_new && !before.apply_to_new {
            registry.check_applies_to_new(&before.target, Some(id))?;
        }
        let mut after = Entry {
            template: change.template,
            apply_to_new: change.apply_to_new,
            ..before.clone()
        };
        if let Some(value) = &change.value {
            after.value = self
                .keep(owner.id, &before.target, before.is_secret(), value)
                .await?;
        }
        let affected: Vec<SandboxId> = registry
            .attachments_of(id)
            .filter(|attachment| {
                after.template != before.template
                    || (change.value.is_some() && attachment.overriding.is_none())
            })
            .map(|attachment| attachment.sandbox.clone())
            .collect();
        let mut proposed = registry.clone();
        proposed.put_entry(after.clone());
        self.settle(owner.id, &after.value, async {
            self.validate(owner.id, &proposed, &affected, id).await?;
            if self.repo.update(&after).await? {
                Ok(())
            } else {
                Err(Error::NotFound(format!("environment entry {}", id.0)))
            }
        })
        .await?;
        if change.value.is_some() {
            self.forget(owner.id, before.value.secret_id()).await;
        }
        self.changed(&affected).await?;
        let sandboxes = self.sandboxes.list(owner.id).await?;
        Ok(view(&proposed, &sandboxes, &after))
    }

    /// Detaches the entry from every sandbox, which loses it at its next start.
    pub async fn delete(&self, owner: &User, id: EntryId) -> Result<()> {
        let registry = self.registry(owner.id).await?;
        let entry = registry.entry(id)?;
        let attachments: Vec<&Attachment> = registry.attachments_of(id).collect();
        if !self.repo.delete(owner.id, id).await? {
            return Err(Error::NotFound(format!("environment entry {}", id.0)));
        }
        self.forget(owner.id, entry.value.secret_id()).await;
        for attachment in &attachments {
            self.forget(owner.id, overriding_secret(attachment)).await;
        }
        let sandboxes: Vec<SandboxId> = attachments.iter().map(|a| a.sandbox.clone()).collect();
        self.changed(&sandboxes).await
    }

    /// The entries attached to the owner's sandbox `name`.
    pub async fn of_sandbox(&self, owner: &User, name: &SandboxName) -> Result<Vec<Attached>> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let sandboxes = self.sandboxes.list(owner.id).await?;
        Ok(registry
            .attached(&sandbox.id)
            .into_iter()
            .map(|(entry, attachment)| Attached {
                view: view(&registry, &sandboxes, entry),
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
        let mut proposed = registry;
        proposed.put_attachment(attachment.clone());
        self.validate(owner.id, &proposed, std::slice::from_ref(&sandbox.id), id)
            .await?;
        self.repo.attach(&attachment).await?;
        self.changed(&[sandbox.id]).await
    }

    /// A template file attached to the sandbox that names the entry's variable then fails the
    /// sandbox's next start, which reports it.
    pub async fn detach(&self, owner: &User, name: &SandboxName, id: EntryId) -> Result<()> {
        let sandbox = self.sandbox(owner, name).await?;
        let registry = self.registry(owner.id).await?;
        let attachment = attached(&registry, &sandbox, id)?;
        self.repo.detach(&sandbox.id, id).await?;
        self.forget(owner.id, overriding_secret(attachment)).await;
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
                check_value(&entry.target, entry.template, value)?;
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
        let work = async {
            self.validate(owner.id, &proposed, std::slice::from_ref(&sandbox.id), id)
                .await?;
            self.repo.attach(&after).await
        };
        match &after.overriding {
            Some(value) => self.settle(owner.id, value, work).await?,
            None => work.await?,
        }
        self.forget(owner.id, overriding_secret(&before)).await;
        self.changed(&[sandbox.id]).await
    }

    /// What the sandbox gets at a start that issues it `token`. Refuses when a template file
    /// attached to it names a variable that is not.
    pub(super) async fn boot(&self, sandbox: &Sandbox, token: &SandboxToken) -> Result<Boot> {
        let registry = self.registry(sandbox.owner).await?;
        let composed = self.compose(&registry, sandbox, token).await?;
        if let Some((path, names)) = composed.unknown.first_key_value() {
            return Err(Error::Invalid(format!(
                "sandbox {} cannot start: template {path} refers to {}, not attached to it",
                sandbox.name,
                listed(names)
            )));
        }
        Ok(composed.boot)
    }

    /// Attaches a new sandbox to each entry that applies to new sandboxes.
    pub(super) async fn attach_new(&self, sandbox: &Sandbox) -> Result<()> {
        for entry in self.repo.entries(sandbox.owner).await? {
            if entry.apply_to_new {
                self.repo
                    .attach(&Attachment {
                        sandbox: sandbox.id.clone(),
                        entry: entry.id,
                        overriding: None,
                    })
                    .await?;
            }
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

    async fn compose(
        &self,
        registry: &Registry,
        sandbox: &Sandbox,
        token: &SandboxToken,
    ) -> Result<Composed> {
        let effective = registry.effective(&sandbox.id);
        let ids: Vec<SecretId> = effective
            .iter()
            .filter_map(|(_, value)| value.secret_id())
            .collect();
        let secrets = if ids.is_empty() {
            BTreeMap::default()
        } else {
            self.secrets.get(sandbox.owner, &ids).await?
        };
        compose(
            &sandbox.name,
            &effective,
            &secrets,
            self.links.sandbox_file(sandbox, token),
        )
    }

    /// Refuses a change that would leave one of `sandboxes` with colliding entries, beyond a
    /// limit, or with the entry `saved`, a template, naming a variable not attached to it.
    async fn validate(
        &self,
        owner: UserId,
        proposed: &Registry,
        sandboxes: &[SandboxId],
        saved: EntryId,
    ) -> Result<()> {
        if sandboxes.is_empty() {
            return Ok(());
        }
        // The token a start would issue is as long as any other, so the files it sizes are too.
        let token = SandboxToken::generate()?;
        let theirs = self.sandboxes.list(owner).await?;
        let target = &proposed.entry(saved)?.target;
        for sandbox in theirs.iter().filter(|s| sandboxes.contains(&s.id)) {
            let composed = self.compose(proposed, sandbox, &token).await?;
            if let Target::File(path) = target
                && let Some(names) = composed.unknown.get(path)
            {
                return Err(Error::Invalid(format!(
                    "template {path} refers to {}, not attached to sandbox {}",
                    listed(names),
                    sandbox.name
                )));
            }
        }
        Ok(())
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

    /// Runs `work`, removing the secret `kept` holds when it fails.
    async fn settle<T>(
        &self,
        owner: UserId,
        kept: &StoredValue,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let done = work.await;
        if done.is_err() {
            self.forget(owner, kept.secret_id()).await;
        }
        done
    }

    // A value left behind only takes room in the store, and the owner's deletion removes it.
    async fn forget(&self, owner: UserId, id: Option<SecretId>) {
        if let Some(id) = id {
            let _ = self.secrets.remove(owner, id).await;
        }
    }

    async fn content(&self, owner: UserId, value: &StoredValue) -> Result<SecretSlice<u8>> {
        match &value.kept {
            Kept::Plain(content) => Ok(SecretSlice::from(content.clone())),
            Kept::Secret(secret) => self
                .secrets
                .get(owner, &[secret.id])
                .await?
                .remove(&secret.id)
                .ok_or_else(|| Error::internal("environment", "a secret value is missing")),
        }
    }

    // Recorded after the write, so a start that loaded the earlier values began before it.
    async fn changed(&self, sandboxes: &[SandboxId]) -> Result<()> {
        if sandboxes.is_empty() {
            return Ok(());
        }
        self.sandboxes
            .record_environment_change(sandboxes, Utc::now())
            .await
    }
}

fn view(registry: &Registry, sandboxes: &[Sandbox], entry: &Entry) -> EntryView {
    let attached: BTreeSet<&SandboxId> = registry
        .attachments_of(entry.id)
        .map(|attachment| &attachment.sandbox)
        .collect();
    EntryView {
        entry: entry.clone(),
        sandboxes: sandboxes
            .iter()
            .filter(|sandbox| attached.contains(&sandbox.id))
            .map(|sandbox| sandbox.name.clone())
            .collect(),
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

fn own_path() -> HomePath {
    HomePath::parse(SANDBOX_FILE).expect("a valid home path")
}

/// Refuses a value its target cannot take: a variable takes one line of text, a file at most
/// [`FILE_LIMIT`], and only a text file can be a template.
fn check_value(target: &Target, template: bool, value: &SecretSlice<u8>) -> Result<()> {
    let bytes = value.expose_secret();
    match target {
        Target::Variable(name) => {
            if template {
                return Err(Error::Invalid(format!(
                    "variable {name} cannot be a template; only a file can"
                )));
            }
            let one_line =
                std::str::from_utf8(bytes).is_ok_and(|text| !text.contains(['\n', '\r', '\0']));
            if !one_line {
                return Err(Error::Invalid(format!(
                    "variable {name} must be one line of text without NUL bytes; \
                     base64-encode other values, or make them a file"
                )));
            }
            if name.as_str().len() + bytes.len() > VARIABLES_LIMIT {
                return Err(Error::LimitReached(format!(
                    "variable {name} takes more than the {} a sandbox's variables may take",
                    amount(VARIABLES_LIMIT)
                )));
            }
        }
        Target::File(path) => {
            if bytes.len() > FILE_LIMIT {
                return Err(Error::LimitReached(format!(
                    "file {path} is {} bytes, more than the {} a file may hold",
                    bytes.len(),
                    amount(FILE_LIMIT)
                )));
            }
            if template {
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    Error::Invalid(format!("template {path} must be text, in UTF-8"))
                })?;
                Template::parse(text).map_err(|error| match error {
                    Error::Invalid(problem) => {
                        Error::Invalid(format!("template {path}: {problem}"))
                    }
                    other => other,
                })?;
            }
        }
    }
    Ok(())
}

/// Refuses a template naming anything but one of the owner's variables.
fn known_in_registry(
    registry: &Registry,
    target: &Target,
    content: &SecretSlice<u8>,
) -> Result<()> {
    let text = String::from_utf8_lossy(content.expose_secret());
    let template = Template::parse(&text)?;
    let variables = registry.variables();
    let unknown: BTreeSet<CredentialName> = template
        .names()
        .into_iter()
        .filter(|name| !variables.contains(name))
        .cloned()
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "{target} refers to {}, not among your variables",
            listed(&unknown)
        )))
    }
}

fn listed(names: &BTreeSet<CredentialName>) -> String {
    names
        .iter()
        .map(CredentialName::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
