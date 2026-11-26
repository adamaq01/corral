use anyhow::{Result, ensure};
use corral_api::Key;
use corral_engine_agent_sandbox::AgentSandboxConfig;
use corral_engine_docker::DockerConfig;
use corral_frontdoor_gate::GateSecret;
use corral_identity_oidc::OidcConfig;
use corral_secrets_database::Keyring;
use corral_secrets_vault::{VaultAuth, VaultConfig};
use secrecy::SecretString;

use crate::config::{Config, Engine, Secrets};
use crate::keys::{self, GATE_SECRET, SECRETS_KEY, SESSION_KEY};

const DATABASE_URL: &str = "DATABASE_URL";
const DEFAULT_DATABASE_URL: &str = "sqlite:///var/lib/corral/corral.db";
const VAULT_TOKEN: &str = "CORRAL_VAULT_TOKEN";
const OIDC_CLIENT_SECRET: &str = "CORRAL_OIDC_CLIENT_SECRET";

/// The adapters the config selects, each with the secrets it reads from the environment.
pub struct Adapters {
    pub database_url: String,
    pub session_key: Key,
    pub gate_secret: GateSecret,
    pub secrets: SecretsChoice,
    pub engine: EngineChoice,
    pub oidc: Option<Oidc>,
}

pub enum SecretsChoice {
    Database(Keyring),
    Vault(VaultConfig),
}

pub enum EngineChoice {
    Docker(DockerConfig),
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
        let missing: Vec<&str> = [
            (SESSION_KEY, true),
            (SECRETS_KEY, matches!(config.secrets, Secrets::Database {})),
            (GATE_SECRET, true),
            (VAULT_TOKEN, vault_token),
            (OIDC_CLIENT_SECRET, config.identity.oidc.is_some()),
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
            Engine::Docker(docker) => EngineChoice::Docker(docker.clone()),
            Engine::AgentSandbox(agent) => EngineChoice::AgentSandbox(agent.clone()),
        };
        Ok(Self {
            database_url: database_url_from(var(DATABASE_URL)),
            session_key: keys::session_key(&secret(SESSION_KEY))?,
            gate_secret: keys::gate_secret(secret(GATE_SECRET))?,
            secrets,
            engine,
            oidc: config.identity.oidc.clone().map(|config| Oidc {
                config,
                client_secret: secret(OIDC_CLIENT_SECRET),
            }),
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
            "CORRAL_GATE_SECRET",
        ] {
            assert!(message.contains(name), "{message}");
        }
        assert!(!message.contains("CORRAL_SECRETS_KEY"), "{message}");
    }

    #[test]
    fn a_registry_password_variable_is_not_needed_to_start() {
        let config = config(
            "[[registries]]\nregistry = \"ghcr.io\"\nusername = \"corral\"\n\
             password_env = \"CORRAL_GHCR_TOKEN\"",
        );
        let vars = generated();
        let vars: Vec<(&str, &str)> = vars
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert!(read(&config, &vars).is_ok());
    }

    #[test]
    fn each_selected_adapter_gets_its_secrets() {
        let mut vars = generated();
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
        assert!(matches!(adapters.engine, EngineChoice::Docker(_)));
        assert!(adapters.oidc.is_none());
        assert_eq!(adapters.database_url, "sqlite:///var/lib/corral/corral.db");

        let elsewhere = config(
            "[identity.oidc]\nissuer = \"https://idp\"\nclient_id = \"corral\"\n\
             [secrets]\nkind = \"vault\"\naddress = \"http://vault\"\n[secrets.auth]\nkind = \"token\"\n\
             [engine]\nkind = \"agent-sandbox\"\nnamespace = \"n\"\nservice_account = \"s\"\n\
             storage_class = \"c\"",
        );
        let adapters = read(&elsewhere, &vars).unwrap();
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
