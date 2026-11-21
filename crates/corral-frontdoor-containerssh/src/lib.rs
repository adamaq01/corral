mod dto;
mod lockout;
mod routes;

pub use lockout::LockoutPolicy;
pub use routes::router;
use secrecy::SecretString;

#[derive(Clone, Debug)]
pub struct FrontDoorConfig {
    pub proxy_private_key: SecretString,
    pub lockout: LockoutPolicy,
}
