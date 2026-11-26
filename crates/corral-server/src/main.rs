mod adapters;
mod cli;
mod config;
mod crypto;
mod deletions;
mod gate;
mod healthcheck;
mod keys;
mod local_account;
mod registry_logins;
mod serve;
mod ui;
#[cfg(test)]
mod web_types;
mod wiring;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

use crate::cli::{Cli, Command, ConfigCommand, KeysCommand, LocalAccountCommand};
use crate::config::Config;

fn log_filter(rust_log: &str) -> EnvFilter {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .parse_lossy(rust_log);
    corral_secrets_vault::NOISY_LOG_TARGETS
        .into_iter()
        .filter(|target| !rust_log.contains(target))
        .fold(filter, |filter, target| {
            filter.add_directive(format!("{target}=off").parse().expect("a valid directive"))
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    crypto::install_provider();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(log_filter(&std::env::var("RUST_LOG").unwrap_or_default()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve::run(&Config::load(&cli.config)?).await,
        Command::Gate => gate::run(&Config::load(&cli.config)?).await,
        Command::LocalAccount(LocalAccountCommand::Create { username, admin }) => {
            local_account::create(&Config::load(&cli.config)?, &username, admin).await
        }
        Command::Healthcheck { gate } => healthcheck::run(&Config::load(&cli.config)?, gate).await,
        Command::Config(ConfigCommand::Check) => {
            Config::load(&cli.config)?;
            println!("{} is valid", cli.config.display());
            Ok(())
        }
        Command::Keys(KeysCommand::Generate) => {
            print!("{}", keys::generate());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::log_filter;

    #[test]
    fn vault_client_logs_are_off_unless_rust_log_names_them() {
        let cases = [
            ("", "vaultrs=off,rustify=off,info"),
            ("debug", "vaultrs=off,rustify=off,debug"),
            ("info,vaultrs=debug", "vaultrs=debug,rustify=off,info"),
        ];
        for (rust_log, expected) in cases {
            assert_eq!(log_filter(rust_log).to_string(), expected, "{rust_log:?}");
        }
    }
}
