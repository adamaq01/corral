use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use corral_api::Key;
use corral_frontdoor_gate::GateSecret;
use corral_secrets_database::Keyring;
use secrecy::{ExposeSecret, SecretString};
use ssh_key::rand_core::{OsRng, RngCore};

pub const SESSION_KEY: &str = "CORRAL_SESSION_KEY";
pub const SECRETS_KEY: &str = "CORRAL_SECRETS_KEY";
pub const GATE_SECRET: &str = "CORRAL_GATE_SECRET";

pub fn generate() -> String {
    format!(
        "{SESSION_KEY}={}\n{SECRETS_KEY}=1:{}\n{GATE_SECRET}={}\n",
        STANDARD.encode(random::<64>()),
        STANDARD.encode(random::<32>()),
        STANDARD.encode(random::<32>()),
    )
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn session_key(value: &SecretString) -> Result<Key> {
    let bytes = STANDARD
        .decode(value.expose_secret().trim())
        .ok()
        .with_context(|| format!("{SESSION_KEY} is not base64"))?;
    Key::try_from(bytes.as_slice()).with_context(|| format!("{SESSION_KEY} is too short"))
}

pub fn keyring(value: &SecretString) -> Result<Keyring> {
    Keyring::from_env(value.expose_secret()).with_context(|| format!("{SECRETS_KEY} is invalid"))
}

pub fn gate_secret(value: SecretString) -> Result<GateSecret> {
    GateSecret::new(value).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use secrecy::SecretString;

    use super::{
        GATE_SECRET, SECRETS_KEY, SESSION_KEY, gate_secret, generate, keyring, session_key,
    };

    fn lines(output: &str) -> BTreeMap<String, SecretString> {
        output
            .lines()
            .map(|line| {
                let (name, value) = line.split_once('=').unwrap();
                (name.to_owned(), SecretString::from(value))
            })
            .collect()
    }

    #[test]
    fn generated_keys_are_env_lines_that_parse_back() {
        let output = generate();
        assert_eq!(output.lines().count(), 3);
        let keys = lines(&output);
        assert_eq!(
            keys.keys().map(String::as_str).collect::<Vec<_>>(),
            [GATE_SECRET, SECRETS_KEY, SESSION_KEY]
        );
        session_key(&keys[SESSION_KEY]).unwrap();
        keyring(&keys[SECRETS_KEY]).unwrap();
        gate_secret(keys[GATE_SECRET].clone()).unwrap();
    }

    #[test]
    fn every_generation_is_fresh() {
        assert_ne!(generate(), generate());
    }

    #[test]
    fn malformed_keys_are_rejected() {
        assert!(session_key(&secret("not base64!")).is_err());
        assert!(session_key(&secret(&STANDARD.encode([7; 63]))).is_err());
        assert!(keyring(&secret("1:short")).is_err());
        assert!(gate_secret(secret("short")).is_err());
    }

    fn secret(value: &str) -> SecretString {
        SecretString::from(value)
    }
}
