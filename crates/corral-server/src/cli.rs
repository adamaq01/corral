use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Per-user sandboxes reachable over SSH")]
pub struct Cli {
    /// The TOML config file.
    #[arg(
        long,
        global = true,
        env = "CORRAL_CONFIG",
        default_value = "/etc/corral/config.toml"
    )]
    pub config: PathBuf,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Serve the web API and the SSH front door's webhooks (the default).
    Serve,
    /// Manage local accounts.
    #[command(subcommand)]
    LocalAccount(LocalAccountCommand),
    /// Generate keys.
    #[command(subcommand)]
    Keys(KeysCommand),
    /// Exit 0 when both listeners answer `/healthz`, for container healthchecks.
    Healthcheck,
}

#[derive(Subcommand)]
pub enum LocalAccountCommand {
    /// Create a local account. The password is prompted for on a terminal, else read from stdin.
    Create {
        username: String,
        /// Give the account the admin role.
        #[arg(long)]
        admin: bool,
    },
}

#[derive(Subcommand)]
pub enum KeysCommand {
    /// Print fresh `CORRAL_SESSION_KEY`, `CORRAL_SECRETS_KEY` and `CORRAL_PROXY_PRIVATE_KEY`
    /// lines for a .env file.
    Generate,
}
