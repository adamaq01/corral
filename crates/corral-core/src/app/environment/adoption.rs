use chrono::Utc;
use secrecy::{ExposeSecret, SecretSlice};

use super::Environment;
use crate::Result;
use crate::domain::{Attachment, Entry, EntryId, Target, UserId};

impl Environment {
    /// Makes each credential kept before entries existed a secret variable that applies to new
    /// sandboxes, attached to each sandbox its owner has, then forgets the credential. Picks up
    /// where an interrupted run stopped. Returns how many it made.
    pub async fn adopt_credentials(&self) -> Result<usize> {
        let mut made = 0;
        for user in self.repo.awaiting_adoption().await? {
            made += self.adopt(user).await?;
            self.repo.adopted(user).await?;
        }
        Ok(made)
    }

    async fn adopt(&self, user: UserId) -> Result<usize> {
        let legacy = self.secrets.legacy_credentials(user).await?;
        if legacy.is_empty() {
            return Ok(0);
        }
        let sandboxes = self.sandboxes.list(user).await?;
        let mut made = 0;
        for (name, value) in legacy {
            let target = Target::Variable(name);
            let registry = self.registry(user).await?;
            let id = if let Some(adopted) =
                registry.entries.iter().find(|entry| entry.target == target)
            {
                adopted.id
            } else {
                let value = SecretSlice::from(value.expose_secret().as_bytes().to_vec());
                let entry = Entry {
                    id: EntryId::generate(),
                    owner: user,
                    template: false,
                    executable: false,
                    apply_to_new: true,
                    value: self.keep(user, &target, true, &value).await?,
                    target,
                    created_at: Utc::now(),
                };
                self.settle(user, [&entry.value], self.repo.insert(&entry))
                    .await?;
                made += 1;
                entry.id
            };
            for sandbox in &sandboxes {
                if registry.attachment(&sandbox.id, id).is_none() {
                    self.repo
                        .attach(&Attachment {
                            sandbox: sandbox.id.clone(),
                            entry: id,
                            overriding: None,
                        })
                        .await?;
                }
            }
        }
        self.secrets.forget_legacy_credentials(user).await?;
        Ok(made)
    }
}
