pub mod secrets;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::{CredentialName, Handle, Role, User, UserId};
use crate::ports::UserRepo;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + seconds, 0).expect("timestamp in range")
}

fn unique(label: &str) -> String {
    format!("{label}-{}", &Uuid::new_v4().simple().to_string()[..12])
}

fn credential(name: &str) -> CredentialName {
    CredentialName::parse(name).expect("valid credential name")
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

async fn seed_user(users: &dyn UserRepo, label: &str) -> User {
    let user = new_user(label, at(0));
    users.insert(&user).await.expect("seed user");
    user
}
