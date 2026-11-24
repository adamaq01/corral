//! `corral`, in a sandbox: acts on that sandbox through Corral's API, with the token Corral wrote
//! into the home at the sandbox's start.

mod client;
mod command;
mod sandbox_file;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::client::{Client, Corral};
use crate::command::{Command, USAGE};
use crate::sandbox_file::SandboxFile;

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
    let outcome =
        SandboxFile::read(&home).and_then(|file| run(&command, &file, &Client::new(&file)));
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
            Ok(status(&reply))
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
        Command::Help => Ok(USAGE.to_owned()),
    }
}

fn expect(reply: client::Reply, status: u16) -> Result<Value, String> {
    if reply.status == status {
        Ok(reply.body)
    } else {
        Err(reply.error())
    }
}

fn status(sandbox: &Value) -> String {
    let field = |value: &Value| value.as_str().unwrap_or("?").to_owned();
    let mut lines = vec![
        format!("Sandbox  {}", field(&sandbox["ssh"]["username"])),
        format!("State    {}", field(&sandbox["state"])),
        format!("Image    {}", field(&sandbox["image"])),
        format!("Size     {}", field(&sandbox["size"])),
    ];
    if let Some(at) = sandbox["deleteAt"].as_str() {
        lines.push(format!("Deletes  at {at}, unless cancelled in the web UI"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::{Value, json};

    use super::run;
    use crate::client::{Corral, Reply};
    use crate::command::Command;
    use crate::sandbox_file::SandboxFile;

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
        }
    }

    fn ran(command: &Command, corral: &Fake) -> (Result<String, String>, Vec<String>) {
        let outcome = run(command, &file(), corral);
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
        });
        let (outcome, calls) = ran(&Command::Status, &Fake::answering(200, sandbox));
        assert_eq!(
            outcome.unwrap(),
            "Sandbox  main.adam\nState    running\nImage    ubuntu\nSize     small"
        );
        assert_eq!(calls, ["GET /api/sandbox"]);
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
