use corral_core::ports::home_staging::{self, Staged};
use corral_core::ports::{Boot, session_env};
use secrecy::ExposeSecret;

const SANDBOX_USER: u64 = 1000;

/// A tar archive, to extract at `/`, of what the sandbox reads when it starts, each file readable
/// by the sandbox user alone: the variables its sessions get, and the home files, staged for
/// `corral-init`.
pub(crate) fn archive(boot: &Boot) -> Vec<u8> {
    let mut archive = Archive(tar::Builder::new(Vec::new()));
    archive.file(
        session_env::PATH.trim_start_matches('/'),
        0o400,
        session_env::sshd_config(&boot.env).as_bytes(),
    );
    let staging = home_staging::DIR.trim_start_matches('/');
    archive.directory(staging, 0o700);
    let Staged { manifest, files } = home_staging::stage(&boot.files);
    let staged = |key: &str| format!("{staging}/{key}");
    archive.file(&staged(home_staging::MANIFEST), 0o600, manifest.as_bytes());
    for (key, file) in files {
        archive.file(&staged(&key), 0o600, file.content.expose_secret());
    }
    archive
        .0
        .into_inner()
        .expect("an archive in memory always writes")
}

struct Archive(tar::Builder<Vec<u8>>);

impl Archive {
    fn file(&mut self, path: &str, mode: u32, contents: &[u8]) {
        let mut header = header(tar::EntryType::Regular, mode);
        header.set_size(contents.len() as u64);
        self.0
            .append_data(&mut header, path, contents)
            .expect("an archive in memory always writes");
    }

    fn directory(&mut self, path: &str, mode: u32) {
        let mut header = header(tar::EntryType::Directory, mode);
        header.set_size(0);
        self.0
            .append_data(&mut header, path, std::io::empty())
            .expect("an archive in memory always writes");
    }
}

fn header(kind: tar::EntryType, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_uid(SANDBOX_USER);
    header.set_gid(SANDBOX_USER);
    header
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use corral_core::domain::{FileMode, HomeFile, HomeFiles, HomePath, VariableName};
    use corral_core::ports::{Boot, VariableMap};
    use secrecy::{SecretSlice, SecretString};

    use super::archive;

    fn env(pairs: &[(&str, &str)]) -> VariableMap {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    VariableName::parse(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect()
    }

    type Entry = (String, u32, (u64, u64), String);

    fn entries_of(boot: &Boot) -> Vec<Entry> {
        let bytes = archive(boot);
        let mut archive = tar::Archive::new(bytes.as_slice());
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let header = entry.header().clone();
                let mut contents = String::new();
                entry.read_to_string(&mut contents).unwrap();
                (
                    header.path().unwrap().display().to_string(),
                    header.mode().unwrap(),
                    (header.uid().unwrap(), header.gid().unwrap()),
                    contents,
                )
            })
            .collect()
    }

    #[test]
    fn archive_holds_the_session_variables_for_the_sandbox_user_alone() {
        let entries = entries_of(&Boot {
            env: env(&[("GH_TOKEN", "gh"), ("ANTHROPIC_API_KEY", "sk")]),
            files: HomeFiles::new(),
        });

        assert_eq!(
            entries[0],
            (
                "etc/corral/sshd_env.conf".to_owned(),
                0o400,
                (1000, 1000),
                "SetEnv \"ANTHROPIC_API_KEY=sk\" \"GH_TOKEN=gh\"\n".to_owned()
            )
        );
        assert_eq!(entries_of(&Boot::default())[0].3, "");
    }

    #[test]
    fn archive_stages_home_files_for_the_sandbox_user_alone() {
        let file = |mode, content: &str| HomeFile {
            mode: FileMode::new(mode).unwrap(),
            content: SecretSlice::from(content.as_bytes().to_vec()),
        };
        let boot = Boot {
            env: VariableMap::new(),
            files: HomeFiles::from([
                (HomePath::parse(".config/a").unwrap(), file(0o600, "first")),
                (HomePath::parse("b").unwrap(), file(0o755, "second")),
            ]),
        };
        let owned = |path: &str, mode, contents: &str| {
            (path.to_owned(), mode, (1000, 1000), contents.to_owned())
        };

        assert_eq!(
            entries_of(&boot)[1..],
            [
                owned("etc/corral/home-files", 0o700, ""),
                owned(
                    "etc/corral/home-files/manifest",
                    0o600,
                    "0 0600 .config/a\n1 0755 b\n"
                ),
                owned("etc/corral/home-files/0", 0o600, "first"),
                owned("etc/corral/home-files/1", 0o600, "second"),
            ]
        );
    }
}
