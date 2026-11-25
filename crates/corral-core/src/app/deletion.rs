use chrono::{DateTime, TimeDelta, Utc};

use super::{CallingSandbox, Sandboxes};
use crate::domain::{Sandbox, SandboxName, User};
use crate::{Error, Result};

/// How long a deletion asked for from inside a sandbox waits, for its owner to cancel it.
pub const DELETION_GRACE: TimeDelta = TimeDelta::minutes(10);

impl Sandboxes {
    /// Whether `corral delete` inside the owner's sandboxes may schedule their deletion. Off
    /// until they turn it on.
    pub async fn deletes_from_inside(&self, owner: &User) -> Result<bool> {
        self.users.deletes_from_inside(owner.id).await
    }

    pub async fn set_deletes_from_inside(&self, owner: &User, allowed: bool) -> Result<()> {
        self.users.set_deletes_from_inside(owner.id, allowed).await
    }

    /// Schedules the calling sandbox's deletion `DELETION_GRACE` after `now`, if its owner allows
    /// that and `confirm` is its name. Asking again keeps the first schedule. The caller stops
    /// the sandbox, which then stays stopped until its owner cancels.
    pub async fn schedule_deletion(
        &self,
        calling: &CallingSandbox,
        confirm: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>> {
        if !self.deletes_from_inside(calling.owner()).await? {
            return Err(Error::Inactive(
                "deleting a sandbox from inside it is off; turn it on in your profile in the \
                 web UI"
                    .to_owned(),
            ));
        }
        let sandbox = self.find(calling.owner(), calling.name()).await?;
        if confirm != sandbox.name.as_str() {
            return Err(Error::Invalid(format!(
                "to confirm, type this sandbox's name, {}",
                sandbox.name
            )));
        }
        if let Some(at) = sandbox.delete_at {
            return Ok(at);
        }
        let at = now + DELETION_GRACE;
        self.repo.set_delete_at(&sandbox.id, Some(at)).await?;
        Ok(at)
    }

    /// Cancels the sandbox's scheduled deletion, if any. It stays stopped.
    pub async fn cancel_deletion(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.repo.set_delete_at(&sandbox.id, None).await
    }

    /// Deletes every sandbox whose deletion is due at `now`, each with the result of its delete.
    pub async fn delete_due(&self, now: DateTime<Utc>) -> Result<Vec<(Sandbox, Result<()>)>> {
        let mut deleted = Vec::new();
        for sandbox in self.repo.due_for_deletion(now).await? {
            let outcome = self.remove(&sandbox).await;
            deleted.push((sandbox, outcome));
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};

    use super::DELETION_GRACE;
    use crate::Error;
    use crate::app::{Arrival, CallingSandbox};
    use crate::domain::{HomePath, SandboxStatus, User};
    use crate::fakes::{EngineCall, World, sandbox_name};
    use crate::ports::{Readiness, SandboxEngine};

    async fn calling(w: &World, owner: &User, name: &str) -> CallingSandbox {
        let sandbox = w
            .sandboxes
            .get(owner, &sandbox_name(name))
            .await
            .unwrap()
            .sandbox;
        let files = w.engine.files_of(&sandbox.id).unwrap();
        let file = &files[&HomePath::parse(crate::app::SANDBOX_FILE).unwrap()];
        let contents: serde_json::Value =
            serde_json::from_slice(secrecy::ExposeSecret::expose_secret(&file.content)).unwrap();
        let token = contents["token"].as_str().unwrap().to_owned();
        w.sandboxes.authenticate(&token).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn deleting_from_inside_is_off_until_the_owner_turns_it_on() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.sandbox(&adam, "main").await;
        let main = calling(&w, &adam, "main").await;
        let now = Utc::now();

        assert!(!w.sandboxes.deletes_from_inside(&adam).await.unwrap());
        assert!(matches!(
            w.sandboxes.schedule_deletion(&main, "main", now).await,
            Err(Error::Inactive(message)) if message.contains("turn it on in your profile")
        ));
        w.sandboxes
            .set_deletes_from_inside(&adam, true)
            .await
            .unwrap();
        assert!(w.sandboxes.deletes_from_inside(&adam).await.unwrap());
        for wrong in ["", "Main", "main.adam", "dev"] {
            assert!(
                matches!(
                    w.sandboxes.schedule_deletion(&main, wrong, now).await,
                    Err(Error::Invalid(message)) if message.ends_with("name, main")
                ),
                "{wrong:?}"
            );
        }
        assert_eq!(
            w.sandboxes
                .get(&adam, &sandbox_name("main"))
                .await
                .unwrap()
                .sandbox
                .delete_at,
            None
        );
        w.sandboxes
            .set_deletes_from_inside(&adam, false)
            .await
            .unwrap();
        assert!(
            w.sandboxes
                .schedule_deletion(&main, "main", now)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_scheduled_sandbox_stays_stopped_until_cancelled_or_deleted_when_due() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let main = w.sandbox(&adam, "main").await;
        let keep = w.sandbox(&adam, "keep").await;
        let eves = w.sandbox(&eve, "main").await;
        w.sandboxes
            .set_deletes_from_inside(&adam, true)
            .await
            .unwrap();
        let now = Utc::now();
        let calling_main = calling(&w, &adam, "main").await;

        let at = w
            .sandboxes
            .schedule_deletion(&calling_main, "main", now)
            .await
            .unwrap();
        assert_eq!(at, now + DELETION_GRACE);
        let later = now + TimeDelta::minutes(3);
        assert_eq!(
            w.sandboxes
                .schedule_deletion(&calling_main, "main", later)
                .await
                .unwrap(),
            at,
            "asking again keeps the first schedule"
        );
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        let view = w.sandboxes.get(&adam, &main.name).await.unwrap();
        assert_eq!(
            (view.sandbox.delete_at, view.status),
            (Some(at), SandboxStatus::Sleeping)
        );
        for refused in [
            w.sandboxes.wake(&adam, &main.name).await.map(drop),
            w.sandboxes.restart(&adam, &main.name).await,
        ] {
            assert!(
                matches!(
                    &refused,
                    Err(Error::Conflict(message)) if message.contains("cancel the deletion")
                ),
                "{refused:?}"
            );
        }
        assert!(matches!(
            w.access.arrive("main.adam").await.unwrap(),
            Arrival::Refused(message) if message.contains("cancel the deletion")
        ));
        assert_eq!(
            w.engine.status(&main.id).await.unwrap(),
            SandboxStatus::Sleeping
        );

        assert_eq!(
            w.sandboxes
                .delete_due(at - TimeDelta::seconds(1))
                .await
                .unwrap()
                .len(),
            0
        );
        w.sandboxes
            .cancel_deletion(&adam, &main.name)
            .await
            .unwrap();
        w.sandboxes
            .cancel_deletion(&adam, &main.name)
            .await
            .unwrap();
        assert_eq!(w.sandboxes.delete_due(at).await.unwrap().len(), 0);
        assert!(matches!(
            w.sandboxes.wake(&adam, &main.name).await,
            Ok(Readiness::Ready)
        ));

        let calling_main = calling(&w, &adam, "main").await;
        let at = w
            .sandboxes
            .schedule_deletion(&calling_main, "main", now)
            .await
            .unwrap();
        let swept = w.sandboxes.delete_due(at).await.unwrap();
        let swept: Vec<_> = swept
            .into_iter()
            .map(|(sandbox, outcome)| (sandbox.id, outcome.is_ok()))
            .collect();
        assert_eq!(swept, [(main.id.clone(), true)]);
        assert!(matches!(
            w.sandboxes.get(&adam, &main.name).await,
            Err(Error::NotFound(_))
        ));
        assert!(
            w.engine
                .calls()
                .iter()
                .any(|call| matches!(call, EngineCall::Delete(id) if *id == main.id))
        );
        for (owner, sandbox) in [(&adam, &keep), (&eve, &eves)] {
            assert!(w.sandboxes.get(owner, &sandbox.name).await.is_ok());
        }
        assert_eq!(w.sandboxes.delete_due(at).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_failed_delete_is_reported_and_retried_at_the_next_sweep() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        w.sandboxes
            .set_deletes_from_inside(&adam, true)
            .await
            .unwrap();
        let calling_main = calling(&w, &adam, "main").await;
        let at = w
            .sandboxes
            .schedule_deletion(&calling_main, "main", Utc::now())
            .await
            .unwrap();

        w.engine.fail_deletes_after(Some(0));
        let swept = w.sandboxes.delete_due(at).await.unwrap();
        assert!(matches!(swept.as_slice(), [(sandbox, Err(_))] if sandbox.id == main.id));
        w.engine.fail_deletes_after(None);
        let swept = w.sandboxes.delete_due(at).await.unwrap();
        assert!(matches!(swept.as_slice(), [(sandbox, Ok(()))] if sandbox.id == main.id));
    }
}
