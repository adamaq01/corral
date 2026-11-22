use std::io::{BufRead, IsTerminal};

use anyhow::{Context, Result, ensure};
use corral_core::domain::{Handle, Role};
use secrecy::SecretString;

use crate::config::Config;
use crate::{adapters, wiring};

pub async fn create(config: &Config, username: &str, admin: bool) -> Result<()> {
    let store = wiring::store(&adapters::database_url()).await?;
    let provider = wiring::local_provider(config, &store)?
        .context("local accounts are off: add [identity.local] to the config")?;
    let role = if admin { Role::Admin } else { Role::Member };
    provider
        .add(&Handle::parse(username)?, read_password()?, role)
        .await?;
    println!("created local account {username}");
    Ok(())
}

fn read_password() -> Result<SecretString> {
    if std::io::stdin().is_terminal() {
        let password = rpassword::prompt_password("Password: ")?;
        ensure!(
            rpassword::prompt_password("Repeat password: ")? == password,
            "the passwords differ"
        );
        return Ok(SecretString::from(password));
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(SecretString::from(line.trim_end_matches(['\r', '\n'])))
}
