//! The SSH front door, `corral gate`: an SSH server that asks the Corral server who connects and
//! which sandbox they reach, and carries every channel into that sandbox over an `sshd -i` the
//! engine runs there. The server's half of the API between the two is here too.

mod api;
mod backlog;
mod bridge;
mod client;
mod config;
mod connection;
pub mod host_key;
mod limits;
mod link;
mod relay;
mod routes;
mod secret;
mod server;
mod session;

pub use client::ServerClient;
pub use config::{Cidr, GateConfig};
pub use limits::Limits;
pub use routes::router;
pub use secret::GateSecret;
pub use server::Gate;
