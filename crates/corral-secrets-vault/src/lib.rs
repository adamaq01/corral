mod config;
mod error;
mod session;
mod store;

pub use config::{VaultAuth, VaultConfig};
pub use store::VaultStore;

/// Log targets to silence: vaultrs and its HTTP client rustify log every failed request as an
/// ERROR, a 404 for a user without credentials included. The store returns the failures that
/// matter to its caller, which logs them.
pub const NOISY_LOG_TARGETS: [&str; 2] = ["vaultrs", "rustify"];
