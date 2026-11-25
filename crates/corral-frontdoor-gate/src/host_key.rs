use std::path::Path;

use corral_core::{Error, Result};
use russh::keys::PrivateKey;

/// The gate's host key from `path`, in OpenSSH form as `ssh-keygen` writes it or in PKCS#8 PEM
/// form as Helm's `genPrivateKey` writes it.
pub fn load(path: &Path) -> Result<PrivateKey> {
    let failed =
        |err: String| Error::internal(format!("read the SSH host key {}", path.display()), err);
    let text = std::fs::read_to_string(path).map_err(|err| failed(err.to_string()))?;
    russh::keys::decode_secret_key(&text, None).map_err(|err| failed(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::load;

    fn run(program: &str, args: &[&str], path: &Path) -> String {
        let output = Command::new(program).args(args).arg(path).output().unwrap();
        assert!(output.status.success(), "{program} {args:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// What OpenSSH reads as the public half of the key at `path`.
    fn openssh_public(path: &Path) -> String {
        key_of(&run("ssh-keygen", &["-y", "-f"], path))
    }

    fn key_of(line: &str) -> String {
        line.split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn reads_the_key_openssh_reads_in_either_form() {
        let dir = tempfile::tempdir().unwrap();
        let openssh = dir.path().join("openssh");
        run(
            "ssh-keygen",
            &["-q", "-t", "ed25519", "-N", "", "-f"],
            &openssh,
        );
        let pem = dir.path().join("pem");
        run(
            "openssl",
            &["genpkey", "-algorithm", "ed25519", "-out"],
            &pem,
        );
        std::fs::set_permissions(&pem, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        for path in [&openssh, &pem] {
            let key = load(path).unwrap();
            assert_eq!(
                key_of(&key.public_key().to_openssh().unwrap()),
                openssh_public(path),
                "{}",
                path.display()
            );
        }

        std::fs::write(dir.path().join("garbage"), "not a key").unwrap();
        assert!(load(&dir.path().join("garbage")).is_err());
        assert!(load(&dir.path().join("missing")).is_err());
    }
}
