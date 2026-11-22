use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{SshPublicKey, UserId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKey {
    pub id: Uuid,
    pub user_id: UserId,
    pub label: String,
    pub public_key: SshPublicKey,
    pub created_at: DateTime<Utc>,
}
