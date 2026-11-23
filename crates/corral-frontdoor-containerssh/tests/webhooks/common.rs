use axum::Router;
use corral_core::domain::{Sandbox, SandboxId, User};
use corral_core::fakes::{EngineCall, World};
use corral_frontdoor_containerssh::{ContainerSshConfig, router};
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey};

/// Adam and Eve, each with a registered key and a running sandbox `main`.
pub struct Fixture {
    pub world: World,
    pub adam: Member,
    pub eve: Member,
}

pub struct Member {
    pub user: User,
    pub key: PrivateKey,
    pub main: Sandbox,
}

impl Member {
    pub fn public_key(&self) -> String {
        public_line(&self.key)
    }
}

impl Fixture {
    pub async fn new() -> Self {
        let world = World::new();
        let adam = member(&world, "adam").await;
        let eve = member(&world, "eve").await;
        Self { world, adam, eve }
    }

    pub fn router(&self) -> Router {
        self.router_with(ContainerSshConfig::default())
    }

    pub fn router_with(&self, config: ContainerSshConfig) -> Router {
        router(self.world.access.clone(), config)
    }

    pub fn starts(&self, id: &SandboxId) -> usize {
        self.world
            .engine
            .calls()
            .iter()
            .filter(|call| matches!(call, EngineCall::Start { id: started, .. } if started == id))
            .count()
    }
}

async fn member(world: &World, handle: &str) -> Member {
    let user = world.member(handle).await;
    let key = generate_key();
    world
        .keys
        .add(&user, "laptop", &public_line(&key))
        .await
        .unwrap();
    let main = world.sandbox(&user, "main").await;
    Member { user, key, main }
}

pub fn generate_key() -> PrivateKey {
    PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
}

pub fn public_line(key: &PrivateKey) -> String {
    key.public_key().to_openssh().unwrap()
}

pub fn private_pem(key: &PrivateKey) -> String {
    key.to_openssh(LineEnding::LF).unwrap().to_string()
}
