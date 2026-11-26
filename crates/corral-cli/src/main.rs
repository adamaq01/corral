//! `corral`, in a sandbox: acts on that sandbox through Corral's API, with the token Corral wrote
//! into the home at the sandbox's start.

mod activity;
mod agents;
mod client;
mod command;
mod first_start;
mod notices;
mod sandbox_file;
mod session_env;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::client::{Client, Corral};
use crate::command::{Command, USAGE};
use crate::sandbox_file::{Repository, SandboxFile};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match command::parse(&args) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(command) => command,
        Err(message) => {
            eprintln!("corral: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    match command {
        Command::ReportActivity => activity::run(&home),
        Command::FirstStart => {
            let path = std::env::var_os("PATH").unwrap_or_default();
            first_start::run(&home, &path, first_start::TIMEOUT);
            return ExitCode::SUCCESS;
        }
        Command::AgentsApply => {
            for notice in agents::apply(&home, Path::new(session_env::PATH)) {
                if let Err(error) = notices::append(&home, &notice) {
                    eprintln!("corral agents-apply: cannot add to the notices ({error}): {notice}");
                }
            }
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let outcome = SandboxFile::read(&home)
        .and_then(|file| run(&command, &file, &Client::new(&file)))
        .map(|text| match command {
            Command::Status => with_notices(text, &notices::read(&home)),
            _ => text,
        });
    match outcome {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("corral: {message}");
            ExitCode::FAILURE
        }
    }
}

/// What to tell the person once the command is done, or why it failed.
fn run(command: &Command, file: &SandboxFile, corral: &impl Corral) -> Result<String, String> {
    let name = &file.sandbox;
    match command {
        Command::Status => {
            let reply = expect(corral.get("/api/sandbox")?, 200)?;
            Ok(status(&reply, file.repository.as_ref()))
        }
        Command::Restart => {
            expect(corral.post("/api/sandbox/restart", &json!({}))?, 202)?;
            Ok(format!(
                "Corral accepted the restart of {name}. This session ends as the sandbox stops; \
                 connect again in a moment."
            ))
        }
        Command::Stop => {
            expect(corral.post("/api/sandbox/stop", &json!({}))?, 202)?;
            Ok(format!(
                "Corral accepted the stop of {name}. This session ends as the sandbox goes to \
                 sleep; connecting again wakes it."
            ))
        }
        Command::KeepAwake(time) => {
            let for_secs = time.map(|time| time.as_secs());
            let reply = expect(
                corral.post("/api/sandbox/keep-awake", &json!({ "forSecs": for_secs }))?,
                200,
            )?;
            Ok(match reply["idle"]["keepAwakeUntil"].as_str() {
                Some(until) => format!("{name} stays awake until {until}, with or without work."),
                None => format!(
                    "{name} sleeps again after {} with no work.",
                    idle_timeout(&reply)
                ),
            })
        }
        Command::Delete(None) => Err(format!(
            "corral delete puts {name} to sleep now and deletes it, with everything in its home, \
             after a grace period. To go ahead, run: corral delete --confirm {name}"
        )),
        Command::Delete(Some(typed)) => {
            let reply = expect(
                corral.post("/api/sandbox/delete", &json!({ "confirm": typed }))?,
                202,
            )?;
            let at = reply["deleteAt"]
                .as_str()
                .unwrap_or("the end of the grace period");
            Ok(format!(
                "{name} goes to sleep now and is deleted at {at}. Until then, cancel the deletion \
                 on its page: {}/sandboxes/{name}",
                file.web.trim_end_matches('/')
            ))
        }
        Command::Help | Command::ReportActivity | Command::FirstStart | Command::AgentsApply => {
            Ok(USAGE.to_owned())
        }
    }
}

/// The sandbox's idle timeout, as `30 min` or `2 h`.
fn idle_timeout(sandbox: &Value) -> String {
    let Some(secs) = sandbox["idle"]["timeoutSecs"].as_u64() else {
        return "its idle timeout".to_owned();
    };
    let minutes = secs / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// The status, then what the last start has to tell, such as a home file it could not write.
fn with_notices(status: String, notices: &[String]) -> String {
    if notices.is_empty() {
        return status;
    }
    let listed: Vec<String> = notices.iter().map(|notice| format!("  {notice}")).collect();
    format!("{status}\n\nAt the last start:\n{}", listed.join("\n"))
}

fn expect(reply: client::Reply, status: u16) -> Result<Value, String> {
    if reply.status == status {
        Ok(reply.body)
    } else {
        Err(reply.error())
    }
}

fn status(sandbox: &Value, repository: Option<&Repository>) -> String {
    let field = |value: &Value| value.as_str().unwrap_or("?").to_owned();
    let line = |label: &str, value: String| format!("{label:<10}  {value}");
    let mut lines = vec![
        line("Sandbox", field(&sandbox["ssh"]["username"])),
        line("State", field(&sandbox["state"])),
    ];
    if let Some(repository) = repository {
        let branch = match &repository.branch {
            Some(branch) => format!(", branch {branch}"),
            None => String::new(),
        };
        lines.push(line("Repository", format!("{}{branch}", repository.url)));
    }
    lines.push(line("Image", field(&sandbox["image"])));
    lines.push(line("Size", field(&sandbox["size"])));
    lines.push(line(
        "Sleeps",
        format!("after {} with no work", idle_timeout(sandbox)),
    ));
    if let Some(until) = sandbox["idle"]["keepAwakeUntil"].as_str() {
        lines.push(line(
            "Awake",
            format!("until {until}, with or without work"),
        ));
    }
    if let Some(at) = sandbox["deleteAt"].as_str() {
        lines.push(line(
            "Deletes",
            format!("at {at}, unless cancelled in the web UI"),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::{Value, json};

    use super::{run, with_notices};
    use crate::client::{Corral, Reply};
    use crate::command::Command;
    use crate::sandbox_file::{Repository, SandboxFile};

    /// Answers every call with one reply and records the calls.
    struct Fake {
        status: u16,
        body: Value,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn answering(status: u16, body: Value) -> Self {
            Self {
                status,
                body,
                calls: RefCell::default(),
            }
        }

        fn reply(&self, call: String) -> Reply {
            self.calls.borrow_mut().push(call);
            Reply {
                status: self.status,
                body: self.body.clone(),
            }
        }
    }

    impl Corral for Fake {
        fn get(&self, path: &str) -> Result<Reply, String> {
            Ok(self.reply(format!("GET {path}")))
        }

        fn post(&self, path: &str, body: &Value) -> Result<Reply, String> {
            Ok(self.reply(format!("POST {path} {body}")))
        }
    }

    fn file() -> SandboxFile {
        SandboxFile {
            api: "http://corral:8080".to_owned(),
            web: "https://corral.example.com/".to_owned(),
            sandbox: "main".to_owned(),
            token: "corral_sbx_1".to_owned(),
            repository: None,
        }
    }

    fn ran(command: &Command, corral: &Fake) -> (Result<String, String>, Vec<String>) {
        ran_in(&file(), command, corral)
    }

    fn ran_in(
        file: &SandboxFile,
        command: &Command,
        corral: &Fake,
    ) -> (Result<String, String>, Vec<String>) {
        let outcome = run(command, file, corral);
        (outcome, corral.calls.borrow().clone())
    }

    #[test]
    fn restart_and_stop_say_the_session_ends() {
        for (command, path) in [
            (Command::Restart, "/api/sandbox/restart"),
            (Command::Stop, "/api/sandbox/stop"),
        ] {
            let (outcome, calls) = ran(&command, &Fake::answering(202, Value::Null));
            let said = outcome.unwrap();
            assert!(said.starts_with("Corral accepted the"), "{said}");
            assert!(said.contains("This session ends"), "{said}");
            assert_eq!(calls, [format!("POST {path} {{}}")]);
        }
    }

    #[test]
    fn status_shows_the_sandbox() {
        let sandbox = json!({
            "ssh": {"username": "main.adam"},
            "state": "running",
            "image": "ubuntu",
            "size": "small",
            "idle": {"timeoutSecs": 5400, "keepAwakeUntil": "2026-11-25T18:00:00Z"},
        });
        let (outcome, calls) = ran(&Command::Status, &Fake::answering(200, sandbox));
        assert_eq!(
            outcome.unwrap(),
            "Sandbox     main.adam\nState       running\nImage       ubuntu\nSize        small\n\
             Sleeps      after 1 h 30 min with no work\n\
             Awake       until 2026-11-25T18:00:00Z, with or without work"
        );
        assert_eq!(calls, ["GET /api/sandbox"]);
    }

    #[test]
    fn status_names_the_repository_after_the_state() {
        let sandbox = json!({
            "ssh": {"username": "main.adam"},
            "state": "running",
            "image": "ubuntu",
            "size": "small",
            "idle": {"timeoutSecs": 1800},
        });
        for (branch, line) in [
            (
                Some("main"),
                "Repository  https://github.com/adamaq01/corral.git, branch main",
            ),
            (None, "Repository  https://github.com/adamaq01/corral.git"),
        ] {
            let cloning = SandboxFile {
                repository: Some(Repository {
                    url: "https://github.com/adamaq01/corral.git".to_owned(),
                    branch: branch.map(str::to_owned),
                }),
                ..file()
            };
            let corral = Fake::answering(200, sandbox.clone());
            let (outcome, _) = ran_in(&cloning, &Command::Status, &corral);
            assert_eq!(
                outcome.unwrap(),
                format!(
                    "Sandbox     main.adam\nState       running\n{line}\nImage       ubuntu\n\
                     Size        small\nSleeps      after 30 min with no work"
                )
            );
        }
    }

    #[test]
    fn keep_awake_says_until_when_or_after_how_long_it_sleeps_again() {
        let kept = json!({"idle": {"timeoutSecs": 1800, "keepAwakeUntil": "2026-11-25T18:00:00Z"}});
        let four_hours = Command::KeepAwake(Some(std::time::Duration::from_hours(4)));
        let (outcome, calls) = ran(&four_hours, &Fake::answering(200, kept));
        assert_eq!(
            outcome.unwrap(),
            "main stays awake until 2026-11-25T18:00:00Z, with or without work."
        );
        assert_eq!(calls, [r#"POST /api/sandbox/keep-awake {"forSecs":14400}"#]);

        let off = json!({"idle": {"timeoutSecs": 1800, "keepAwakeUntil": null}});
        let (outcome, calls) = ran(&Command::KeepAwake(None), &Fake::answering(200, off));
        assert_eq!(
            outcome.unwrap(),
            "main sleeps again after 30 min with no work."
        );
        assert_eq!(calls, [r#"POST /api/sandbox/keep-awake {"forSecs":null}"#]);
    }

    #[test]
    fn status_ends_with_the_notices_of_the_last_start() {
        assert_eq!(
            with_notices("State running".to_owned(), &[]),
            "State running"
        );
        assert_eq!(
            with_notices(
                "State running".to_owned(),
                &["Corral could not write ~/x.".to_owned()]
            ),
            "State running\n\nAt the last start:\n  Corral could not write ~/x."
        );
    }

    #[test]
    fn delete_asks_for_the_name_before_calling_corral() {
        let (outcome, calls) = ran(&Command::Delete(None), &Fake::answering(202, Value::Null));
        let refused = outcome.unwrap_err();
        assert!(
            refused.ends_with("run: corral delete --confirm main"),
            "{refused}"
        );
        assert_eq!(calls, [] as [String; 0]);

        let scheduled = Fake::answering(202, json!({"deleteAt": "2026-11-23T12:10:00Z"}));
        let (outcome, calls) = ran(&Command::Delete(Some("main".to_owned())), &scheduled);
        assert_eq!(
            outcome.unwrap(),
            "main goes to sleep now and is deleted at 2026-11-23T12:10:00Z. Until then, cancel \
             the deletion on its page: https://corral.example.com/sandboxes/main"
        );
        assert_eq!(calls, [r#"POST /api/sandbox/delete {"confirm":"main"}"#]);
    }

    #[test]
    fn refusals_carry_corrals_explanation() {
        let off = Fake::answering(403, json!({"error": "deleting from inside is off"}));
        let (outcome, _) = ran(&Command::Delete(Some("main".to_owned())), &off);
        assert_eq!(outcome, Err("deleting from inside is off".to_owned()));
        let (outcome, _) = ran(&Command::Stop, &Fake::answering(502, Value::Null));
        assert_eq!(outcome, Err("Corral answered 502".to_owned()));
    }
}
