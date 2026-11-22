use aes_gcm::Nonce;
use aes_gcm::aead::{Aead, Generate, Payload};
use corral_core::{Error, Result};

use crate::Keyring;

const NONCE_LEN: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub key_version: u8,
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl Sealed {
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1 + NONCE_LEN + self.ciphertext.len());
        bytes.push(self.key_version);
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let truncated = || Error::internal("sealed value", "truncated");
        let (&key_version, rest) = bytes.split_first().ok_or_else(truncated)?;
        let (nonce, ciphertext) = rest.split_first_chunk().ok_or_else(truncated)?;
        Ok(Self {
            key_version,
            nonce: *nonce,
            ciphertext: ciphertext.to_vec(),
        })
    }
}

/// # Panics
/// If `plaintext` exceeds the AES-GCM message limit of 64 GiB.
#[must_use]
pub fn seal(keyring: &Keyring, aad: &[u8], plaintext: &[u8]) -> Sealed {
    let (key_version, cipher) = keyring.current();
    let nonce = Nonce::generate();
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("plaintext within the AES-GCM message limit");
    Sealed {
        key_version,
        nonce: nonce.into(),
        ciphertext,
    }
}

pub fn open(keyring: &Keyring, aad: &[u8], sealed: &Sealed) -> Result<Vec<u8>> {
    let cipher = keyring.get(sealed.key_version).ok_or_else(|| {
        Error::internal(
            "sealed value",
            format!(
                "sealed with key version {}, which CORRAL_SECRETS_KEY does not hold",
                sealed.key_version
            ),
        )
    })?;
    cipher
        .decrypt(
            (&sealed.nonce).into(),
            Payload {
                msg: &sealed.ciphertext,
                aad,
            },
        )
        .map_err(|_| Error::internal("sealed value", "does not match its key, owner and name"))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use corral_core::Error;

    use super::{Sealed, open, seal};
    use crate::Keyring;

    fn kr() -> Keyring {
        keyring(&[(1, 1)])
    }

    fn keyring(entries: &[(u8, u8)]) -> Keyring {
        let value = entries
            .iter()
            .map(|(version, byte)| format!("{version}:{}", STANDARD.encode([*byte; 32])))
            .collect::<Vec<_>>()
            .join(",");
        Keyring::from_env(&value).unwrap()
    }

    #[test]
    fn round_trip() {
        let s = seal(&kr(), b"u/OPENAI_API_KEY", b"sk-1");
        assert_eq!(open(&kr(), b"u/OPENAI_API_KEY", &s).unwrap(), b"sk-1");
    }

    #[test]
    fn swapped_owner_or_name_fails() {
        let s = seal(&kr(), b"u1/A", b"x");
        assert!(open(&kr(), b"u2/A", &s).is_err());
        assert!(open(&kr(), b"u1/B", &s).is_err());
    }

    #[test]
    fn older_key_version_still_opens() {
        let old = seal(&keyring(&[(1, 1)]), b"u/A", b"x");
        let rotated = keyring(&[(2, 2), (1, 1)]);
        assert_eq!(open(&rotated, b"u/A", &old).unwrap(), b"x");
        let new = seal(&rotated, b"u/A", b"x");
        assert_eq!((old.key_version, new.key_version), (1, 2));
        assert!(matches!(
            open(&keyring(&[(1, 1)]), b"u/A", &new),
            Err(Error::Internal { .. })
        ));
    }

    #[test]
    fn wrong_key_under_the_same_version_fails() {
        let s = seal(&keyring(&[(1, 1)]), b"u/A", b"x");
        assert!(matches!(
            open(&keyring(&[(1, 9)]), b"u/A", &s),
            Err(Error::Internal { .. })
        ));
    }

    #[test]
    fn every_seal_draws_a_fresh_nonce() {
        let (a, b) = (seal(&kr(), b"u/A", b"x"), seal(&kr(), b"u/A", b"x"));
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn bytes_are_version_nonce_then_ciphertext() {
        let s = seal(&kr(), b"u/A", b"secret");
        let bytes = s.to_bytes();
        assert_eq!(bytes[0], 1);
        assert_eq!(&bytes[1..13], &s.nonce);
        assert_eq!(&bytes[13..], &s.ciphertext[..]);
        assert_eq!(Sealed::from_bytes(&bytes).unwrap(), s);
        for len in 0..bytes.len() {
            let opened =
                Sealed::from_bytes(&bytes[..len]).and_then(|cut| open(&kr(), b"u/A", &cut));
            assert!(matches!(opened, Err(Error::Internal { .. })), "{len}");
        }
    }
}
