use std::time::Duration;

pub const USAGE: &str = "\
corral acts on the Corral sandbox it runs in.

Usage:
  corral status                   Show this sandbox and its state
  corral restart                  Restart this sandbox, which ends this session
  corral stop                     Put this sandbox to sleep, which ends this session
  corral keep-awake <time>        Keep this sandbox awake for <time>, such as 30m, 4h or 2d,
                                  even with no work in it
  corral keep-awake off           Let this sandbox sleep once idle again
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
    /// For so long, or with `None` no longer.
    KeepAwake(Option<Duration>),
    /// Run by the sandbox itself at its start.
    ReportActivity,
    /// Run by `corral-init` at the sandbox's first start.
    FirstStart,
    Help,
}

/// A time such as `90m`, `4h` or `2d`.
fn duration(text: &str) -> Result<Duration, String> {
    let wrong = || format!("{text:?} is not a time such as 30m, 4h or 2d");
    let split = text.len().saturating_sub(1);
    let (count, unit) = (text.get(..split).ok_or_else(wrong)?, text.get(split..));
    let count: u64 = count.parse().map_err(|_| wrong())?;
    let unit = match unit {
        Some("m") => 60,
        Some("h") => 3600,
        Some("d") => 86400,
        _ => return Err(wrong()),
    };
    match count.checked_mul(unit) {
        Some(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
        _ => Err(wrong()),
    }
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["status"] => Ok(Command::Status),
        ["restart"] => Ok(Command::Restart),
        ["stop"] => Ok(Command::Stop),
        ["keep-awake", "off"] => Ok(Command::KeepAwake(None)),
        ["keep-awake", time] => duration(time).map(|time| Command::KeepAwake(Some(time))),
        ["report-activity"] => Ok(Command::ReportActivity),
        ["first-start"] => Ok(Command::FirstStart),
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
    use std::time::Duration;

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
        assert_eq!(
            parsed("keep-awake 4h"),
            Ok(Command::KeepAwake(Some(Duration::from_hours(4))))
        );
        assert_eq!(
            parsed("keep-awake 90m"),
            Ok(Command::KeepAwake(Some(Duration::from_mins(90))))
        );
        assert_eq!(
            parsed("keep-awake 2d"),
            Ok(Command::KeepAwake(Some(Duration::from_hours(48))))
        );
        assert_eq!(parsed("keep-awake off"), Ok(Command::KeepAwake(None)));
        assert_eq!(parsed("report-activity"), Ok(Command::ReportActivity));
        assert_eq!(parsed("first-start"), Ok(Command::FirstStart));
        for wrong in [
            "keep-awake",
            "keep-awake 4",
            "keep-awake 0h",
            "keep-awake h",
            "keep-awake 4w",
            "keep-awake -1h",
            "keep-awake 4é",
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
