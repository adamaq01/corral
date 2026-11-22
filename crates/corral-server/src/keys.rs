use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use corral_api::Key;
use corral_store_sql::Keyring;
use secrecy::{ExposeSecret, SecretString};
use ssh_key::rand_core::{OsRng, RngCore};
use ssh_key::{Algorithm, LineEnding, PrivateKey};

pub const SESSION_KEY: &str = "CORRAL_SESSION_KEY";
pub const SECRETS_KEY: &str = "CORRAL_SECRETS_KEY";
pub const PROXY_PRIVATE_KEY: &str = "CORRAL_PROXY_PRIVATE_KEY";

/// The environment carries the OpenSSH PEM base64-encoded, so it fits on one `.env` line.
pub struct ProxyKey {
    pub private_pem: SecretString,
    pub public_openssh: String,
}

pub fn generate() -> Result<String> {
    let proxy = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)?;
    let pem = proxy.to_openssh(LineEnding::LF)?;
    Ok(format!(
        "{SESSION_KEY}={}\n{SECRETS_KEY}=1:{}\n{PROXY_PRIVATE_KEY}={}\n",
        STANDARD.encode(random::<64>()),
        STANDARD.encode(random::<32>()),
        STANDARD.encode(pem.as_bytes()),
    ))
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

impl ProxyKey {
    pub fn parse(value: &SecretString) -> Result<Self> {
        let pem = STANDARD
            .decode(value.expose_secret().trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .with_context(|| {
                format!("{PROXY_PRIVATE_KEY} must be a base64-encoded OpenSSH private key")
            })?;
        let key = PrivateKey::from_openssh(&pem)
            .with_context(|| format!("{PROXY_PRIVATE_KEY} is not an OpenSSH private key"))?;
        ensure!(
            !key.is_encrypted(),
            "{PROXY_PRIVATE_KEY} must not be protected by a passphrase"
        );
        Ok(Self {
            public_openssh: key.public_key().to_openssh()?,
            private_pem: SecretString::from(pem),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use secrecy::{ExposeSecret, SecretString};
    use ssh_key::{PrivateKey, PublicKey};

    use super::{
        PROXY_PRIVATE_KEY, ProxyKey, SECRETS_KEY, SESSION_KEY, generate, keyring, session_key,
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
    fn generated_keys_are_three_env_lines_that_parse_back() {
        let output = generate().unwrap();
        assert_eq!(output.lines().count(), 3);
        let keys = lines(&output);
        assert_eq!(
            keys.keys().map(String::as_str).collect::<Vec<_>>(),
            [PROXY_PRIVATE_KEY, SECRETS_KEY, SESSION_KEY]
        );

        session_key(&keys[SESSION_KEY]).unwrap();
        keyring(&keys[SECRETS_KEY]).unwrap();
        let proxy = ProxyKey::parse(&keys[PROXY_PRIVATE_KEY]).unwrap();
        let private = PrivateKey::from_openssh(proxy.private_pem.expose_secret()).unwrap();
        assert!(
            proxy
                .private_pem
                .expose_secret()
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert_eq!(
            PublicKey::from_openssh(&proxy.public_openssh).unwrap(),
            *private.public_key()
        );
    }

    #[test]
    fn every_generation_is_fresh() {
        assert_ne!(generate().unwrap(), generate().unwrap());
    }

    #[test]
    fn malformed_keys_are_rejected() {
        assert!(session_key(&secret("not base64!")).is_err());
        assert!(session_key(&secret(&STANDARD.encode([7; 63]))).is_err());
        assert!(keyring(&secret("1:short")).is_err());
        let raw_pem = generate_pem();
        assert!(ProxyKey::parse(&secret(&raw_pem)).is_err());
        assert!(ProxyKey::parse(&secret(&STANDARD.encode("ssh-ed25519 AAAA"))).is_err());
    }

    fn secret(value: &str) -> SecretString {
        SecretString::from(value)
    }

    fn generate_pem() -> String {
        let keys = lines(&generate().unwrap());
        let encoded = keys[PROXY_PRIVATE_KEY].expose_secret().to_owned();
        String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap()
    }
}
