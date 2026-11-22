use argon2::Argon2;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use corral_core::ports::PasswordHasher;
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretString};

#[derive(Clone, Copy, Debug, Default)]
pub struct Argon2Hasher;

impl PasswordHasher for Argon2Hasher {
    fn hash(&self, password: &SecretString) -> Result<String> {
        Argon2::default()
            .hash_password(password.expose_secret().as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| Error::internal("password hashing", err))
    }

    fn verify(&self, password: &SecretString, hash: &str) -> bool {
        Argon2::default()
            .verify_password(password.expose_secret().as_bytes(), hash)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use argon2::password_hash::PasswordHasher as _;
    use argon2::{Algorithm, Argon2, Params, Version};
    use corral_core::ports::PasswordHasher;
    use secrecy::SecretString;

    use super::Argon2Hasher;

    fn password(value: &str) -> SecretString {
        SecretString::from(value)
    }

    #[test]
    fn argon2_hash_round_trip() {
        let right = password("correct horse battery");
        let hash = Argon2Hasher.hash(&right).unwrap();
        let defaults = format!(
            "$argon2id$v=19$m={},t={},p={}$",
            Params::DEFAULT_M_COST,
            Params::DEFAULT_T_COST,
            Params::DEFAULT_P_COST
        );
        assert!(hash.starts_with(&defaults), "{hash}");
        assert!(!hash.contains("correct horse battery"));
        assert!(Argon2Hasher.verify(&right, &hash));
        assert!(!Argon2Hasher.verify(&password("wrong horse battery"), &hash));

        let again = Argon2Hasher.hash(&right).unwrap();
        assert_ne!(again, hash);
        assert!(Argon2Hasher.verify(&right, &again));
    }

    #[test]
    fn hashes_made_with_other_parameters_still_verify() {
        let right = password("correct horse battery");
        let cheaper = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        )
        .hash_password(b"correct horse battery")
        .unwrap()
        .to_string();
        assert!(Argon2Hasher.verify(&right, &cheaper));
        assert!(!Argon2Hasher.verify(&password("wrong horse battery"), &cheaper));
    }

    #[test]
    fn malformed_hashes_never_verify() {
        let right = password("correct horse battery");
        let hash = Argon2Hasher.hash(&right).unwrap();
        let truncated = &hash[..hash.rfind('$').unwrap()];
        for malformed in [
            "",
            "not a hash",
            "$argon2id$",
            truncated,
            "fake$correct horse battery",
        ] {
            assert!(!Argon2Hasher.verify(&right, malformed), "{malformed}");
        }
    }
}
