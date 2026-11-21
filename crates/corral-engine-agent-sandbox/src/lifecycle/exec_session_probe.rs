use async_trait::async_trait;
use corral_activity::{ActivityError, ActivityProbe, Exec};

const CONTAINER_INIT: &str = "1";

// ContainerSSH's agent replaces itself with the session's program, so a session shows only as a
// pod exec: a process whose parent is outside the container, as PID 1's is. pgrep omits itself,
// and it is this probe's own exec.
pub(crate) struct ExecSessionProbe;

#[async_trait]
impl ActivityProbe for ExecSessionProbe {
    async fn is_active(&self, exec: &dyn Exec) -> Result<bool, ActivityError> {
        let output = exec.run(&["pgrep", "-P", "0"]).await?;
        match output.code {
            0 | 1 => Ok(output.stdout.lines().any(|pid| pid != CONTAINER_INIT)),
            code => Err(ActivityError::UnexpectedExit {
                command: "pgrep -P 0".to_owned(),
                code,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::testing::ScriptedExec;

    const EXEC_ROOTS: &str = "pgrep -P 0";

    async fn active(exec: ScriptedExec) -> Result<bool, ActivityError> {
        ExecSessionProbe.is_active(&exec).await
    }

    #[tokio::test]
    async fn another_exec_root_is_a_session() {
        let exec = ScriptedExec::default().exits(EXEC_ROOTS, 0, "1\n57\n");

        assert!(active(exec).await.unwrap());
    }

    #[tokio::test]
    async fn the_container_init_alone_is_no_session() {
        let exec = ScriptedExec::default().exits(EXEC_ROOTS, 0, "1\n");

        assert!(!active(exec).await.unwrap());
    }

    #[tokio::test]
    async fn failed_pgrep_is_an_error() {
        let exited = ScriptedExec::default().exits(EXEC_ROOTS, 127, "");
        let failed = ScriptedExec::default().fails(EXEC_ROOTS);

        assert!(matches!(
            active(exited).await,
            Err(ActivityError::UnexpectedExit { code: 127, .. })
        ));
        assert!(matches!(active(failed).await, Err(ActivityError::Exec(_))));
    }
}
