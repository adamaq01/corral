pub mod environment;
pub mod repos;
pub mod secrets;
pub mod settings;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::{Handle, PictureUrl, Role, User, UserId, UserStatus};
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
        picture: Some(
            PictureUrl::parse(&format!("https://idp.example.com/{handle}.png"))
                .expect("valid picture"),
        ),
        handle: Handle::parse(&handle).expect("valid handle"),
        role: Role::Member,
        status: UserStatus::Active,
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
    use crate::fakes::{
        MemoryEnvironmentRepo, MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore,
        MemorySettingsRepo, MemoryUserRepo,
    };

    #[tokio::test]
    async fn the_fakes_meet_every_contract() {
        let users = MemoryUserRepo::default();
        let sandboxes = MemorySandboxRepo::default();
        super::repos::every_repo(&users, &MemoryKeyRepo::default(), &sandboxes).await;
        super::environment::environment_repo(&users, &sandboxes, &MemoryEnvironmentRepo::default())
            .await;
        let secrets = MemorySecretStore::default();
        super::secrets::secret_store(&secrets, &secrets).await;
        super::settings::settings_repo(&users, &MemorySettingsRepo::default()).await;
    }
}
