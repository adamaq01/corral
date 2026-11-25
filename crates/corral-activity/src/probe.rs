use corral_core::Result;

use crate::{Exec, herdr, processes};

const SANDBOX_USER: &str = "sandbox";

/// Whether the sandbox has an SSH session or a working Herdr terminal. Every session is an
/// `sshd -i` the gate runs in the sandbox, which names its processes after the sandbox user.
pub async fn is_active(exec: &dyn Exec) -> Result<bool> {
    let processes = processes::list(exec).await?;
    Ok(processes
        .iter()
        .any(|process| is_sshd_session(&process.args))
        || herdr::is_working(&processes, exec).await?)
}

// OpenSSH names a session's processes `sshd: sandbox [priv]` and `sshd: sandbox@pts/0`, and from
// 9.8 on `sshd-session: ...`.
fn is_sshd_session(args: &str) -> bool {
    ["sshd: ", "sshd-session: "]
        .iter()
        .filter_map(|prefix| args.strip_prefix(prefix)?.strip_prefix(SANDBOX_USER))
        .any(|rest| rest.is_empty() || rest.starts_with(['@', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::tests::{LIST, listing};
    use crate::testing::ScriptedExec;

    const HERDR_LIST: &str = "/opt/corral/bin/hc ls --json";
    const INIT: (u32, u32, &str) = (
        1,
        0,
        "/opt/corral/bin/tini -- /opt/corral/bin/corral-init idle",
    );

    fn listed(processes: &[(u32, u32, &str)]) -> ScriptedExec {
        ScriptedExec::default().exits(LIST, 0, &listing(processes))
    }

    #[tokio::test]
    async fn an_sshd_session_of_any_openssh_is_activity() {
        for session in [
            "sshd-session: sandbox [priv]",
            "sshd-session: sandbox@pts/0",
            "sshd-session: sandbox@notty",
            "sshd: sandbox [priv]",
            "sshd: sandbox@pts/1",
        ] {
            let exec = listed(&[INIT, (40, 0, session)]);
            assert!(is_active(&exec).await.unwrap(), "{session}");
        }
        for other in [
            "sshd-session: sandboxer@pts/0",
            "sshd-auth: sandbox [net]",
            "/opt/corral/sbin/sshd -i -e -f /etc/corral/sshd_env.conf",
            "/bin/bash -c sshd: sandbox",
            "/bin/bash -l",
        ] {
            let exec = listed(&[INIT, (7, 0, other)]);
            assert!(!is_active(&exec).await.unwrap(), "{other}");
        }
    }

    #[tokio::test]
    async fn a_working_herdr_terminal_without_a_session_is_activity() {
        let exec = listed(&[INIT, (35, 1, "/opt/corral/bin/herdr-core serve")]).exits(
            HERDR_LIST,
            0,
            r#"{"terminals":[{"state":"live","status":"working"}]}"#,
        );

        assert!(is_active(&exec).await.unwrap());
    }

    #[tokio::test]
    async fn a_failed_listing_is_an_error() {
        let failed = ScriptedExec::default().fails(LIST);
        let missing = ScriptedExec::default().exits(LIST, 127, "");

        assert!(is_active(&failed).await.is_err());
        let message = is_active(&missing).await.unwrap_err().to_string();
        assert!(message.contains("code 127"), "{message}");
    }
}
