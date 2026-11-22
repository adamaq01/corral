use std::path::PathBuf;

use anyhow::{Result, ensure};
use corral_api::Key;
use corral_store_sql::Keyring;
use secrecy::SecretString;
use ssh_key::PublicKey;

use crate::config::{Config, Engine, OpenBaoAuth, Secrets};
use crate::keys::{self, PROXY_PUBLIC_KEY, SECRETS_KEY, SESSION_KEY};

const DATABASE_URL: &str = "DATABASE_URL";
const DEFAULT_DATABASE_URL: &str = "sqlite:///var/lib/corral/corral.db";
const STATIC_DIR: &str = "CORRAL_STATIC_DIR";
const OPENBAO_TOKEN: &str = "CORRAL_OPENBAO_TOKEN";
const OIDC_CLIENT_SECRET: &str = "CORRAL_OIDC_CLIENT_SECRET";

/// What `serve` reads from the environment. Each optional secret is set exactly when the config
/// selects the adapter that needs it.
pub struct Env {
    pub database_url: String,
    pub static_dir: Option<PathBuf>,
    pub session_key: Key,
    pub secrets_key: Option<Keyring>,
    pub openbao_token: Option<SecretString>,
    pub oidc_client_secret: Option<SecretString>,
    pub proxy_public_key: Option<PublicKey>,
}

impl Env {
    pub fn load(config: &Config) -> Result<Self> {
        Self::read(config, |name| std::env::var(name).ok())
    }

    fn read(config: &Config, lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        // An empty value counts as unset, so a `.env` line left blank falls back to the default.
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let secret = |name: &str| SecretString::from(var(name).unwrap_or_default());
        let database = matches!(config.secrets, Secrets::Database {});
        let openbao_token = matches!(
            &config.secrets,
            Secrets::OpenBao(bao) if matches!(bao.auth, OpenBaoAuth::Token {})
        );
        let oidc = config.identity.oidc.is_some();
        let docker = matches!(config.engine, Engine::Docker(_));
        let missing: Vec<&str> = [
            (SESSION_KEY, true),
            (SECRETS_KEY, database),
            (OPENBAO_TOKEN, openbao_token),
            (OIDC_CLIENT_SECRET, oidc),
            (PROXY_PUBLIC_KEY, docker),
        ]
        .into_iter()
        .filter(|(name, needed)| *needed && var(name).is_none())
        .map(|(name, _)| name)
        .collect();
        ensure!(
            missing.is_empty(),
            "set the environment variables {}",
            missing.join(", ")
        );
        Ok(Self {
            database_url: database_url_from(var(DATABASE_URL)),
            static_dir: var(STATIC_DIR).map(PathBuf::from),
            session_key: keys::session_key(&secret(SESSION_KEY))?,
            secrets_key: database
                .then(|| keys::keyring(&secret(SECRETS_KEY)))
                .transpose()?,
            openbao_token: openbao_token.then(|| secret(OPENBAO_TOKEN)),
            oidc_client_secret: oidc.then(|| secret(OIDC_CLIENT_SECRET)),
            proxy_public_key: docker
                .then(|| keys::proxy_public_key(&var(PROXY_PUBLIC_KEY).unwrap_or_default()))
                .transpose()?,
        })
    }

    /// The only variable `local-account create` needs.
    pub fn database_url() -> String {
        database_url_from(
            std::env::var(DATABASE_URL)
                .ok()
                .filter(|url| !url.is_empty()),
        )
    }
}

fn database_url_from(value: Option<String>) -> String {
    value.unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Env;
    use crate::config::Config;
    use crate::keys;

    const PROXY_KEY: &str = include_str!("../../corral-core/tests/fixtures/adam_ed25519.pub");

    fn config(extra: &str) -> Config {
        Config::parse(&format!(
            "[http]\nbase_url = \"http://corral.lan\"\n[ssh]\npublic_host = \"corral.lan\"\n\
             [identity.local]\n{extra}"
        ))
        .unwrap()
    }

    fn read(config: &Config, vars: &[(&str, &str)]) -> anyhow::Result<Env> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Env::read(config, |name| vars.get(name).cloned())
    }

    fn generated() -> Vec<(String, String)> {
        keys::generate()
            .lines()
            .map(|line| {
                let (name, value) = line.split_once('=').unwrap();
                (name.to_owned(), value.to_owned())
            })
            .collect()
    }

    #[test]
    fn every_missing_variable_is_reported_together() {
        let config = config(
            "[identity.oidc]\nissuer = \"https://idp\"\nclient_id = \"corral\"\n\
             [secrets]\nkind = \"openbao\"\naddress = \"http://bao\"\n[secrets.auth]\nkind = \"token\"",
        );
        let message = read(&config, &[("CORRAL_SESSION_KEY", "")])
            .err()
            .unwrap()
            .to_string();
        for name in [
            "CORRAL_SESSION_KEY",
            "CORRAL_OPENBAO_TOKEN",
            "CORRAL_OIDC_CLIENT_SECRET",
            "CORRAL_PROXY_PUBLIC_KEY",
        ] {
            assert!(message.contains(name), "{message}");
        }
        assert!(!message.contains("CORRAL_SECRETS_KEY"), "{message}");
    }

    #[test]
    fn only_the_selected_adapters_secrets_are_read() {
        let mut vars = generated();
        vars.push(("CORRAL_PROXY_PUBLIC_KEY".to_owned(), PROXY_KEY.to_owned()));
        vars.push(("CORRAL_OIDC_CLIENT_SECRET".to_owned(), "unused".to_owned()));
        let vars: Vec<(&str, &str)> = vars
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let env = read(&config(""), &vars).unwrap();
        assert!(env.secrets_key.is_some() && env.proxy_public_key.is_some());
        assert!(env.oidc_client_secret.is_none() && env.openbao_token.is_none());
        assert_eq!(env.database_url, "sqlite:///var/lib/corral/corral.db");
        assert!(env.static_dir.is_none());

        let agent = config(
            "[engine]\nkind = \"agent-sandbox\"\nnamespace = \"n\"\nservice_account = \"s\"\n\
             storage_class = \"c\"\nagent_image = \"a\"",
        );
        let without_proxy: Vec<_> = vars
            .iter()
            .copied()
            .filter(|(name, _)| *name != "CORRAL_PROXY_PUBLIC_KEY")
            .collect();
        assert!(
            read(&agent, &without_proxy)
                .unwrap()
                .proxy_public_key
                .is_none()
        );
    }
}
