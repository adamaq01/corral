mod dto;
mod lockout;
mod routes;

pub use lockout::LockoutPolicy;
pub use routes::router;

#[derive(Clone, Copy, Debug)]
pub struct FrontDoorConfig {
    pub lockout: LockoutPolicy,
}
