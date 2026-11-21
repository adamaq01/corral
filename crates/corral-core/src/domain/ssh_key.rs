use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::UserId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKey {
    pub id: Uuid,
    pub user_id: UserId,
    pub label: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}
