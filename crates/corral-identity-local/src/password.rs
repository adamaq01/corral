use argon2::Argon2;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretString};

pub(crate) const MIN_PASSWORD_CHARS: usize = 12;

pub(crate) fn hash(password: &SecretString) -> Result<String> {
    if password.expose_secret().chars().count() < MIN_PASSWORD_CHARS {
        return Err(Error::Invalid(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    Argon2::default()
        .hash_password(password.expose_secret().as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|err| Error::internal("password hashing", err))
}

pub(crate) fn verify(password: &SecretString, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.expose_secret().as_bytes(), hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use argon2::password_hash::PasswordHasher as _;
    use argon2::{Algorithm, Argon2, Params, Version};
    use corral_core::Error;
    use secrecy::SecretString;

    use super::{hash, verify};

    fn password(value: &str) -> SecretString {
        SecretString::from(value)
    }

    #[test]
    fn argon2_hash_round_trip() {
        let right = password("correct horse battery");
        let hashed = hash(&right).unwrap();
        let defaults = format!(
            "$argon2id$v=19$m={},t={},p={}$",
            Params::DEFAULT_M_COST,
            Params::DEFAULT_T_COST,
            Params::DEFAULT_P_COST
        );
        assert!(hashed.starts_with(&defaults), "{hashed}");
        assert!(!hashed.contains("correct horse battery"));
        assert!(verify(&right, &hashed));
        assert!(!verify(&password("wrong horse battery"), &hashed));

        let again = hash(&right).unwrap();
        assert_ne!(again, hashed);
        assert!(verify(&right, &again));
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
        assert!(verify(&right, &cheaper));
        assert!(!verify(&password("wrong horse battery"), &cheaper));
    }

    #[test]
    fn malformed_hashes_never_verify() {
        let right = password("correct horse battery");
        let hashed = hash(&right).unwrap();
        let truncated = &hashed[..hashed.rfind('$').unwrap()];
        for malformed in ["", "not a hash", "$argon2id$", truncated] {
            assert!(!verify(&right, malformed), "{malformed}");
        }
    }

    #[test]
    fn short_passwords_are_invalid_and_never_echoed() {
        for short in ["elevenchars", "ééééééééééé"] {
            let err = hash(&password(short)).unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{err:?}");
            assert!(!err.to_string().contains(short));
        }
        assert!(hash(&password("twelve chars")).is_ok());
    }
}
