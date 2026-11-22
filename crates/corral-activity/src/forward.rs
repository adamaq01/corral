use std::collections::HashSet;

use corral_core::Result;

use crate::{Exec, pgrep};

const ESTABLISHED: &str = "01";

/// Whether any of these port-forward agents carries an open TCP connection.
pub(crate) async fn any_open(exec: &dyn Exec, pids: &[&str]) -> Result<bool> {
    if pids.is_empty() {
        return Ok(false);
    }
    let fds: Vec<String> = pids.iter().map(|pid| format!("/proc/{pid}/fd")).collect();
    let mut argv = vec!["ls", "-l"];
    argv.extend(fds.iter().map(String::as_str));
    let listing = list(exec, &argv).await?;
    let held = sockets(&listing);
    if held.is_empty() {
        return Ok(false);
    }
    let tables = list(exec, &["cat", "/proc/net/tcp", "/proc/net/tcp6"]).await?;
    Ok(established(&tables).any(|inode| held.contains(inode)))
}

// An agent can exit between the process listing and this one, and a kernel without IPv6 has no
// tcp6 table, so a listing that fails part way still holds what it could read.
async fn list(exec: &dyn Exec, argv: &[&str]) -> Result<String> {
    let output = exec.run(argv).await?;
    match output.code {
        0..=2 => Ok(output.stdout),
        code => Err(pgrep::unexpected_exit(argv, code)),
    }
}

fn sockets(listing: &str) -> HashSet<&str> {
    listing
        .lines()
        .filter_map(|line| line.split_once("-> socket:[")?.1.split_once(']'))
        .map(|(inode, _)| inode)
        .collect()
}

fn established(tables: &str) -> impl Iterator<Item = &str> {
    tables.lines().filter_map(
        |line| match line.split_whitespace().collect::<Vec<_>>()[..] {
            [_, _, _, ESTABLISHED, _, _, _, _, _, inode, ..] => Some(inode),
            _ => None,
        },
    )
}
