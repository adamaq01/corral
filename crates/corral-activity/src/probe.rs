use corral_core::Result;

use crate::{Exec, herdr, pgrep};

const CONTAINER_INIT: &str = "1";
const FORWARD_SERVER: &str = "containerssh-agent forward-server";

/// How an SSH session shows in a sandbox's process list, which depends on how the engine
/// connects sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sessions {
    /// sshd runs in the sandbox and names each session's process `sshd: sandbox`.
    Sshd,
    /// The front door execs each session into the container, so it shows as a process whose
    /// parent is outside the container, as PID 1's is.
    Exec,
}

/// Whether the sandbox has an SSH session or a working Herdr terminal.
pub async fn is_active(sessions: Sessions, exec: &dyn Exec) -> Result<bool> {
    Ok(sessions.any(exec).await? || herdr::is_working(exec).await?)
}

impl Sessions {
    async fn any(self, exec: &dyn Exec) -> Result<bool> {
        match self {
            Self::Sshd => pgrep::matches(exec, &["pgrep", "-f", "sshd: sandbox"]).await,
            // pgrep leaves itself out, and it is this probe's own exec.
            Self::Exec => {
                let argv = ["pgrep", "-a", "-P", "0"];
                let output = exec.run(&argv).await?;
                match output.code {
                    0 | 1 => Ok(output.stdout.lines().any(is_exec_session)),
                    code => Err(pgrep::unexpected_exit(&argv, code)),
                }
            }
        }
    }
}

// A `pgrep -a` line of a process whose parent is outside the container. The ContainerSSH
// agent serves port forwards from a process that outlives the connection that started it, so
// that process never counts as a session.
fn is_exec_session(line: &str) -> bool {
    let (pid, command) = line.split_once(' ').unwrap_or((line, ""));
    pid != CONTAINER_INIT && !command.ends_with(FORWARD_SERVER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ScriptedExec;

    const SSHD: &str = "pgrep -f sshd: sandbox";
    const EXEC_ROOTS: &str = "pgrep -a -P 0";
    const HERDR_CORE: &str = "pgrep -x herdr-core";
    const HERDR_LIST: &str = "hc ls --json";

    fn no_herdr(exec: ScriptedExec) -> ScriptedExec {
        exec.exits(HERDR_CORE, 1, "")
    }

    #[tokio::test]
    async fn an_sshd_session_is_activity() {
        let session = ScriptedExec::default().exits(SSHD, 0, "42\n");
        let none = no_herdr(ScriptedExec::default().exits(SSHD, 1, ""));

        assert!(is_active(Sessions::Sshd, &session).await.unwrap());
        assert!(!is_active(Sessions::Sshd, &none).await.unwrap());
    }

    #[tokio::test]
    async fn an_exec_root_other_than_init_is_a_session() {
        let session =
            ScriptedExec::default().exits(EXEC_ROOTS, 0, "1 /usr/bin/tini\n57 /bin/bash -l\n");
        let init_alone =
            no_herdr(ScriptedExec::default().exits(EXEC_ROOTS, 0, "1 /usr/bin/tini\n"));
        let nothing = no_herdr(ScriptedExec::default().exits(EXEC_ROOTS, 1, ""));

        assert!(is_active(Sessions::Exec, &session).await.unwrap());
        assert!(!is_active(Sessions::Exec, &init_alone).await.unwrap());
        assert!(!is_active(Sessions::Exec, &nothing).await.unwrap());
    }

    #[tokio::test]
    async fn a_port_forward_server_is_not_a_session() {
        const FORWARDER: &str = "/opt/containerssh-agent/usr/bin/containerssh-agent forward-server";
        let forwards_only = no_herdr(ScriptedExec::default().exits(
            EXEC_ROOTS,
            0,
            &format!("1 /usr/bin/tini -- /usr/local/bin/corral-entrypoint\n35 {FORWARDER}\n75 {FORWARDER}\n"),
        ));
        let with_a_shell = ScriptedExec::default().exits(
            EXEC_ROOTS,
            0,
            &format!("1 /usr/bin/tini\n35 {FORWARDER}\n80 /bin/bash -l\n"),
        );

        assert!(!is_active(Sessions::Exec, &forwards_only).await.unwrap());
        assert!(is_active(Sessions::Exec, &with_a_shell).await.unwrap());
    }

    #[tokio::test]
    async fn a_working_herdr_terminal_without_a_session_is_activity() {
        let exec = ScriptedExec::default()
            .exits(SSHD, 1, "")
            .exits(HERDR_CORE, 0, "35\n")
            .exits(
                HERDR_LIST,
                0,
                r#"{"terminals":[{"state":"live","status":"working"}]}"#,
            );

        assert!(is_active(Sessions::Sshd, &exec).await.unwrap());
    }

    #[tokio::test]
    async fn a_failed_session_check_is_an_error() {
        let failed = ScriptedExec::default().fails(SSHD);
        let missing = ScriptedExec::default().exits(EXEC_ROOTS, 127, "");

        assert!(is_active(Sessions::Sshd, &failed).await.is_err());
        let message = is_active(Sessions::Exec, &missing)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("code 127"), "{message}");
    }
}
