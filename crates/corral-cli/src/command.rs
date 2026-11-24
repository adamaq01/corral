pub const USAGE: &str = "\
corral acts on the Corral sandbox it runs in.

Usage:
  corral status                   Show this sandbox and its state
  corral restart                  Restart this sandbox, which ends this session
  corral stop                     Put this sandbox to sleep, which ends this session
  corral delete --confirm <name>  Put this sandbox to sleep now and delete it with its home
                                  after a grace period, if your profile allows it
  corral help                     Show this help";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Restart,
    Stop,
    /// With the name typed to confirm, if any.
    Delete(Option<String>),
    Help,
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["status"] => Ok(Command::Status),
        ["restart"] => Ok(Command::Restart),
        ["stop"] => Ok(Command::Stop),
        ["delete"] => Ok(Command::Delete(None)),
        ["delete", "--confirm", name] => Ok(Command::Delete(Some((*name).to_owned()))),
        ["delete", confirm] if confirm.starts_with("--confirm=") => Ok(Command::Delete(Some(
            confirm.trim_start_matches("--confirm=").to_owned(),
        ))),
        [] | ["help" | "--help" | "-h"] => Ok(Command::Help),
        _ => Err(format!("unknown arguments: {}", args.join(" "))),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    fn parsed(line: &str) -> Result<Command, String> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args)
    }

    #[test]
    fn each_command_and_its_arguments() {
        assert_eq!(parsed("status"), Ok(Command::Status));
        assert_eq!(parsed("restart"), Ok(Command::Restart));
        assert_eq!(parsed("stop"), Ok(Command::Stop));
        assert_eq!(parsed("delete"), Ok(Command::Delete(None)));
        for confirmed in ["delete --confirm main", "delete --confirm=main"] {
            assert_eq!(
                parsed(confirmed),
                Ok(Command::Delete(Some("main".to_owned())))
            );
        }
        for help in ["", "help", "--help", "-h"] {
            assert_eq!(parsed(help), Ok(Command::Help));
        }
        for wrong in [
            "start",
            "stop now",
            "delete main",
            "delete --confirm",
            "status -v",
        ] {
            assert!(parsed(wrong).is_err(), "{wrong}");
        }
    }
}
