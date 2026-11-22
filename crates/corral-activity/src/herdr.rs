use serde::Deserialize;

use crate::{ActivityError, Exec, pgrep};

const COMMAND_NOT_FOUND: i32 = 127;

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

pub(crate) async fn is_working(exec: &dyn Exec) -> Result<bool, ActivityError> {
    // `hc ls` starts a core when none is running, so check for one first.
    if !pgrep::matches(exec, &["pgrep", "-x", "herdr-core"]).await? {
        return Ok(false);
    }
    let output = exec.run(&["hc", "ls", "--json"]).await?;
    match output.code {
        0 => {
            let listing: Listing = serde_json::from_str(&output.stdout)
                .map_err(|err| ActivityError::Parse(err.to_string()))?;
            Ok(listing.terminals.iter().any(Terminal::is_active))
        }
        COMMAND_NOT_FOUND => Ok(false),
        code => Err(ActivityError::UnexpectedExit {
            command: "hc ls --json".to_owned(),
            code,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ScriptedExec;

    const CORE_RUNNING: &str = "pgrep -x herdr-core";
    const LIST: &str = "hc ls --json";

    fn core_running() -> ScriptedExec {
        ScriptedExec::default().exits(CORE_RUNNING, 0, "35\n39\n")
    }

    async fn herdr_active_result(listing: &str) -> Result<bool, ActivityError> {
        let exec = core_running().exits(LIST, 0, listing);
        is_working(&exec).await
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

        assert!(matches!(result, Err(ActivityError::Parse(_))));
    }

    #[tokio::test]
    async fn no_herdr_core_is_inactive_without_running_hc() {
        let exec = ScriptedExec::default().exits(CORE_RUNNING, 1, "");

        let active = is_working(&exec).await;

        assert!(!active.unwrap());
        assert_eq!(exec.calls(), [CORE_RUNNING]);
    }

    #[tokio::test]
    async fn missing_hc_is_inactive_not_error() {
        let exec = core_running().exits(LIST, COMMAND_NOT_FOUND, "");

        let active = is_working(&exec).await;

        assert!(!active.unwrap());
    }

    #[tokio::test]
    async fn failing_hc_is_an_error() {
        let exec = core_running().exits(LIST, 1, "");

        let active = is_working(&exec).await;

        assert!(matches!(
            active,
            Err(ActivityError::UnexpectedExit { code: 1, .. })
        ));
    }

    #[tokio::test]
    async fn missing_pgrep_is_an_error() {
        let exec = ScriptedExec::default().exits(CORE_RUNNING, COMMAND_NOT_FOUND, "");

        let active = is_working(&exec).await;

        assert!(matches!(
            active,
            Err(ActivityError::UnexpectedExit { code: 127, .. })
        ));
    }

    #[tokio::test]
    async fn failed_exec_is_an_error() {
        let exec = core_running().fails(LIST);

        let active = is_working(&exec).await;

        assert!(matches!(active, Err(ActivityError::Exec(_))));
    }
}
