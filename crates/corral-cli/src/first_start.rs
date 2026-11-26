//! `corral first-start dotfiles` and `corral first-start repository`, which `corral-init` runs at
//! the sandbox's first start: the first checks out the dotfiles the setup names into the home,
//! after the home files, which `corral-init` then writes again; the second clones the repository
//! the sandbox was made with into the home. Each runs the image's own `git` and adds how it went
//! to the notices. Nothing either meets keeps the sandbox from starting.

use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::notices;
use crate::sandbox_file::{Repository, SandboxFile};

pub const TIMEOUT: Duration = Duration::from_mins(5);
const POLL: Duration = Duration::from_millis(100);
const SSH: &str = "ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes";
const DOTFILES: &str = ".config/corral-agents/dotfiles.json";
const DOTFILES_GIT: &str = ".config/corral-agents/dotfiles.git";

/// A part of the first start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Dotfiles,
    Repository,
}

enum Failure {
    NoGit,
    /// Why, in a line.
    Failed(String),
}

/// Checks out the dotfiles of the setup in `home`, or clones the repository of its sandbox file,
/// if there is one, with the first `git` on `path`, stopped after `timeout`.
pub fn run(part: Part, home: &Path, path: &OsStr, timeout: Duration) {
    let said = match part {
        Part::Dotfiles => dotfiles(home, path, timeout),
        Part::Repository => SandboxFile::read(home)
            .ok()
            .and_then(|file| file.repository)
            .map(|repository| clone(home, &repository, path, timeout)),
    };
    if let Some(said) = said {
        tell(home, &said);
    }
}

fn tell(home: &Path, said: &str) {
    let said = without_userinfo(said);
    if let Err(error) = notices::append(home, &said) {
        eprintln!("corral first-start: cannot add to the notices ({error}): {said}");
    }
}

/// What to tell the sandbox user about the dotfiles, if the setup names any.
fn dotfiles(home: &Path, path: &OsStr, timeout: Duration) -> Option<String> {
    let unreadable =
        |why: &dyn Display| format!("Could not read the dotfiles from ~/{DOTFILES}: {why}");
    let text = match std::fs::read_to_string(home.join(DOTFILES)) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => return Some(unreadable(&error)),
    };
    let dotfiles: Repository = match serde_json::from_str(&text) {
        Ok(dotfiles) => dotfiles,
        Err(error) => return Some(unreadable(&error)),
    };
    let url = &dotfiles.url;
    Some(match check_out(home, &dotfiles, path, timeout) {
        Ok(()) => format!("Dotfiles from {url} checked out into the home"),
        Err(why) => format!("Checking out dotfiles from {url} failed: {why}"),
    })
}

/// Clones `dotfiles` bare into the home, then checks it out with the home as its work tree,
/// replacing what is at each of its paths. A failure leaves no bare clone.
fn check_out(
    home: &Path,
    dotfiles: &Repository,
    path: &OsStr,
    timeout: Duration,
) -> Result<(), String> {
    dotfiles.check_url()?;
    let bare = home.join(DOTFILES_GIT);
    if bare.symlink_metadata().is_ok() {
        return Err(format!("~/{DOTFILES_GIT} already exists"));
    }
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(&bare);
    let mut work_tree = OsString::from("--work-tree=");
    work_tree.push(home);
    let mut checkout = Command::new("git");
    checkout
        .arg(git_dir)
        .arg(work_tree)
        .args(["checkout", "-f"]);
    let clone = clone_command(dotfiles, &["--bare"], &bare);
    run_git(home, clone, path, timeout)
        .and_then(|()| run_git(home, checkout, path, timeout))
        .map_err(|failure| {
            let _ = std::fs::remove_dir_all(&bare);
            match failure {
                Failure::NoGit => "this image has no git".to_owned(),
                Failure::Failed(why) => why,
            }
        })
}

/// What to tell the sandbox user about the clone.
fn clone(home: &Path, repository: &Repository, path: &OsStr, timeout: Duration) -> String {
    let url = &repository.url;
    let directory = repository.directory();
    let target = home.join(directory);
    if target.symlink_metadata().is_ok() {
        return format!("~/{directory} already exists, so {url} was not cloned");
    }
    let cloned = run_git(home, clone_command(repository, &[], &target), path, timeout);
    if cloned.is_err() {
        let _ = std::fs::remove_dir_all(&target);
    }
    match cloned {
        Ok(()) => format!("Cloned {url} into ~/{directory}"),
        Err(Failure::NoGit) => format!("Cloning {url} needs git, which this image does not have"),
        Err(Failure::Failed(why)) => format!("Cloning {url} failed: {why}"),
    }
}

/// `git clone` of `repository` into `target`, with `options` before the repository's own.
fn clone_command(repository: &Repository, options: &[&str], target: &Path) -> Command {
    let mut git = Command::new("git");
    git.arg("clone").args(options);
    if let Some(branch) = &repository.branch {
        git.args(["--branch", branch]);
    }
    git.arg("--").arg(&repository.url).arg(target);
    git
}

/// Runs `git` in `home` with the first `git` on `path`, never prompting, stopped with its
/// children after `timeout`.
fn run_git(home: &Path, mut git: Command, path: &OsStr, timeout: Duration) -> Result<(), Failure> {
    let failed = |error: std::io::Error| Failure::Failed(error.to_string());
    let mut output = output_file(home).map_err(failed)?;
    let spawned = git
        .current_dir(home)
        .env("PATH", path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", SSH)
        .stdin(std::process::Stdio::null())
        .stdout(output.try_clone().map_err(failed)?)
        .stderr(output.try_clone().map_err(failed)?)
        .process_group(0)
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
            // The whole group, since git's ssh or credential helper outlives git killed alone.
            let _ = kill_process_group(Pid::from_child(&child), Signal::KILL);
            let _ = child.wait();
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

    use rustix::io::Errno;
    use rustix::process::{Pid, test_kill_process};

    use super::{Part, run};
    use crate::notices;

    /// Clones as git would, except into a directory named `missing`, where it fails as git does
    /// for an unknown repository, `private`, where it fails as git does when ssh is refused,
    /// `slow`, where it never ends, and `stalled`, where it never ends waiting on a child of its
    /// own, as git waits on ssh, whose pid it writes to `grandchild` in the home. A clone records
    /// its arguments in `invocation`, in the clone's `.git` unless it is bare, and a checkout in
    /// `checkout` in its git directory, which fails as git does for an empty repository when the
    /// clone was of a URL ending in `unborn.git`.
    const FAKE_GIT: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
record() {
  printf '%s\n' "$@" "GIT_TERMINAL_PROMPT=$GIT_TERMINAL_PROMPT" "GIT_SSH_COMMAND=$GIT_SSH_COMMAND"
}
case "$1" in
  --git-dir=*)
    git_dir=${1#--git-dir=}
    if [ -e "$git_dir/unborn" ]; then
      echo "fatal: You are on a branch yet to be born" >&2
      exit 128
    fi
    record "$@" >"$git_dir/checkout"
    exit
    ;;
esac
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
  */stalled)
    mkdir -p "$target"
    sleep 300 &
    echo $! >"${target%/*}/grandchild"
    wait
    ;;
esac
case "$2" in
  --bare) dir=$target ;;
  *) dir=$target/.git ;;
esac
mkdir -p "$dir"
case "$url" in
  */unborn.git) : >"$target/unborn" ;;
esac
record "$@" >"$dir/invocation"
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

    /// Writes the setup's dotfiles file, `dotfiles`, into `home`.
    fn write_dotfiles(home: &Path, dotfiles: &str) {
        std::fs::create_dir_all(home.join(".config/corral-agents")).unwrap();
        std::fs::write(home.join(".config/corral-agents/dotfiles.json"), dotfiles).unwrap();
    }

    /// Runs the first start's two parts in `corral-init`'s order.
    fn started(home: &Path, path: &Path) -> Vec<String> {
        for part in [Part::Dotfiles, Part::Repository] {
            run(part, home, path.as_os_str(), Duration::from_secs(60));
        }
        notices::read(home)
    }

    #[test]
    fn each_part_runs_alone() {
        let home = home(
            "parts",
            r#"{"url": "file:///srv/repo.git", "branch": null}"#,
        );
        write_dotfiles(
            &home,
            r#"{"url": "file:///srv/dotfiles.git", "branch": null}"#,
        );
        run(
            Part::Dotfiles,
            &home,
            fake_git().as_os_str(),
            Duration::from_secs(60),
        );
        assert_eq!(
            notices::read(&home),
            ["Dotfiles from file:///srv/dotfiles.git checked out into the home"]
        );
        assert!(!home.join("repo").exists());

        run(
            Part::Repository,
            &home,
            fake_git().as_os_str(),
            Duration::from_secs(60),
        );
        assert_eq!(
            notices::read(&home),
            [
                "Dotfiles from file:///srv/dotfiles.git checked out into the home",
                "Cloned file:///srv/repo.git into ~/repo"
            ]
        );
        std::fs::remove_dir_all(&home).unwrap();
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
    fn dotfiles_are_cloned_bare_and_checked_out_into_the_home_before_the_repository() {
        let home = home(
            "dotfiles",
            r#"{"url": "file:///srv/repo.git", "branch": null}"#,
        );
        write_dotfiles(
            &home,
            r#"{"url": "https://github.com/me/dotfiles.git", "branch": "main"}"#,
        );
        assert_eq!(
            started(&home, fake_git()),
            [
                "Dotfiles from https://github.com/me/dotfiles.git checked out into the home",
                "Cloned file:///srv/repo.git into ~/repo"
            ]
        );
        let bare = home.join(".config/corral-agents/dotfiles.git");
        let environment = "GIT_TERMINAL_PROMPT=0\n\
                           GIT_SSH_COMMAND=ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes\n";
        assert_eq!(
            std::fs::read_to_string(bare.join("invocation")).unwrap(),
            format!(
                "clone\n--bare\n--branch\nmain\n--\nhttps://github.com/me/dotfiles.git\n{}\n\
                 {environment}",
                bare.display()
            )
        );
        assert_eq!(
            std::fs::read_to_string(bare.join("checkout")).unwrap(),
            format!(
                "--git-dir={}\n--work-tree={}\ncheckout\n-f\n{environment}",
                bare.display(),
                home.display()
            )
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_failed_checkout_says_gits_fatal_line_and_leaves_no_bare_clone() {
        let home = home("dotfiles-unborn", "null");
        write_dotfiles(
            &home,
            r#"{"url": "https://github.com/me/unborn.git", "branch": null}"#,
        );
        assert_eq!(
            started(&home, fake_git()),
            [
                "Checking out dotfiles from https://github.com/me/unborn.git failed: fatal: You \
                 are on a branch yet to be born"
            ]
        );
        assert!(!home.join(".config/corral-agents/dotfiles.git").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn dotfiles_git_may_not_reach_are_a_notice_and_run_no_git() {
        for (test, dotfiles, said) in [
            (
                "dotfiles-login",
                r#"{"url": "https://me:tok3n@github.com/me/dotfiles", "branch": null}"#,
                "Checking out dotfiles from https://github.com/me/dotfiles failed: the repository \
                 URL carries a login; put the token in Setup, as a git token or SSH key, and \
                 leave it out of the URL",
            ),
            (
                "dotfiles-shape",
                r#"{"url": "ext::sh -c id", "branch": null}"#,
                "Checking out dotfiles from ext::sh -c id failed: the repository must be an \
                 https://, ssh:// or git@host:path address",
            ),
            (
                "dotfiles-invalid",
                r#"{"branch": "main"}"#,
                "Could not read the dotfiles from ~/.config/corral-agents/dotfiles.json: missing \
                 field `url` at line 1 column 18",
            ),
        ] {
            let home = home(test, "null");
            write_dotfiles(&home, dotfiles);
            assert_eq!(started(&home, fake_git()), [said], "{test}");
            assert!(!home.join(".config/corral-agents/dotfiles.git").exists());
            std::fs::remove_dir_all(&home).unwrap();
        }
    }

    #[test]
    fn a_bare_clone_in_the_way_is_left_alone_and_named() {
        let home = home("dotfiles-exists", "null");
        write_dotfiles(
            &home,
            r#"{"url": "file:///srv/dotfiles.git", "branch": null}"#,
        );
        std::fs::create_dir(home.join(".config/corral-agents/dotfiles.git")).unwrap();
        std::fs::write(home.join(".config/corral-agents/dotfiles.git/mine"), "").unwrap();
        assert_eq!(
            started(&home, fake_git()),
            [
                "Checking out dotfiles from file:///srv/dotfiles.git failed: \
                 ~/.config/corral-agents/dotfiles.git already exists"
            ]
        );
        assert!(
            home.join(".config/corral-agents/dotfiles.git/mine")
                .exists()
        );
        assert!(
            !home
                .join(".config/corral-agents/dotfiles.git/invocation")
                .exists()
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn without_git_nothing_is_cloned_and_it_says_so() {
        let home = home(
            "no-git",
            r#"{"url": "git@github.com:me/x.git", "branch": null}"#,
        );
        write_dotfiles(
            &home,
            r#"{"url": "git@github.com:me/dotfiles", "branch": null}"#,
        );
        let empty = scratch("no-git-bin");
        assert_eq!(
            started(&home, &empty),
            [
                "Checking out dotfiles from git@github.com:me/dotfiles failed: this image has no \
                 git",
                "Cloning git@github.com:me/x.git needs git, which this image does not have"
            ]
        );
        assert!(!home.join("x").exists());
        assert!(!home.join(".config/corral-agents/dotfiles.git").exists());
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
        run(
            Part::Repository,
            &home,
            fake_git().as_os_str(),
            Duration::from_secs(1),
        );
        assert!(began.elapsed() < Duration::from_secs(10));
        assert_eq!(
            notices::read(&home),
            ["Cloning https://github.com/me/slow failed: git took too long and was stopped"]
        );
        assert!(!home.join("slow").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_timed_out_clone_takes_its_children_with_it() {
        let home = home(
            "stalled",
            r#"{"url": "git@github.com:me/stalled.git", "branch": null}"#,
        );
        run(
            Part::Repository,
            &home,
            fake_git().as_os_str(),
            Duration::from_secs(1),
        );
        assert_eq!(
            notices::read(&home),
            ["Cloning git@github.com:me/stalled.git failed: git took too long and was stopped"]
        );
        let grandchild = std::fs::read_to_string(home.join("grandchild")).unwrap();
        let grandchild = Pid::from_raw(grandchild.trim().parse().unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while test_kill_process(grandchild) != Err(Errno::SRCH) {
            assert!(Instant::now() < deadline, "git's child outlived the clone");
            std::thread::sleep(Duration::from_millis(50));
        }
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
