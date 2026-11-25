use secrecy::SecretSlice;
use serde_json::json;

use super::Sandboxes;
use crate::Result;
use crate::domain::{
    FileMode, HomeFile, HomePath, Sandbox, SandboxId, SandboxName, SandboxStatus, SandboxToken,
    TokenHash, User, UserStatus,
};

/// Where the `corral` command in a sandbox finds what it needs, relative to the home.
pub const SANDBOX_FILE: &str = ".config/corral/sandbox.json";

/// Where a sandbox reaches Corral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxLinks {
    /// The base URL of the API, as sandboxes reach it.
    pub api: String,
    /// The base URL of the web UI, as people open it.
    pub web: String,
}

impl SandboxLinks {
    /// The file a start writes into the sandbox's home: the links, its name and the token of
    /// that start, readable only by the sandbox user.
    pub(super) fn sandbox_file(
        &self,
        sandbox: &SandboxName,
        token: &SandboxToken,
    ) -> (HomePath, HomeFile) {
        let contents = json!({
            "api": self.api,
            "web": self.web,
            "sandbox": sandbox,
            "token": token.expose(),
        });
        let path = HomePath::parse(SANDBOX_FILE).expect("a valid home path");
        let file = HomeFile {
            mode: FileMode::OWNER_ONLY,
            content: SecretSlice::from(format!("{contents:#}\n").into_bytes()),
        };
        (path, file)
    }
}

/// A running sandbox, proven by the token Corral issued it at its start, and its active owner.
#[derive(Clone, Debug)]
pub struct CallingSandbox {
    sandbox: Sandbox,
    owner: User,
}

impl CallingSandbox {
    #[must_use]
    pub fn owner(&self) -> &User {
        &self.owner
    }

    #[must_use]
    pub fn name(&self) -> &SandboxName {
        &self.sandbox.name
    }

    #[must_use]
    pub fn id(&self) -> &SandboxId {
        &self.sandbox.id
    }
}

impl Sandboxes {
    /// The sandbox `presented` is the current token of, while it runs and its owner is active.
    pub async fn authenticate(&self, presented: &str) -> Result<Option<CallingSandbox>> {
        let Some(sandbox) = self.repo.by_token(&TokenHash::of(presented)).await? else {
            return Ok(None);
        };
        // An idle sleep stops a sandbox without Corral, so a token is only good while it runs.
        if !matches!(
            self.engine.status(&sandbox.id).await,
            Ok(SandboxStatus::Running)
        ) {
            return Ok(None);
        }
        let owner = self.users.get(sandbox.owner).await?;
        Ok(owner
            .filter(|owner| owner.status == UserStatus::Active)
            .map(|owner| CallingSandbox { sandbox, owner }))
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;
    use serde_json::{Value, json};

    use super::SANDBOX_FILE;
    use crate::domain::{HomePath, SandboxId, UserStatus};
    use crate::fakes::{World, sandbox_name};
    use crate::ports::{SandboxEngine, UserRepo};

    /// The token the sandbox's last boot wrote into its home, and the rest of that file.
    fn issued(w: &World, id: &SandboxId) -> (String, Value) {
        let files = w.engine.files_of(id).unwrap();
        let file = &files[&HomePath::parse(SANDBOX_FILE).unwrap()];
        assert_eq!(file.mode.bits(), 0o600);
        let mut contents: Value = serde_json::from_slice(file.content.expose_secret()).unwrap();
        let token = contents["token"].as_str().unwrap().to_owned();
        contents["token"] = Value::Null;
        (token, contents)
    }

    async fn holder(w: &World, token: &str) -> Option<String> {
        w.sandboxes
            .authenticate(token)
            .await
            .unwrap()
            .map(|calling| format!("{}.{}", calling.name(), calling.owner().handle))
    }

    #[tokio::test]
    async fn each_boot_issues_the_sandbox_a_token_for_itself_alone() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let main = w.sandbox(&adam, "main").await;
        let theirs = w.sandbox(&eve, "main").await;

        let (token, rest) = issued(&w, &main.id);
        assert_eq!(
            rest,
            json!({
                "api": "http://corral.internal:8080",
                "web": "https://corral.example.com",
                "sandbox": "main",
                "token": null,
            })
        );
        assert_eq!(holder(&w, &token).await.as_deref(), Some("main.adam"));
        let (eves, _) = issued(&w, &theirs.id);
        assert_eq!(holder(&w, &eves).await.as_deref(), Some("main.eve"));
        assert_ne!(token, eves);
        for wrong in ["", "corral_sbx_0", &token[..token.len() - 1]] {
            assert_eq!(holder(&w, wrong).await, None, "{wrong:?}");
        }

        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        assert_eq!(
            holder(&w, &token).await.as_deref(),
            Some("main.adam"),
            "a wake of a running sandbox keeps its token"
        );
        w.sandboxes.restart(&adam, &main.name).await.unwrap();
        let (rotated, _) = issued(&w, &main.id);
        assert_ne!(rotated, token);
        assert_eq!(
            holder(&w, &token).await,
            None,
            "a restart revokes the old one"
        );
        assert_eq!(holder(&w, &rotated).await.as_deref(), Some("main.adam"));
    }

    #[tokio::test]
    async fn a_token_dies_when_its_sandbox_stops_sleeps_or_goes() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        let (token, _) = issued(&w, &main.id);

        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        assert_eq!(holder(&w, &token).await, None, "a stop revokes it");
        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        assert_eq!(
            holder(&w, &token).await,
            None,
            "a wake does not bring it back"
        );

        let (woken, _) = issued(&w, &main.id);
        w.engine.stop(&main.id).await.unwrap();
        assert_eq!(
            holder(&w, &woken).await,
            None,
            "an idle sleep, which only the engine sees, leaves it unusable"
        );
        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        let (current, _) = issued(&w, &main.id);
        assert_eq!(holder(&w, &woken).await, None);
        assert_eq!(holder(&w, &current).await.as_deref(), Some("main.adam"));

        w.users
            .update(&crate::domain::User {
                status: UserStatus::Disabled,
                ..adam.clone()
            })
            .await
            .unwrap();
        assert_eq!(holder(&w, &current).await, None, "a disabled owner's");
        w.users.update(&adam).await.unwrap();
        assert_eq!(holder(&w, &current).await.as_deref(), Some("main.adam"));

        w.sandboxes
            .delete(&adam, &sandbox_name("main"))
            .await
            .unwrap();
        w.sandbox(&adam, "main").await;
        assert_eq!(holder(&w, &current).await, None, "a delete revokes it");
    }
}
