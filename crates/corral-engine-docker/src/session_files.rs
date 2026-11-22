use corral_core::ports::CredentialMap;
use secrecy::ExposeSecret;
use ssh_key::PublicKey;
use tracing::warn;

const AUTHORIZED_KEYS: &str = "etc/ssh/corral_authorized_keys";
const SSHD_ENV: &str = "etc/corral/sshd_env.conf";

/// A tar archive, to extract at `/`, of what the sandbox's sshd reads when it starts: the key
/// the front door logs in with, and the credentials its sessions see.
pub(crate) fn archive(proxy_key: &PublicKey, env: &CredentialMap) -> Vec<u8> {
    let authorized_key = format!(
        "{}\n",
        proxy_key.to_openssh().expect("a public key always encodes")
    );
    let mut archive = tar::Builder::new(Vec::new());
    for (path, mode, contents) in [
        (AUTHORIZED_KEYS, 0o644, authorized_key),
        (SSHD_ENV, 0o600, set_env(env)),
    ] {
        let mut header = tar::Header::new_ustar();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        archive
            .append_data(&mut header, path, contents.as_bytes())
            .expect("an archive in memory always writes");
    }
    archive
        .into_inner()
        .expect("an archive in memory always writes")
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

    use corral_core::domain::CredentialName;
    use corral_core::ports::CredentialMap;
    use secrecy::SecretString;
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

    fn entries(env: &CredentialMap) -> Vec<(String, u32, (u64, u64), String)> {
        let bytes = archive(&PublicKey::from_openssh(PROXY_KEY).unwrap(), env);
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
    fn archive_holds_root_owned_sshd_files() {
        let entries = entries(&env(&[("GH_TOKEN", "gh"), ("ANTHROPIC_API_KEY", "sk")]));

        assert_eq!(
            entries,
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
