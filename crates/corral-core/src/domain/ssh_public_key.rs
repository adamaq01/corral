use ssh_key::{Algorithm, HashAlg, PublicKey};

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshPublicKey {
    openssh: String,
    fingerprint: String,
}

impl SshPublicKey {
    pub fn parse(line: &str) -> Result<Self> {
        let invalid = || {
            Error::Invalid("expected an OpenSSH public key, certificates are not accepted".into())
        };
        let mut key = PublicKey::from_openssh(line.trim()).map_err(|_| invalid())?;
        // ssh-key parses any name@domain algorithm, certificate types included, as an opaque key.
        if matches!(key.algorithm(), Algorithm::Other(_)) {
            return Err(invalid());
        }
        key.set_comment("");
        Ok(Self {
            openssh: key.to_openssh().map_err(|_| invalid())?,
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.openssh
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::SshPublicKey;
    use crate::Error;
    use crate::fakes::{
        ADAM_CERT, ADAM_FINGERPRINT, ADAM_KEY, ECDSA_FINGERPRINT, ECDSA_KEY, RSA_FINGERPRINT,
        RSA_KEY, algorithm_and_blob,
    };

    #[test]
    fn parse_normalizes_and_fingerprints_like_ssh_keygen() {
        for (line, fingerprint) in [
            (ADAM_KEY, ADAM_FINGERPRINT),
            (RSA_KEY, RSA_FINGERPRINT),
            (ECDSA_KEY, ECDSA_FINGERPRINT),
        ] {
            let key = SshPublicKey::parse(line).unwrap();
            assert_eq!(key.as_str(), algorithm_and_blob(line));
            assert_eq!(key.fingerprint(), fingerprint);
        }
    }

    #[test]
    fn parse_ignores_comments_and_surrounding_whitespace() {
        let bare = algorithm_and_blob(ADAM_KEY);
        let expected = SshPublicKey::parse(&bare).unwrap();
        for line in [format!("  {bare}\n"), format!("{bare} someone@else")] {
            assert_eq!(SshPublicKey::parse(&line).unwrap(), expected, "{line}");
        }
    }

    #[test]
    fn parse_rejects_certificates_and_unknown_key_types() {
        for bad in [
            ADAM_CERT,
            "ssh-ed25519-cert-v01@openssh.com AAAAIHNzaC1lZDI1NTE5LWNlcnQtdjAxQG9wZW5zc2guY29tAAAAB2tleWRhdGE=",
            "foo@example.com AAAAD2Zvb0BleGFtcGxlLmNvbQAAAAdrZXlkYXRh",
            "ssh-ed25519 not-base64",
            "ssh-ed25519",
            "",
        ] {
            assert!(
                matches!(SshPublicKey::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }
}
