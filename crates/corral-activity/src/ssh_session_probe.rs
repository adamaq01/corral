use async_trait::async_trait;

use crate::{ActivityError, ActivityProbe, Exec, pgrep};

pub struct SshSessionProbe {
    pattern: String,
}

impl SshSessionProbe {
    #[must_use]
    pub fn new(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_owned(),
        }
    }
}

#[async_trait]
impl ActivityProbe for SshSessionProbe {
    async fn is_active(&self, exec: &dyn Exec) -> Result<bool, ActivityError> {
        pgrep::matches(exec, &["pgrep", "-f", &self.pattern]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ScriptedExec;

    #[tokio::test]
    async fn ssh_probe_uses_given_pattern() {
        let exec = ScriptedExec::default().exits("pgrep -f sshd: sandbox", 0, "42\n");

        let active = SshSessionProbe::new("sshd: sandbox").is_active(&exec).await;

        assert!(active.unwrap());
        assert_eq!(exec.calls(), ["pgrep -f sshd: sandbox"]);
    }

    #[tokio::test]
    async fn no_matching_session_is_inactive() {
        let exec = ScriptedExec::default().exits("pgrep -f containerssh-agent console", 1, "");

        let active = SshSessionProbe::new("containerssh-agent console")
            .is_active(&exec)
            .await;

        assert!(!active.unwrap());
    }

    #[tokio::test]
    async fn failed_exec_is_an_error() {
        let exec = ScriptedExec::default().fails("pgrep -f sshd: sandbox");

        let active = SshSessionProbe::new("sshd: sandbox").is_active(&exec).await;

        assert!(matches!(active, Err(ActivityError::Exec(_))));
    }

    #[tokio::test]
    async fn unexpected_pgrep_exit_is_an_error() {
        let exec = ScriptedExec::default().exits("pgrep -f sshd: sandbox", 127, "");

        let active = SshSessionProbe::new("sshd: sandbox").is_active(&exec).await;

        assert!(matches!(
            active,
            Err(ActivityError::UnexpectedExit { code: 127, .. })
        ));
    }
}
