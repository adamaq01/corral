mod adapters;
mod cli;
mod config;
mod crypto;
mod healthcheck;
mod keys;
mod local_account;
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

#[tokio::main]
async fn main() -> Result<()> {
    crypto::install_provider();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve::run(&Config::load(&cli.config)?).await,
        Command::LocalAccount(LocalAccountCommand::Create { username, admin }) => {
            local_account::create(&Config::load(&cli.config)?, &username, admin).await
        }
        Command::Healthcheck => healthcheck::run(&Config::load(&cli.config)?).await,
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
