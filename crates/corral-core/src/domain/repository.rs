use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Error, Result};

const URL_MAX: usize = 1024;
const BRANCH_MAX: usize = 255;
const LOGIN: &str = "the repository URL carries a login; put the token in Setup, as a git token \
                     or SSH key, and leave it out of the URL";
const SHAPE: &str = "the repository must be an https://, ssh:// or git@host:path address";

/// A git repository a sandbox clones at its first start, and the branch to check out, else the
/// repository's default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "RepositoryInput")]
pub struct Repository {
    url: String,
    #[ts(optional = nullable)]
    branch: Option<String>,
}

#[derive(Deserialize)]
struct RepositoryInput {
    url: String,
    branch: Option<String>,
}

impl TryFrom<RepositoryInput> for Repository {
    type Error = Error;

    fn try_from(input: RepositoryInput) -> Result<Self> {
        Self::new(&input.url, input.branch.as_deref())
    }
}

impl Repository {
    /// Takes an `https://` URL without a login, an `ssh://` URL without a password, a `file:///`
    /// URL, or `user@host:path`.
    pub fn new(url: &str, branch: Option<&str>) -> Result<Self> {
        let url = url.trim();
        check_url(url)?;
        if let Some(branch) = branch {
            check_branch(branch)?;
        }
        Ok(Self {
            url: url.to_owned(),
            branch: branch.map(str::to_owned),
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// The directory in the home it is cloned into: the last segment of its path without
    /// `.git`, else `repo`.
    #[must_use]
    pub fn directory(&self) -> String {
        let path = match self.url.split_once("://") {
            Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
            None => self.url.split_once(':').map_or("", |(_, path)| path),
        };
        let last = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        match last.strip_suffix(".git").unwrap_or(last) {
            "" | "." | ".." => "repo".to_owned(),
            name => name.to_owned(),
        }
    }
}

fn check_url(url: &str) -> Result<()> {
    let shape = || Error::Invalid(SHAPE.to_owned());
    if url.len() > URL_MAX || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(shape());
    }
    // Git hands a host or a user to ssh as an argument, so neither may read as an option.
    let named = |part: &str| !part.is_empty() && !part.starts_with('-');
    match url.split_once("://") {
        Some((scheme @ ("https" | "ssh"), rest)) => {
            let authority = rest.split('/').next().unwrap_or_default();
            let (user, host) = match authority.rsplit_once('@') {
                Some((user, host)) => (Some(user), host),
                None => (None, authority),
            };
            if user.is_some_and(|user| scheme == "https" || user.contains(':')) {
                return Err(Error::Invalid(LOGIN.to_owned()));
            }
            let user_fine = user.is_none_or(|user| !user.starts_with('-'));
            (named(host) && user_fine).then_some(()).ok_or_else(shape)
        }
        Some(("file", path)) if path.len() > 1 && path.starts_with('/') => Ok(()),
        Some(_) => Err(shape()),
        None => {
            let (login, path) = url.split_once(':').ok_or_else(shape)?;
            let (user, host) = login.split_once('@').ok_or_else(shape)?;
            let scp_like = named(user)
                && named(host)
                && !login.contains('/')
                && !host.contains('@')
                && !path.is_empty();
            scp_like.then_some(()).ok_or_else(shape)
        }
    }
}

fn check_branch(branch: &str) -> Result<()> {
    let refused = |c: char| {
        c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    };
    let named = !branch.is_empty()
        && branch.len() <= BRANCH_MAX
        && !branch.starts_with('-')
        && !branch.contains("..")
        && !branch.chars().any(refused);
    if named {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "branch {branch:?} is not a git branch name"
        )))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Repository;
    use crate::Error;

    const LOGIN: &str = "the repository URL carries a login; put the token in Setup, as a git \
                         token or SSH key, and leave it out of the URL";
    const SHAPE: &str = "the repository must be an https://, ssh:// or git@host:path address";

    fn refusal(url: &str, branch: Option<&str>) -> String {
        match Repository::new(url, branch) {
            Err(Error::Invalid(why)) => why,
            other => panic!("{url:?} {branch:?}: {other:?}"),
        }
    }

    #[test]
    fn https_ssh_file_and_scp_like_addresses_are_repositories() {
        for url in [
            "https://github.com/adamaq01/corral.git",
            "https://git.example.com:8443/team/tools",
            "ssh://git@github.com/me/x.git",
            "ssh://git@host:2222/srv/x",
            "file:///srv/repo.git",
            "git@github.com:me/y",
            "me@host.lan:repos/x.git",
        ] {
            let repository = Repository::new(url, None).unwrap();
            assert_eq!((repository.url(), repository.branch()), (url, None));
        }
        let trimmed =
            Repository::new(" https://github.com/me/x.git\n", Some("feat/setup")).unwrap();
        assert_eq!(
            (trimmed.url(), trimmed.branch()),
            ("https://github.com/me/x.git", Some("feat/setup"))
        );
    }

    #[test]
    fn a_login_in_the_url_is_refused_saying_where_the_token_goes() {
        for url in [
            "https://me:tok3n@github.com/me/x.git",
            "https://:tok3n@github.com/me/x.git",
            "https://ghp_tok3n@github.com/me/x.git",
            "https://me@github.com/me/x.git",
            "https://@github.com/me/x.git",
            "ssh://git:tok3n@host/srv/x",
        ] {
            assert_eq!(refusal(url, None), LOGIN, "{url}");
        }
    }

    #[test]
    fn any_other_address_is_refused() {
        let long = format!("https://github.com/{}", "x".repeat(1024));
        for url in [
            "",
            "   ",
            "github.com/me/x",
            "http://github.com/me/x.git",
            "git://github.com/me/x",
            "ftp://host/x",
            "https://",
            "https:///me/x",
            "file://srv/x",
            "file:///",
            "https://github.com/me/my repo",
            "https://github.com/me/x\u{7}",
            "host:path",
            "@host:path",
            "git@:path",
            "git@host:",
            "a@b@host:path",
            "./git@host:path",
            "-oProxyCommand=x@host:path",
            "git@-oProxyCommand=x:path",
            "ssh://-oProxyCommand=x/y",
            "ssh://-oProxyCommand=x@host/y",
            &long,
        ] {
            assert_eq!(refusal(url, None), SHAPE, "{url:?}");
        }
    }

    #[test]
    fn a_branch_is_a_git_branch_name() {
        let url = "https://github.com/me/x.git";
        let longest = "b".repeat(255);
        for branch in ["main", "feat/setup", "release-1.2", "v1.0_rc", &longest] {
            let repository = Repository::new(url, Some(branch)).unwrap();
            assert_eq!(repository.branch(), Some(branch));
        }
        let long = "b".repeat(256);
        for branch in [
            "", "-x", "a..b", "a b", "a\tb", "a~1", "a^", "a:b", "a?", "a*", "a[b", "a\\b",
            "a\u{1}", &long,
        ] {
            assert_eq!(
                refusal(url, Some(branch)),
                format!("branch {branch:?} is not a git branch name")
            );
        }
    }

    #[test]
    fn the_directory_is_the_last_path_segment_without_git() {
        for (url, directory) in [
            ("https://github.com/me/x.git", "x"),
            ("git@github.com:me/y", "y"),
            ("git@github.com:z.git", "z"),
            ("file:///srv/repo.git", "repo"),
            ("ssh://git@host:2222/srv/tools", "tools"),
            ("https://github.com/me/tools/", "tools"),
            ("https://git.example.com", "repo"),
            ("https://git.example.com/.git", "repo"),
            ("https://git.example.com/me/..", "repo"),
        ] {
            let repository = Repository::new(url, None).unwrap();
            assert_eq!(repository.directory(), directory, "{url}");
        }
    }

    #[test]
    fn serde_reads_only_what_the_constructor_accepts() {
        let read: Repository =
            serde_json::from_str(r#"{"url": "git@github.com:me/y", "branch": "main"}"#).unwrap();
        assert_eq!(
            read,
            Repository::new("git@github.com:me/y", Some("main")).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            json!({"url": "git@github.com:me/y", "branch": "main"})
        );
        let unbranched: Repository =
            serde_json::from_str(r#"{"url": "git@github.com:me/y"}"#).unwrap();
        assert_eq!(
            serde_json::to_value(&unbranched).unwrap(),
            json!({"url": "git@github.com:me/y", "branch": null})
        );
        let error =
            serde_json::from_str::<Repository>(r#"{"url": "https://me:tok3n@github.com/x"}"#)
                .unwrap_err()
                .to_string();
        assert!(error.contains(LOGIN), "{error}");
        assert!(!error.contains("tok3n"), "{error}");
    }
}
