use std::collections::BTreeMap;
use std::fmt;

use aes_gcm::{Aes256Gcm, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::{Error, Result};
use zeroize::Zeroizing;

#[derive(Debug)]
pub struct Keyring {
    current: u8,
    keys: BTreeMap<u8, Key>,
}

pub(crate) struct Key(Aes256Gcm);

impl Keyring {
    pub fn from_env(value: &str) -> Result<Self> {
        let mut current = None;
        let mut keys = BTreeMap::new();
        for (position, entry) in (1..).zip(value.split(',')) {
            let (version, key) = parse_entry(position, entry)?;
            if keys.insert(version, key).is_some() {
                return Err(invalid(position, "repeats an earlier key version"));
            }
            current.get_or_insert(version);
        }
        let current = current.ok_or_else(|| invalid(1, "is missing"))?;
        Ok(Self { current, keys })
    }

    pub(crate) fn current(&self) -> (u8, &Aes256Gcm) {
        (self.current, &self.keys[&self.current].0)
    }

    pub(crate) fn get(&self, version: u8) -> Option<&Aes256Gcm> {
        self.keys.get(&version).map(|key| &key.0)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key([REDACTED])")
    }
}

fn parse_entry(position: usize, entry: &str) -> Result<(u8, Key)> {
    let (version, encoded) = entry
        .trim()
        .split_once(':')
        .ok_or_else(|| invalid(position, "must look like <version>:<base64 key>"))?;
    let version = version
        .parse()
        .map_err(|_| invalid(position, "must start with a version from 0 to 255"))?;
    let mut bytes = Zeroizing::new([0; 32]);
    match STANDARD.decode_slice(encoded, bytes.as_mut_slice()) {
        Ok(32) => Ok((version, Key(Aes256Gcm::new((&*bytes).into())))),
        _ => Err(invalid(
            position,
            "must hold exactly 32 bytes encoded as standard base64",
        )),
    }
}

fn invalid(position: usize, problem: &str) -> Error {
    Error::Invalid(format!("CORRAL_SECRETS_KEY entry {position} {problem}"))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use corral_core::Error;

    use super::Keyring;

    fn encoded(byte: u8, len: usize) -> String {
        STANDARD.encode(vec![byte; len])
    }

    #[test]
    fn bad_key_env_is_rejected() {
        assert!(Keyring::from_env("1:short").is_err());

        let key = encoded(7, 32);
        assert!(key.starts_with("BwcH"));
        for value in [
            String::new(),
            key.clone(),
            format!("{key}:1"),
            format!("x:{key}"),
            format!("256:{key}"),
            format!("1:{}", encoded(7, 31)),
            format!("1:{}", encoded(7, 33)),
            format!("1:{}", &key[..43]),
            format!("1:{key}!"),
            format!("1:{key},"),
            format!("1:{key},1:{key}"),
        ] {
            match Keyring::from_env(&value) {
                Err(Error::Invalid(message)) => assert!(!message.contains("BwcH"), "{message}"),
                other => panic!("{value:?} gave {other:?}"),
            }
        }
    }

    #[test]
    fn first_entry_is_current_and_whitespace_is_ignored() {
        let keyring =
            Keyring::from_env(&format!(" 2:{} ,1:{}\n", encoded(2, 32), encoded(1, 32))).unwrap();
        assert_eq!(keyring.current().0, 2);
        assert!(keyring.get(1).is_some());
        assert!(keyring.get(3).is_none());
    }

    #[test]
    fn debug_is_redacted() {
        let keyring =
            Keyring::from_env(&format!("2:{},1:{}", encoded(2, 32), encoded(1, 32))).unwrap();
        assert_eq!(
            format!("{keyring:?}"),
            "Keyring { current: 2, keys: {1: Key([REDACTED]), 2: Key([REDACTED])} }"
        );
    }
}
