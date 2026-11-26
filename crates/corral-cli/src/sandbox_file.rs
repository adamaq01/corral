use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where Corral writes the file at every start of the sandbox, relative to the home.
const PATH: &str = ".config/corral/sandbox.json";

/// What Corral wrote into the home at the sandbox's last start.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct SandboxFile {
    /// The base URL of Corral's API, as the sandbox reaches it.
    pub api: String,
    /// The base URL of Corral's web UI.
    pub web: String,
    pub sandbox: String,
    pub token: String,
    /// What the sandbox's first start clones into the home, if its owner named a repository.
    pub repository: Option<Repository>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Repository {
    pub url: String,
    /// The branch to check out, else the repository's default.
    pub branch: Option<String>,
}

impl Repository {
    /// The directory in the home it is cloned into: the last segment of its path without
    /// `.git`, else `repo`, as Corral names it.
    pub fn directory(&self) -> &str {
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
            "" | "." | ".." => "repo",
            name => name,
        }
    }
}

impl SandboxFile {
    pub fn read(home: &Path) -> Result<Self, String> {
        let path: PathBuf = home.join(PATH);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "cannot read {}: {error}. Corral writes it at every start of the sandbox; \
                 restart the sandbox from the web UI if it is missing",
                path.display()
            )
        })?;
        serde_json::from_str(&text)
            .map_err(|error| format!("{} is not valid: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Repository, SandboxFile};

    fn home(test: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("corral-cli-{test}-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".config/corral")).unwrap();
        home
    }

    #[test]
    fn reads_what_corral_wrote() {
        let home = home("read");
        std::fs::write(
            home.join(".config/corral/sandbox.json"),
            r#"{"api": "http://corral:8080", "web": "https://corral.example.com",
                "sandbox": "main", "token": "corral_sbx_1",
                "repository": {"url": "git@github.com:me/x.git", "branch": null}}"#,
        )
        .unwrap();
        assert_eq!(
            SandboxFile::read(&home),
            Ok(SandboxFile {
                api: "http://corral:8080".to_owned(),
                web: "https://corral.example.com".to_owned(),
                sandbox: "main".to_owned(),
                token: "corral_sbx_1".to_owned(),
                repository: Some(Repository {
                    url: "git@github.com:me/x.git".to_owned(),
                    branch: None,
                }),
            })
        );
        std::fs::write(
            home.join(".config/corral/sandbox.json"),
            r#"{"api": "http://corral:8080", "web": "https://corral.example.com",
                "sandbox": "main", "token": "corral_sbx_1", "repository": null}"#,
        )
        .unwrap();
        assert_eq!(SandboxFile::read(&home).unwrap().repository, None);
        std::fs::write(home.join(".config/corral/sandbox.json"), "{}").unwrap();
        let invalid = SandboxFile::read(&home).unwrap_err();
        assert!(invalid.contains("is not valid"), "{invalid}");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_repository_is_cloned_into_its_last_path_segment_without_git() {
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
            let repository = Repository {
                url: url.to_owned(),
                branch: None,
            };
            assert_eq!(repository.directory(), directory, "{url}");
        }
    }

    #[test]
    fn a_missing_file_says_where_it_comes_from() {
        let missing = SandboxFile::read(Path::new("/nonexistent")).unwrap_err();
        assert!(
            missing.starts_with("cannot read /nonexistent/.config/corral/sandbox.json"),
            "{missing}"
        );
        assert!(missing.contains("restart the sandbox"), "{missing}");
    }
}
