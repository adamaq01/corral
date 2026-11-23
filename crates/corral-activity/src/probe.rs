use corral_core::Result;

use crate::processes::{self, Process};
use crate::{Exec, forward, herdr};

const CONTAINER_INIT: u32 = 1;
const OUTSIDE: u32 = 0;
const FORWARD_SERVER: &str = "containerssh-agent forward-server";
const SANDBOX_USER: &str = "sandbox";

/// How an SSH session shows in a sandbox's process list, which depends on how the engine
/// connects sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sessions {
    /// sshd runs in the sandbox and names each session's processes after the sandbox user.
    Sshd,
    /// The front door execs each session into the container, so it shows as a process whose
    /// parent is outside the container, as PID 1's is.
    Exec,
}

/// Whether the sandbox has an SSH session or a working Herdr terminal.
pub async fn is_active(sessions: Sessions, exec: &dyn Exec) -> Result<bool> {
    let processes = processes::list(exec).await?;
    Ok(sessions.any(&processes, exec).await? || herdr::is_working(&processes, exec).await?)
}

impl Sessions {
    async fn any(self, processes: &[Process], exec: &dyn Exec) -> Result<bool> {
        match self {
            Self::Sshd => Ok(processes
                .iter()
                .any(|process| is_sshd_session(&process.args))),
            Self::Exec => {
                let mut forwards = Vec::new();
                for root in processes
                    .iter()
                    .filter(|process| process.ppid == OUTSIDE && process.pid != CONTAINER_INIT)
                {
                    // The ContainerSSH agent's port-forward process outlives the SSH connection
                    // that started it, so it counts only while it carries a forwarded connection.
                    if root.args.ends_with(FORWARD_SERVER) {
                        forwards.push(root.pid.to_string());
                    } else {
                        return Ok(true);
                    }
                }
                let forwards: Vec<&str> = forwards.iter().map(String::as_str).collect();
                forward::any_open(exec, &forwards).await
            }
        }
    }
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
            let exec = listed(&[INIT, (40, 7, session)]);
            assert!(is_active(Sessions::Sshd, &exec).await.unwrap(), "{session}");
        }
        for other in [
            "sshd: /opt/corral/sbin/sshd -D -e -f /opt/corral/etc/sshd_config [listener] 0 of 10-100 startups",
            "sshd-session: sandboxer@pts/0",
            "sshd-auth: sandbox [net]",
            "/bin/bash -c sshd: sandbox",
        ] {
            let exec = listed(&[INIT, (7, 1, other)]);
            assert!(!is_active(Sessions::Sshd, &exec).await.unwrap(), "{other}");
        }
    }

    #[tokio::test]
    async fn an_exec_root_other_than_init_and_the_probe_is_a_session() {
        let session = listed(&[INIT, (57, 0, "/bin/bash -l")]);
        let init_alone = listed(&[INIT, (7, 1, "/opt/corral/bin/busybox sleep 2147483647")]);

        assert!(is_active(Sessions::Exec, &session).await.unwrap());
        assert!(!is_active(Sessions::Exec, &init_alone).await.unwrap());
    }

    // Captured on kind from ContainerSSH 0.6.1 and agent v0.11.0: three SSH connections have
    // forwarded a port, and the last one carries an HTTP response through `ssh -L`.
    const FORWARD_ROOTS: [(u32, u32, &str); 4] = [
        INIT,
        (
            92,
            0,
            "/opt/containerssh-agent/usr/bin/containerssh-agent forward-server",
        ),
        (
            216,
            0,
            "/opt/containerssh-agent/usr/bin/containerssh-agent forward-server",
        ),
        (
            528,
            0,
            "/opt/containerssh-agent/usr/bin/containerssh-agent forward-server",
        ),
    ];
    const FORWARD_FDS: &str = "/opt/corral/bin/busybox ls -l /proc/92/fd /proc/216/fd /proc/528/fd";
    const TCP_TABLES: &str = "/opt/corral/bin/busybox cat /proc/net/tcp /proc/net/tcp6";
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
        listed(&FORWARD_ROOTS)
            .exits(FORWARD_FDS, fds_code, fds)
            .exits(TCP_TABLES, 0, tables)
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
        let exited = format!("ls: /proc/92/fd: No such file or directory\n{FDS_OPEN}");

        assert!(
            is_active(Sessions::Exec, &forwards(&exited, 1, TCP_OPEN))
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
        let mut roots = FORWARD_ROOTS.to_vec();
        roots.push((80, 0, "/bin/bash -l"));

        assert!(is_active(Sessions::Exec, &listed(&roots)).await.unwrap());
    }

    #[tokio::test]
    async fn a_working_herdr_terminal_without_a_session_is_activity() {
        let exec = listed(&[INIT, (35, 1, "/opt/corral/bin/herdr-core serve")]).exits(
            HERDR_LIST,
            0,
            r#"{"terminals":[{"state":"live","status":"working"}]}"#,
        );

        assert!(is_active(Sessions::Sshd, &exec).await.unwrap());
        assert!(is_active(Sessions::Exec, &exec).await.unwrap());
    }

    #[tokio::test]
    async fn a_failed_listing_is_an_error() {
        let failed = ScriptedExec::default().fails(LIST);
        let missing = ScriptedExec::default().exits(LIST, 127, "");

        assert!(is_active(Sessions::Sshd, &failed).await.is_err());
        let message = is_active(Sessions::Exec, &missing)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("code 127"), "{message}");
    }
}
