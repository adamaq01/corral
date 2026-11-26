use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use corral_core::app::{Registry, Settings};
use corral_core::ports::{RegistryAccess, RegistryLogin, RegistryLogins};
use secrecy::SecretString;

type Lookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// The registries settings list, each with the password its variable holds at the moment Corral
/// reaches it.
pub struct EnvRegistryLogins {
    settings: Settings,
    lookup: Box<Lookup>,
    warned: Mutex<BTreeSet<String>>,
}

impl EnvRegistryLogins {
    pub fn new(
        settings: Settings,
        lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            settings,
            lookup: Box::new(lookup),
            warned: Mutex::default(),
        }
    }

    /// Whether `variable` holds a password; an empty one is none.
    pub fn password_present(&self, variable: &str) -> bool {
        self.password(variable).is_some()
    }

    fn password(&self, variable: &str) -> Option<String> {
        (self.lookup)(variable).filter(|value| !value.is_empty())
    }

    fn login(&self, row: &Registry) -> Option<RegistryLogin> {
        let (username, variable) = row.username.as_ref().zip(row.password_env.as_ref())?;
        if let Some(password) = self.password(variable) {
            return Some(RegistryLogin {
                username: username.clone(),
                password: SecretString::from(password),
            });
        }
        let first = self
            .warned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(variable.clone());
        if first {
            tracing::warn!(
                registry = %row.registry,
                %variable,
                "the registry's password variable is not set, so Corral reaches it without a login"
            );
        }
        None
    }
}

#[async_trait]
impl RegistryLogins for EnvRegistryLogins {
    async fn registries(&self) -> corral_core::Result<Vec<RegistryAccess>> {
        Ok(self
            .settings
            .registries()
            .await?
            .into_iter()
            .map(|row| RegistryAccess {
                login: self.login(&row),
                registry: row.registry,
                plain_http: row.plain_http,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use corral_core::app::{Configured, Registry, Source};
    use corral_core::fakes::World;
    use corral_core::ports::RegistryLogins;
    use secrecy::ExposeSecret;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::EnvRegistryLogins;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn logins(password: Option<&'static str>) -> EnvRegistryLogins {
        let world = World::with_configured(Configured {
            registries: vec![Registry {
                registry: "ghcr.io".to_owned(),
                username: Some("corral".to_owned()),
                password_env: Some("CORRAL_GHCR".to_owned()),
                plain_http: false,
                source: Source::Config,
            }],
            ..Configured::default()
        });
        EnvRegistryLogins::new(world.settings, move |name: &str| {
            password
                .filter(|_| name == "CORRAL_GHCR")
                .map(str::to_owned)
        })
    }

    #[tokio::test]
    async fn an_unset_password_variable_means_no_login_and_a_warning() {
        let capture = Capture::default();
        let writer = capture.clone();
        let _scope = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish()
            .set_default();

        for password in [None, Some("")] {
            let unset = logins(password);
            for _ in 0..2 {
                let registries = unset.registries().await.unwrap();
                assert_eq!(registries[0].registry, "ghcr.io");
                assert!(registries[0].login.is_none());
            }
            assert!(!unset.password_present("CORRAL_GHCR"));
        }
        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert_eq!(logs.matches("WARN").count(), 2, "{logs}");
        assert_eq!(logs.matches("CORRAL_GHCR").count(), 2, "{logs}");

        let set = logins(Some("tok"));
        let registries = set.registries().await.unwrap();
        let login = registries[0].login.as_ref().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.expose_secret()),
            ("corral", "tok")
        );
        assert!(set.password_present("CORRAL_GHCR"));
    }
}
