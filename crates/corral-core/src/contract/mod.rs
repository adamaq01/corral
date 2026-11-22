pub mod repos;
pub mod secrets;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::{Handle, Role, User, UserId};
use crate::ports::UserRepo;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + seconds, 0).expect("timestamp in range")
}

fn unique(label: &str) -> String {
    format!("{label}-{}", &Uuid::new_v4().simple().to_string()[..12])
}

fn new_user(label: &str, created_at: DateTime<Utc>) -> User {
    let handle = unique(label);
    User {
        id: UserId(Uuid::new_v4()),
        subject: format!("subject-{handle}"),
        email: Some(format!("{handle}@example.com")),
        handle: Handle::parse(&handle).expect("valid handle"),
        role: Role::Member,
        enabled: true,
        created_at,
    }
}

pub async fn seed_user(users: &dyn UserRepo, label: &str) -> User {
    let user = new_user(label, at(0));
    users.insert(&user).await.expect("seed user");
    user
}

#[cfg(test)]
mod tests {
    use crate::fakes::{MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore, MemoryUserRepo};

    #[tokio::test]
    async fn the_fakes_meet_every_contract() {
        let users = MemoryUserRepo::default();
        super::repos::every_repo(
            &users,
            &MemoryKeyRepo::default(),
            &MemorySandboxRepo::default(),
        )
        .await;
        super::secrets::secret_store(&MemorySecretStore::default()).await;
    }
}
