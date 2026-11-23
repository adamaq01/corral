//! A sandbox's processes, listed with the busybox of the Corral overlay that every sandbox mounts,
//! so a probe needs nothing from the sandbox's image.

use corral_core::{Error, Result};

use crate::Exec;
use crate::exec::unexpected_exit;

pub(crate) const BUSYBOX: &str = "/opt/corral/bin/busybox";
// The shell prints its PID, which `ps` keeps through `exec`, so the listing leaves itself out.
const LIST: [&str; 3] = [
    "/opt/corral/bin/sh",
    "-c",
    "echo $$; exec /opt/corral/bin/busybox ps -o pid,ppid,args",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub(crate) pid: u32,
    pub(crate) ppid: u32,
    /// The command line, its arguments joined by spaces.
    pub(crate) args: String,
}

impl Process {
    /// The last part of the path the process runs.
    pub(crate) fn program(&self) -> &str {
        let path = self.args.split(' ').next().unwrap_or_default();
        path.rsplit('/').next().unwrap_or(path)
    }
}

pub(crate) async fn list(exec: &dyn Exec) -> Result<Vec<Process>> {
    let output = exec.run(&LIST).await?;
    if output.code != 0 {
        return Err(unexpected_exit(&LIST, output.code));
    }
    parse(&output.stdout).ok_or_else(|| {
        Error::internal("unreadable process listing", format!("{:?}", output.stdout))
    })
}

fn parse(listing: &str) -> Option<Vec<Process>> {
    let mut lines = listing.lines();
    let own: u32 = lines.next()?.trim().parse().ok()?;
    if !lines.next()?.trim_start().starts_with("PID") {
        return None;
    }
    let mut processes = Vec::new();
    for line in lines {
        let mut fields = line.split_whitespace();
        let pid = fields.next()?.parse().ok()?;
        let parent = fields.next()?.parse().ok()?;
        let args = fields.collect::<Vec<_>>().join(" ");
        if pid != own {
            processes.push(Process {
                pid,
                ppid: parent,
                args,
            });
        }
    }
    Some(processes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testing::ScriptedExec;

    pub(crate) const LIST: &str =
        "/opt/corral/bin/sh -c echo $$; exec /opt/corral/bin/busybox ps -o pid,ppid,args";

    /// What the listing prints for these `(pid, ppid, args)`, the listing's own process included.
    pub(crate) fn listing(processes: &[(u32, u32, &str)]) -> String {
        let lines: Vec<String> = processes
            .iter()
            .map(|(pid, ppid, args)| format!("{pid:>5} {ppid:>5} {args}\n"))
            .collect();
        format!(
            "106\nPID   PPID  COMMAND\n{}  106     0 /opt/corral/bin/busybox ps -o pid,ppid,args\n",
            lines.concat()
        )
    }

    #[tokio::test]
    async fn the_listing_leaves_itself_out() {
        let exec = ScriptedExec::default().exits(
            LIST,
            0,
            &listing(&[
                (
                    1,
                    0,
                    "/opt/corral/bin/tini -- /opt/corral/bin/corral-init docker",
                ),
                (37, 1, "/opt/corral/bin/herdr-core  serve"),
            ]),
        );

        let processes = list(&exec).await.unwrap();

        assert_eq!(
            processes,
            [
                Process {
                    pid: 1,
                    ppid: 0,
                    args: "/opt/corral/bin/tini -- /opt/corral/bin/corral-init docker".to_owned(),
                },
                Process {
                    pid: 37,
                    ppid: 1,
                    args: "/opt/corral/bin/herdr-core serve".to_owned(),
                },
            ]
        );
        assert_eq!(processes[1].program(), "herdr-core");
        assert_eq!(processes[0].program(), "tini");
    }

    #[tokio::test]
    async fn an_unreadable_or_failed_listing_is_an_error() {
        for (code, stdout) in [
            (0, "not a pid\n"),
            (0, "106\nno header\n"),
            (0, "106\nPID PPID COMMAND\n  x 0 sh\n"),
            (127, ""),
        ] {
            let exec = ScriptedExec::default().exits(LIST, code, stdout);
            assert!(list(&exec).await.is_err(), "{code} {stdout:?}");
        }
    }
}
