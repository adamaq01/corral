use corral_core::{Error, Result};
use serde::Deserialize;

use crate::Exec;
use crate::exec::unexpected_exit;
use crate::processes::Process;

const LIST: [&str; 3] = ["/opt/corral/bin/hc", "ls", "--json"];

#[derive(Deserialize)]
struct Listing {
    terminals: Vec<Terminal>,
}

#[derive(Deserialize)]
struct Terminal {
    state: String,
    status: Option<String>,
}

impl Terminal {
    fn is_active(&self) -> bool {
        self.state == "live" && matches!(self.status.as_deref(), None | Some("working" | "blocked"))
    }
}

pub(crate) async fn is_working(processes: &[Process], exec: &dyn Exec) -> Result<bool> {
    // `hc ls` starts a core when none is running, so it runs only beside one.
    if !processes
        .iter()
        .any(|process| process.program() == "herdr-core")
    {
        return Ok(false);
    }
    let output = exec.run(&LIST).await?;
    if output.code != 0 {
        return Err(unexpected_exit(&LIST, output.code));
    }
    let listing: Listing = serde_json::from_str(&output.stdout)
        .map_err(|err| Error::internal("unreadable `hc ls --json` output", err))?;
    Ok(listing.terminals.iter().any(Terminal::is_active))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ScriptedExec;

    const LIST: &str = "/opt/corral/bin/hc ls --json";

    fn core() -> Vec<Process> {
        vec![Process {
            pid: 35,
            ppid: 1,
            args: "/opt/corral/bin/herdr-core serve".to_owned(),
        }]
    }

    async fn herdr_active_result(listing: &str) -> Result<bool> {
        let exec = ScriptedExec::default().exits(LIST, 0, listing);
        is_working(&core(), &exec).await
    }

    async fn herdr_active(listing: &str) -> bool {
        herdr_active_result(listing).await.unwrap()
    }

    #[tokio::test]
    async fn herdr_status_parsing() {
        assert!(herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","state":"live","status":"working","turn":0}]}"#).await);
        assert!(herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","state":"live","status":"blocked","turn":0}]}"#).await);
        assert!(herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","agent":"sleep","cwd":"/home/sandbox","state":"live","turn":0}]}"#).await);
        assert!(herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","state":"live","status":null,"turn":0}]}"#).await);
        assert!(!herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","state":"live","status":"done","turn":0}]}"#).await);
        assert!(!herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","agent":"sleep","reason":"killed","signal":1,"state":"finished","turn":0}]}"#).await);
        assert!(!herdr_active(r#"{"hidden_finished":0,"terminals":[]}"#).await);
        assert!(!herdr_active(r#"{"hidden_finished":1,"terminals":[]}"#).await);
    }

    #[tokio::test]
    async fn one_active_terminal_among_idle_ones_is_active() {
        assert!(herdr_active(r#"{"hidden_finished":0,"terminals":[{"address":"t1","state":"live","status":"done","turn":1},{"address":"t2","state":"live","status":"working","turn":0}]}"#).await);
    }

    #[tokio::test]
    async fn unreadable_listing_is_an_error() {
        let result = herdr_active_result("hc: no systemd user manager here").await;

        let message = result.unwrap_err().to_string();
        assert!(message.contains("unreadable"), "{message}");
    }

    #[tokio::test]
    async fn no_herdr_core_is_inactive_without_running_hc() {
        let exec = ScriptedExec::default();
        let other = vec![Process {
            pid: 40,
            ppid: 1,
            args: "/usr/bin/herdr-core-helper serve".to_owned(),
        }];

        assert!(!is_working(&[], &exec).await.unwrap());
        assert!(!is_working(&other, &exec).await.unwrap());
        assert_eq!(exec.calls(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_failing_or_missing_hc_is_an_error() {
        for code in [1, 127] {
            let exec = ScriptedExec::default().exits(LIST, code, "");

            let message = is_working(&core(), &exec).await.unwrap_err().to_string();

            assert!(message.contains(&format!("code {code}")), "{message}");
        }
    }

    #[tokio::test]
    async fn failed_exec_is_an_error() {
        let exec = ScriptedExec::default().fails(LIST);

        assert!(is_working(&core(), &exec).await.is_err());
    }
}
