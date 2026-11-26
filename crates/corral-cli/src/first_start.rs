//! `corral first-start`, which `corral-init` runs at the sandbox's first start, after the home
//! files: clones the repository the sandbox was made with into the home, with the image's own
//! `git`, and adds how it went to the notices. Nothing it meets keeps the sandbox from starting.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek};
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use crate::notices;
use crate::sandbox_file::{Repository, SandboxFile};

pub const TIMEOUT: Duration = Duration::from_mins(5);
const POLL: Duration = Duration::from_millis(100);
const SSH: &str = "ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes";

enum Failure {
    NoGit,
    /// Why, in a line.
    Failed(String),
}

/// Clones the repository of the sandbox file in `home`, if it names one, with the first `git` on
/// `path`, stopped after `timeout`.
pub fn run(home: &Path, path: &OsStr, timeout: Duration) {
    let Some(repository) = SandboxFile::read(home)
        .ok()
        .and_then(|file| file.repository)
    else {
        return;
    };
    let said = without_userinfo(&clone(home, &repository, path, timeout));
    if let Err(error) = notices::append(home, &said) {
        eprintln!("corral first-start: cannot add to the notices ({error}): {said}");
    }
}

/// What to tell the sandbox user about the clone.
fn clone(home: &Path, repository: &Repository, path: &OsStr, timeout: Duration) -> String {
    let url = &repository.url;
    let directory = repository.directory();
    let target = home.join(directory);
    if target.symlink_metadata().is_ok() {
        return format!("~/{directory} already exists, so {url} was not cloned");
    }
    match git_clone(home, repository, &target, path, timeout) {
        Ok(()) => format!("Cloned {url} into ~/{directory}"),
        Err(Failure::NoGit) => format!("Cloning {url} needs git, which this image does not have"),
        Err(Failure::Failed(why)) => format!("Cloning {url} failed: {why}"),
    }
}

fn git_clone(
    home: &Path,
    repository: &Repository,
    target: &Path,
    path: &OsStr,
    timeout: Duration,
) -> Result<(), Failure> {
    let failed = |error: std::io::Error| Failure::Failed(error.to_string());
    let mut output = output_file(home).map_err(failed)?;
    let mut git = Command::new("git");
    git.arg("clone");
    if let Some(branch) = &repository.branch {
        git.args(["--branch", branch]);
    }
    let spawned = git
        .arg("--")
        .arg(&repository.url)
        .arg(target)
        .current_dir(home)
        .env("PATH", path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", SSH)
        .stdin(std::process::Stdio::null())
        .stdout(output.try_clone().map_err(failed)?)
        .stderr(output.try_clone().map_err(failed)?)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(Failure::NoGit),
        Err(error) => return Err(Failure::Failed(format!("cannot run git: {error}"))),
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(failed)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(target);
            return Err(Failure::Failed(
                "git took too long and was stopped".to_owned(),
            ));
        }
        std::thread::sleep(POLL);
    };
    if status.success() {
        return Ok(());
    }
    Err(Failure::Failed(why_git_failed(&mut output, status)))
}

/// A file for git's output with no name left in the home. It is no pipe because git's own
/// children, such as an ssh connection kept open for later, may hold it after git ends.
fn output_file(home: &Path) -> std::io::Result<File> {
    let path = home.join(format!(".corral-first-start-{}", std::process::id()));
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

/// Git's last `fatal:` line, which advice may follow, else its last line.
fn why_git_failed(output: &mut File, status: ExitStatus) -> String {
    let mut text = Vec::new();
    let _ = output.rewind().and_then(|()| output.read_to_end(&mut text));
    let text = String::from_utf8_lossy(&text);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        .iter()
        .rfind(|line| line.starts_with("fatal:"))
        .or(lines.last())
        .map_or_else(
            || format!("git ended with {status}"),
            |line| (*line).to_owned(),
        )
}

/// `text` with the user, and any password, taken out of each URL in it.
fn without_userinfo(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme) = rest.find("://") {
        let (before, after) = rest.split_at(scheme + 3);
        shown.push_str(before);
        let end = after
            .find(|c: char| c == '/' || c == '\'' || c == '"' || c.is_whitespace())
            .unwrap_or(after.len());
        let (authority, tail) = after.split_at(end);
        shown.push_str(
            authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host),
        );
        rest = tail;
    }
    shown.push_str(rest);
    shown
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use super::run;
    use crate::notices;

    /// Clones as git would, except into a directory named `missing`, where it fails as git does
    /// for an unknown repository, `private`, where it fails as git does when ssh is refused, and
    /// `slow`, where it never ends.
    const FAKE_GIT: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
for arg; do url=$target; target=$arg; done
case "$target" in
  */missing)
    echo "Cloning into '$target'..." >&2
    echo "fatal: repository '$url' not found" >&2
    echo >&2
    exit 128
    ;;
  */private)
    echo "Cloning into '$target'..." >&2
    echo "git@github.com: Permission denied (publickey)." >&2
    echo "fatal: Could not read from remote repository." >&2
    echo >&2
    echo "Please make sure you have the correct access rights" >&2
    echo "and the repository exists." >&2
    exit 128
    ;;
  */slow)
    mkdir -p "$target"
    exec sleep 30
    ;;
esac
mkdir -p "$target/.git"
printf '%s\n' "$@" "GIT_TERMINAL_PROMPT=$GIT_TERMINAL_PROMPT" "GIT_SSH_COMMAND=$GIT_SSH_COMMAND" \
  >"$target/.git/invocation"
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "corral-cli-first-start-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A PATH with the fake `git` alone, written once so no test runs it while it is open.
    fn fake_git() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            let dir = scratch("bin");
            let git = dir.join("git");
            std::fs::write(&git, FAKE_GIT).unwrap();
            std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
            dir
        })
    }

    /// A home whose sandbox file names `repository`, a JSON value.
    fn home(test: &str, repository: &str) -> PathBuf {
        let home = scratch(test);
        std::fs::create_dir_all(home.join(".config/corral")).unwrap();
        std::fs::write(
            home.join(".config/corral/sandbox.json"),
            format!(
                r#"{{"api": "http://corral:8080", "web": "https://corral.example.com",
                    "sandbox": "main", "token": "corral_sbx_1", "repository": {repository}}}"#
            ),
        )
        .unwrap();
        home
    }

    fn started(home: &Path, path: &Path) -> Vec<String> {
        run(home, path.as_os_str(), Duration::from_secs(60));
        notices::read(home)
    }

    #[test]
    fn a_clone_runs_git_clone_into_the_home_and_says_so() {
        let home = home(
            "cloned",
            r#"{"url": "https://github.com/me/x.git", "branch": "main"}"#,
        );
        assert_eq!(
            started(&home, fake_git()),
            ["Cloned https://github.com/me/x.git into ~/x"]
        );
        let target = home.join("x");
        assert_eq!(
            std::fs::read_to_string(target.join(".git/invocation")).unwrap(),
            format!(
                "clone\n--branch\nmain\n--\nhttps://github.com/me/x.git\n{}\n\
                 GIT_TERMINAL_PROMPT=0\n\
                 GIT_SSH_COMMAND=ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes\n",
                target.display()
            )
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_failed_clone_says_gits_fatal_line_without_the_urls_user() {
        let home = home(
            "failed",
            r#"{"url": "https://me@github.com/me/missing.git", "branch": null}"#,
        );
        assert_eq!(
            started(&home, fake_git()),
            [
                "Cloning https://github.com/me/missing.git failed: fatal: repository \
                 'https://github.com/me/missing.git' not found"
            ]
        );
        assert!(!home.join("missing").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_failed_clone_quotes_gits_fatal_line_over_its_last_one() {
        let home = home(
            "private",
            r#"{"url": "git@github.com:me/private.git", "branch": null}"#,
        );
        assert_eq!(
            started(&home, fake_git()),
            [
                "Cloning git@github.com:me/private.git failed: fatal: Could not read from remote \
                 repository."
            ]
        );
        assert!(!home.join("private").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn without_git_nothing_is_cloned_and_it_says_so() {
        let home = home(
            "no-git",
            r#"{"url": "git@github.com:me/x.git", "branch": null}"#,
        );
        let empty = scratch("no-git-bin");
        assert_eq!(
            started(&home, &empty),
            ["Cloning git@github.com:me/x.git needs git, which this image does not have"]
        );
        assert!(!home.join("x").exists());
        std::fs::remove_dir_all(&home).unwrap();
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn a_directory_in_the_way_is_left_alone_and_named() {
        let home = home(
            "exists",
            r#"{"url": "file:///srv/repo.git", "branch": null}"#,
        );
        std::fs::create_dir(home.join("repo")).unwrap();
        std::fs::write(
            home.join(".config/corral/notices"),
            "Corral could not write ~/x.\n",
        )
        .unwrap();
        assert_eq!(
            started(&home, fake_git()),
            [
                "Corral could not write ~/x.",
                "~/repo already exists, so file:///srv/repo.git was not cloned"
            ]
        );
        assert!(!home.join("repo/.git").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_clone_that_takes_too_long_is_stopped_and_leaves_nothing() {
        let home = home(
            "slow",
            r#"{"url": "https://github.com/me/slow", "branch": null}"#,
        );
        let began = Instant::now();
        run(&home, fake_git().as_os_str(), Duration::from_secs(1));
        assert!(began.elapsed() < Duration::from_secs(10));
        assert_eq!(
            notices::read(&home),
            ["Cloning https://github.com/me/slow failed: git took too long and was stopped"]
        );
        assert!(!home.join("slow").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn without_a_repository_there_is_nothing_to_say() {
        let home = home("none", "null");
        assert_eq!(started(&home, fake_git()), Vec::<String>::new());
        assert!(!home.join(".config/corral/notices").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
