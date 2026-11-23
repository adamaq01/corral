use anyhow::{Result, ensure};
use corral_api::Key;
use corral_core::ports::{RegistryAccess, RegistryLogin};
use corral_engine_agent_sandbox::AgentSandboxConfig;
use corral_engine_docker::DockerConfig;
use corral_identity_oidc::OidcConfig;
use corral_secrets_database::Keyring;
use corral_secrets_vault::{VaultAuth, VaultConfig};
use secrecy::SecretString;
use ssh_key::PublicKey;

use crate::config::{Config, Engine, Secrets};
use crate::keys::{self, PROXY_PUBLIC_KEY, SECRETS_KEY, SESSION_KEY};

const DATABASE_URL: &str = "DATABASE_URL";
const DEFAULT_DATABASE_URL: &str = "sqlite:///var/lib/corral/corral.db";
const VAULT_TOKEN: &str = "CORRAL_VAULT_TOKEN";
const OIDC_CLIENT_SECRET: &str = "CORRAL_OIDC_CLIENT_SECRET";

/// The adapters the config selects, each with the secrets it reads from the environment.
pub struct Adapters {
    pub database_url: String,
    pub session_key: Key,
    pub secrets: SecretsChoice,
    pub engine: EngineChoice,
    pub oidc: Option<Oidc>,
    pub registries: Vec<RegistryAccess>,
}

pub enum SecretsChoice {
    Database(Keyring),
    Vault(VaultConfig),
}

pub enum EngineChoice {
    Docker {
        config: DockerConfig,
        proxy_public_key: PublicKey,
    },
    AgentSandbox(AgentSandboxConfig),
}

pub struct Oidc {
    pub config: OidcConfig,
    pub client_secret: SecretString,
}

impl Adapters {
    pub fn resolve(config: &Config) -> Result<Self> {
        Self::read(config, |name| std::env::var(name).ok())
    }

    fn read(config: &Config, lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        // An empty value counts as unset, so a `.env` line left blank falls back to the default.
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let vault_token = matches!(
            &config.secrets,
            Secrets::Vault(vault) if matches!(vault.auth, VaultAuth::Token { .. })
        );
        let passwords = config
            .registries
            .iter()
            .filter_map(|registry| registry.password_env.as_deref());
        let missing: Vec<&str> = [
            (SESSION_KEY, true),
            (SECRETS_KEY, matches!(config.secrets, Secrets::Database {})),
            (VAULT_TOKEN, vault_token),
            (OIDC_CLIENT_SECRET, config.identity.oidc.is_some()),
            (PROXY_PUBLIC_KEY, matches!(config.engine, Engine::Docker(_))),
        ]
        .into_iter()
        .chain(passwords.map(|name| (name, true)))
        .filter(|(name, needed)| *needed && var(name).is_none())
        .map(|(name, _)| name)
        .collect();
        ensure!(
            missing.is_empty(),
            "set the environment variables {}",
            missing.join(", ")
        );
        let set = |name: &str| var(name).unwrap_or_default();
        let secret = |name: &str| SecretString::from(set(name));
        let secrets = match &config.secrets {
            Secrets::Database {} => SecretsChoice::Database(keys::keyring(&secret(SECRETS_KEY))?),
            Secrets::Vault(vault) => {
                let mut vault = vault.clone();
                if let VaultAuth::Token { token } = &mut vault.auth {
                    *token = secret(VAULT_TOKEN);
                }
                SecretsChoice::Vault(vault)
            }
        };
        let engine = match &config.engine {
            Engine::Docker(docker) => EngineChoice::Docker {
                config: docker.clone(),
                proxy_public_key: keys::proxy_public_key(&set(PROXY_PUBLIC_KEY))?,
            },
            Engine::AgentSandbox(agent) => EngineChoice::AgentSandbox(agent.clone()),
        };
        Ok(Self {
            database_url: database_url_from(var(DATABASE_URL)),
            session_key: keys::session_key(&secret(SESSION_KEY))?,
            secrets,
            engine,
            oidc: config.identity.oidc.clone().map(|config| Oidc {
                config,
                client_secret: secret(OIDC_CLIENT_SECRET),
            }),
            registries: config
                .registries
                .iter()
                .map(|registry| RegistryAccess {
                    registry: registry.registry.clone(),
                    login: registry
                        .username
                        .clone()
                        .zip(registry.password_env.as_deref())
                        .map(|(username, password)| RegistryLogin {
                            username,
                            password: secret(password),
                        }),
                    plain_http: registry.plain_http,
                })
                .collect(),
        })
    }
}

/// The only variable `local-account create` needs.
pub fn database_url() -> String {
    database_url_from(
        std::env::var(DATABASE_URL)
            .ok()
            .filter(|url| !url.is_empty()),
    )
}

fn database_url_from(value: Option<String>) -> String {
    value.unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use corral_secrets_vault::VaultAuth;
    use secrecy::ExposeSecret;

    use super::{Adapters, EngineChoice, SecretsChoice};
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

    fn read(config: &Config, vars: &[(&str, &str)]) -> anyhow::Result<Adapters> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Adapters::read(config, |name| vars.get(name).cloned())
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
             [secrets]\nkind = \"vault\"\naddress = \"http://vault\"\n[secrets.auth]\nkind = \"token\"",
        );
        let message = read(&config, &[("CORRAL_SESSION_KEY", "")])
            .err()
            .unwrap()
            .to_string();
        for name in [
            "CORRAL_SESSION_KEY",
            "CORRAL_VAULT_TOKEN",
            "CORRAL_OIDC_CLIENT_SECRET",
            "CORRAL_PROXY_PUBLIC_KEY",
        ] {
            assert!(message.contains(name), "{message}");
        }
        assert!(!message.contains("CORRAL_SECRETS_KEY"), "{message}");
    }

    #[test]
    fn registries_get_the_passwords_their_variables_hold() {
        let config = config(
            "[[registries]]\nregistry = \"ghcr.io\"\nusername = \"corral\"\n\
             password_env = \"CORRAL_GHCR_TOKEN\"\n\
             [[registries]]\nregistry = \"registry.lan:5000\"\nplain_http = true",
        );
        let mut vars = generated();
        vars.push(("CORRAL_PROXY_PUBLIC_KEY".to_owned(), PROXY_KEY.to_owned()));
        let without: Vec<(&str, &str)> = vars
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let message = read(&config, &without).err().unwrap().to_string();
        assert!(message.contains("CORRAL_GHCR_TOKEN"), "{message}");

        let mut with = without.clone();
        with.push(("CORRAL_GHCR_TOKEN", "ghp_token"));
        let registries = read(&config, &with).unwrap().registries;
        assert_eq!(registries.len(), 2);
        let ghcr = registries[0].login.as_ref().unwrap();
        assert_eq!(
            (
                registries[0].registry.as_str(),
                ghcr.username.as_str(),
                ghcr.password.expose_secret(),
                registries[0].plain_http
            ),
            ("ghcr.io", "corral", "ghp_token", false)
        );
        assert!(registries[1].login.is_none() && registries[1].plain_http);
    }

    #[test]
    fn each_selected_adapter_gets_its_secrets() {
        let mut vars = generated();
        vars.push(("CORRAL_PROXY_PUBLIC_KEY".to_owned(), PROXY_KEY.to_owned()));
        vars.push((
            "CORRAL_OIDC_CLIENT_SECRET".to_owned(),
            "oidc-secret".to_owned(),
        ));
        vars.push(("CORRAL_VAULT_TOKEN".to_owned(), "s.vault-token".to_owned()));
        let vars: Vec<(&str, &str)> = vars
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let adapters = read(&config(""), &vars).unwrap();
        assert!(matches!(adapters.secrets, SecretsChoice::Database(_)));
        assert!(matches!(
            adapters.engine,
            EngineChoice::Docker { proxy_public_key, .. }
                if proxy_public_key.to_openssh().unwrap() == PROXY_KEY.trim()
        ));
        assert!(adapters.oidc.is_none());
        assert_eq!(adapters.database_url, "sqlite:///var/lib/corral/corral.db");

        let elsewhere = config(
            "[identity.oidc]\nissuer = \"https://idp\"\nclient_id = \"corral\"\n\
             [secrets]\nkind = \"vault\"\naddress = \"http://vault\"\n[secrets.auth]\nkind = \"token\"\n\
             [engine]\nkind = \"agent-sandbox\"\nnamespace = \"n\"\nservice_account = \"s\"\n\
             storage_class = \"c\"\nagent_image = \"a\"",
        );
        let without_proxy: Vec<_> = vars
            .iter()
            .copied()
            .filter(|(name, _)| *name != "CORRAL_PROXY_PUBLIC_KEY")
            .collect();
        let adapters = read(&elsewhere, &without_proxy).unwrap();
        let SecretsChoice::Vault(vault) = adapters.secrets else {
            panic!("vault was configured");
        };
        assert!(matches!(
            vault.auth,
            VaultAuth::Token { token } if token.expose_secret() == "s.vault-token"
        ));
        assert!(matches!(adapters.engine, EngineChoice::AgentSandbox(_)));
        assert_eq!(
            adapters.oidc.unwrap().client_secret.expose_secret(),
            "oidc-secret"
        );
    }
}
