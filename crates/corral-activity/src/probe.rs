use corral_core::Result;

use crate::{Exec, forward, herdr, pgrep};

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
                if !matches!(output.code, 0 | 1) {
                    return Err(pgrep::unexpected_exit(&argv, output.code));
                }
                let mut forwards = Vec::new();
                for line in output.stdout.lines() {
                    match ExecRoot::parse(line) {
                        ExecRoot::Init => {}
                        ExecRoot::PortForward(pid) => forwards.push(pid),
                        ExecRoot::Session => return Ok(true),
                    }
                }
                forward::any_open(exec, &forwards).await
            }
        }
    }
}

/// A process whose parent is outside the container, from a `pgrep -a` line.
enum ExecRoot<'a> {
    Init,
    // The ContainerSSH agent's port-forward process outlives the SSH connection that started
    // it, so it counts only while it carries a forwarded connection.
    PortForward(&'a str),
    Session,
}

impl<'a> ExecRoot<'a> {
    fn parse(line: &'a str) -> Self {
        let (pid, command) = line.split_once(' ').unwrap_or((line, ""));
        if pid == CONTAINER_INIT {
            Self::Init
        } else if command.ends_with(FORWARD_SERVER) {
            Self::PortForward(pid)
        } else {
            Self::Session
        }
    }
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

    // Captured on kind from ContainerSSH 0.6.1 and agent v0.11.0: three SSH connections have
    // forwarded a port, and the last one carries an HTTP response through `ssh -L`.
    const FORWARD_ROOTS: &str = "1 /usr/bin/tini -- /usr/local/bin/corral-entrypoint
92 /opt/containerssh-agent/usr/bin/containerssh-agent forward-server
216 /opt/containerssh-agent/usr/bin/containerssh-agent forward-server
528 /opt/containerssh-agent/usr/bin/containerssh-agent forward-server
";
    const FORWARD_FDS: &str = "ls -l /proc/92/fd /proc/216/fd /proc/528/fd";
    const TCP_TABLES: &str = "cat /proc/net/tcp /proc/net/tcp6";
    const FDS_OPEN: &str = "/proc/216/fd:
total 0
lr-x------ 1 sandbox sandbox 64 Nov 22 23:24 0 -> pipe:[5984947]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:24 1 -> pipe:[5984948]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:24 2 -> pipe:[5984949]
lr-x------ 1 sandbox sandbox 64 Nov 22 23:24 3 -> /sys/fs/cgroup/cpu.max
lrwx------ 1 sandbox sandbox 64 Nov 22 23:24 5 -> anon_inode:[eventpoll]
lrwx------ 1 sandbox sandbox 64 Nov 22 23:24 6 -> anon_inode:[eventfd]

/proc/528/fd:
total 0
lr-x------ 1 sandbox sandbox 64 Nov 22 23:26 0 -> pipe:[6003339]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:26 1 -> pipe:[6003340]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:26 2 -> pipe:[6003341]
lr-x------ 1 sandbox sandbox 64 Nov 22 23:26 3 -> /sys/fs/cgroup/cpu.max
lrwx------ 1 sandbox sandbox 64 Nov 22 23:26 4 -> socket:[6011346]
lrwx------ 1 sandbox sandbox 64 Nov 22 23:26 5 -> anon_inode:[eventpoll]
lrwx------ 1 sandbox sandbox 64 Nov 22 23:26 6 -> anon_inode:[eventfd]

/proc/92/fd:
total 0
lr-x------ 1 sandbox sandbox 64 Nov 22 23:24 0 -> pipe:[5998649]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:24 1 -> pipe:[5998650]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:24 2 -> pipe:[5998651]
lr-x------ 1 sandbox sandbox 64 Nov 22 23:24 3 -> /sys/fs/cgroup/cpu.max
lrwx------ 1 sandbox sandbox 64 Nov 22 23:24 5 -> anon_inode:[eventpoll]
lrwx------ 1 sandbox sandbox 64 Nov 22 23:24 6 -> anon_inode:[eventfd]
";
    const TCP_OPEN: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F40 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5998623 1 0000000094470c97 100 0 0 10 0
   1: 0100007F:1F40 0100007F:B6AA 01 00000000:00000000 00:00000000 00000000  1000        0 6003343 1 00000000c808fc3d 20 4 31 11 -1
   2: 0100007F:B6AA 0100007F:1F40 01 00000000:00000000 02:00000495 00000000  1000        0 6011346 2 0000000003fc3469 20 4 30 11 -1
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
";
    // The same agents once the HTTP response ends, the tunnel still open.
    const FDS_CLOSED: &str = "/proc/528/fd:
total 0
lr-x------ 1 sandbox sandbox 64 Nov 22 23:26 0 -> pipe:[6003339]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:26 1 -> pipe:[6003340]
l-wx------ 1 sandbox sandbox 64 Nov 22 23:26 2 -> pipe:[6003341]
lr-x------ 1 sandbox sandbox 64 Nov 22 23:26 3 -> /sys/fs/cgroup/cpu.max
lrwx------ 1 sandbox sandbox 64 Nov 22 23:26 5 -> anon_inode:[eventpoll]
lrwx------ 1 sandbox sandbox 64 Nov 22 23:26 6 -> anon_inode:[eventfd]
";
    const TCP_CLOSED: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F40 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5998623 1 0000000094470c97 100 0 0 10 0
   1: 0100007F:B6AA 0100007F:1F40 06 00000000:00000000 03:00001692 00000000     0        0 0 3 0000000028a3686c
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
";

    fn forwards(fds: &str, fds_code: i32, tables: &str) -> ScriptedExec {
        no_herdr(
            ScriptedExec::default()
                .exits(EXEC_ROOTS, 0, FORWARD_ROOTS)
                .exits(FORWARD_FDS, fds_code, fds)
                .exits(TCP_TABLES, 0, tables),
        )
    }

    #[tokio::test]
    async fn a_port_forward_is_a_session_while_it_carries_a_connection() {
        let open = forwards(FDS_OPEN, 0, TCP_OPEN);
        let closed = forwards(FDS_CLOSED, 0, TCP_CLOSED);
        let closed_between_listings = forwards(FDS_OPEN, 0, TCP_CLOSED);

        assert!(is_active(Sessions::Exec, &open).await.unwrap());
        assert!(!is_active(Sessions::Exec, &closed).await.unwrap());
        assert!(
            !is_active(Sessions::Exec, &closed_between_listings)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn a_port_forward_that_exits_mid_probe_leaves_the_others_counted() {
        let exited =
            format!("ls: cannot access '/proc/92/fd': No such file or directory\n{FDS_OPEN}");

        assert!(
            is_active(Sessions::Exec, &forwards(&exited, 2, TCP_OPEN))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn an_unreadable_port_forward_is_an_error() {
        let missing = forwards("", 127, TCP_OPEN);

        let message = is_active(Sessions::Exec, &missing)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("code 127"), "{message}");
    }

    #[tokio::test]
    async fn a_shell_beside_idle_port_forwards_is_a_session() {
        let with_a_shell = ScriptedExec::default().exits(
            EXEC_ROOTS,
            0,
            &format!("{FORWARD_ROOTS}80 /bin/bash -l\n"),
        );

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
