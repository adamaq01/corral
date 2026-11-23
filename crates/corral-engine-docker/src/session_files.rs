use corral_core::ports::home_staging::{self, Staged};
use corral_core::ports::{Boot, CredentialMap};
use secrecy::ExposeSecret;
use ssh_key::PublicKey;
use tracing::warn;

const AUTHORIZED_KEYS: &str = "etc/ssh/corral_authorized_keys";
const SSHD_ENV: &str = "etc/corral/sshd_env.conf";
const ROOT: u64 = 0;
const SANDBOX_USER: u64 = 1000;

/// A tar archive, to extract at `/`, of what the sandbox reads when it starts: the key the front
/// door logs in with and the credentials its sessions see, both for sshd, and the home files,
/// staged for `corral-home-init` where only the sandbox user may read them.
pub(crate) fn archive(proxy_key: &PublicKey, boot: &Boot) -> Vec<u8> {
    let authorized_key = format!(
        "{}\n",
        proxy_key.to_openssh().expect("a public key always encodes")
    );
    let mut archive = Archive(tar::Builder::new(Vec::new()));
    archive.file(AUTHORIZED_KEYS, 0o644, ROOT, authorized_key.as_bytes());
    archive.file(SSHD_ENV, 0o600, ROOT, set_env(&boot.env).as_bytes());
    let staging = home_staging::DIR.trim_start_matches('/');
    archive.directory(staging, 0o700, SANDBOX_USER);
    let Staged { manifest, files } = home_staging::stage(&boot.files);
    let staged = |key: &str| format!("{staging}/{key}");
    archive.file(
        &staged(home_staging::MANIFEST),
        0o600,
        SANDBOX_USER,
        manifest.as_bytes(),
    );
    for (key, file) in files {
        archive.file(
            &staged(&key),
            0o600,
            SANDBOX_USER,
            file.content.expose_secret(),
        );
    }
    archive
        .0
        .into_inner()
        .expect("an archive in memory always writes")
}

struct Archive(tar::Builder<Vec<u8>>);

impl Archive {
    fn file(&mut self, path: &str, mode: u32, owner: u64, contents: &[u8]) {
        let mut header = header(tar::EntryType::Regular, mode, owner);
        header.set_size(contents.len() as u64);
        self.0
            .append_data(&mut header, path, contents)
            .expect("an archive in memory always writes");
    }

    fn directory(&mut self, path: &str, mode: u32, owner: u64) {
        let mut header = header(tar::EntryType::Directory, mode, owner);
        header.set_size(0);
        self.0
            .append_data(&mut header, path, std::io::empty())
            .expect("an archive in memory always writes");
    }
}

fn header(kind: tar::EntryType, mode: u32, owner: u64) -> tar::Header {
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_uid(owner);
    header.set_gid(owner);
    header
}

fn set_env(env: &CredentialMap) -> String {
    let quoted: Vec<String> = env
        .iter()
        .filter_map(|(name, value)| {
            let value = value.expose_secret();
            // A line break would end the SetEnv line and let the value write sshd directives.
            if value.contains(['\n', '\r', '\0']) {
                warn!(
                    credential = name.as_str(),
                    "left out a credential that is not one line"
                );
                return None;
            }
            let value = value.replace('\\', "\\\\").replace('"', "\\\"");
            Some(format!("\"{}={value}\"", name.as_str()))
        })
        .collect();
    if quoted.is_empty() {
        String::new()
    } else {
        format!("SetEnv {}\n", quoted.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use corral_core::domain::{CredentialName, FileMode, HomeFile, HomeFiles, HomePath};
    use corral_core::ports::{Boot, CredentialMap};
    use secrecy::{SecretSlice, SecretString};
    use ssh_key::PublicKey;

    use super::archive;

    const PROXY_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIqeCTDFpVlRmE6nzi3s6Ui2G8N7lHGk06q4B8yZ3sAi";

    fn env(pairs: &[(&str, &str)]) -> CredentialMap {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    CredentialName::parse(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect()
    }

    type Entry = (String, u32, (u64, u64), String);

    fn entries_of(boot: &Boot) -> Vec<Entry> {
        let bytes = archive(&PublicKey::from_openssh(PROXY_KEY).unwrap(), boot);
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

    fn entries(env: &CredentialMap) -> Vec<Entry> {
        entries_of(&Boot {
            env: env.clone(),
            files: HomeFiles::new(),
        })
    }

    #[test]
    fn archive_holds_root_owned_sshd_files() {
        let entries = entries(&env(&[("GH_TOKEN", "gh"), ("ANTHROPIC_API_KEY", "sk")]));

        assert_eq!(
            entries[..2],
            [
                (
                    "etc/ssh/corral_authorized_keys".to_owned(),
                    0o644,
                    (0, 0),
                    format!("{PROXY_KEY}\n")
                ),
                (
                    "etc/corral/sshd_env.conf".to_owned(),
                    0o600,
                    (0, 0),
                    "SetEnv \"ANTHROPIC_API_KEY=sk\" \"GH_TOKEN=gh\"\n".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn archive_stages_home_files_for_the_sandbox_user_alone() {
        let file = |mode, content: &str| HomeFile {
            mode: FileMode::new(mode).unwrap(),
            content: SecretSlice::from(content.as_bytes().to_vec()),
        };
        let boot = Boot {
            env: CredentialMap::new(),
            files: HomeFiles::from([
                (HomePath::parse(".config/a").unwrap(), file(0o600, "first")),
                (HomePath::parse("b").unwrap(), file(0o755, "second")),
            ]),
        };
        let owned = |path: &str, mode, contents: &str| {
            (path.to_owned(), mode, (1000, 1000), contents.to_owned())
        };

        assert_eq!(
            entries_of(&boot)[2..],
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

    #[test]
    fn values_are_quoted() {
        let odd = entries(&env(&[("ODD_TOKEN", r#"p#ss "word" it's $HOME \x\"#)]));

        assert_eq!(
            odd[1].3,
            "SetEnv \"ODD_TOKEN=p#ss \\\"word\\\" it's $HOME \\\\x\\\\\"\n"
        );
        assert_eq!(entries(&CredentialMap::new())[1].3, "");
    }

    #[test]
    fn values_that_are_not_one_line_never_reach_sshd_config() {
        let written = entries(&env(&[
            ("GH_TOKEN", "gh"),
            ("NEWLINE", "x\nForceCommand /bin/false"),
            ("RETURN", "x\rPermitTTY no"),
            ("NUL", "x\0y"),
        ]));

        assert_eq!(written[1].3, "SetEnv \"GH_TOKEN=gh\"\n");
    }
}
