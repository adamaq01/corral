use std::io::{BufRead, IsTerminal};

use anyhow::{Context, Result, ensure};
use corral_core::domain::Role;
use secrecy::SecretString;

use crate::config::Config;
use crate::{store, wiring};

pub async fn create(config: &Config, username: &str, admin: bool) -> Result<()> {
    let store = store::connect().await?;
    let accounts = wiring::local_accounts(config, &store)?
        .context("local accounts are off: set [identity.local] enabled = true")?;
    let role = if admin { Role::Admin } else { Role::Member };
    accounts
        .create(None, username, read_password()?, role)
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
