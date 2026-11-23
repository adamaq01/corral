use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{
    Attachment, CredentialName, Entry, EntryId, Glimpse, HomePath, Kept, SandboxId, SecretId,
    SecretRef, StoredValue, Target, UserId,
};
use corral_core::ports::EnvironmentRepo;
use corral_core::{Error, Result};
use corral_sql::{db_error, on_pool, parsed};
use uuid::Uuid;

use crate::Store;

const VARIABLE: &str = "variable";
const FILE: &str = "file";

#[derive(sqlx::FromRow)]
struct EntryRow {
    id: Uuid,
    owner: Uuid,
    kind: String,
    target: String,
    template: bool,
    apply_to_new: bool,
    content: Option<Vec<u8>>,
    secret_id: Option<Uuid>,
    size: i64,
    glimpse_length: Option<i64>,
    glimpse_head: Option<String>,
    glimpse_tail: Option<String>,
    set_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    sandbox_id: String,
    entry_id: Uuid,
    override_content: Option<Vec<u8>>,
    override_secret_id: Option<Uuid>,
    override_size: Option<i64>,
    override_glimpse_length: Option<i64>,
    override_glimpse_head: Option<String>,
    override_glimpse_tail: Option<String>,
    override_set_at: Option<DateTime<Utc>>,
}

/// A value as its columns hold it, either the content itself or where the secret store keeps it.
struct Columns {
    content: Option<Vec<u8>>,
    secret_id: Option<Uuid>,
    size: i64,
    glimpse_length: Option<i64>,
    glimpse_head: Option<String>,
    glimpse_tail: Option<String>,
    set_at: DateTime<Utc>,
}

impl Columns {
    fn of(value: &StoredValue) -> Self {
        let (content, secret) = match &value.kept {
            Kept::Plain(content) => (Some(content.clone()), None),
            Kept::Secret(secret) => (None, Some(secret)),
        };
        let glimpse = secret.and_then(|secret| secret.glimpse.as_ref());
        let (head, tail) = glimpse.and_then(|glimpse| glimpse.ends.clone()).unzip();
        Self {
            content,
            secret_id: secret.map(|secret| secret.id.0),
            size: count(value.size()),
            glimpse_length: glimpse.map(|glimpse| count(glimpse.length)),
            glimpse_head: head,
            glimpse_tail: tail,
            set_at: value.set_at,
        }
    }

    fn value(self) -> Result<StoredValue> {
        let kept = match (self.content, self.secret_id) {
            (Some(content), None) => Kept::Plain(content),
            (None, Some(id)) => Kept::Secret(SecretRef {
                id: SecretId(id),
                size: amount("size", self.size)?,
                glimpse: self
                    .glimpse_length
                    .map(|length| {
                        Ok::<_, Error>(Glimpse {
                            length: amount("glimpse length", length)?,
                            ends: self.glimpse_head.zip(self.glimpse_tail),
                        })
                    })
                    .transpose()?,
            }),
            _ => {
                return Err(Error::internal(
                    "stored value",
                    "has both or neither of a content and a secret id",
                ));
            }
        };
        Ok(StoredValue {
            kept,
            set_at: self.set_at,
        })
    }
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn amount(column: &str, value: i64) -> Result<usize> {
    parsed(column, usize::try_from(value))
}

impl TryFrom<EntryRow> for Entry {
    type Error = Error;

    fn try_from(row: EntryRow) -> Result<Self> {
        let target = match row.kind.as_str() {
            VARIABLE => {
                Target::Variable(parsed("variable name", CredentialName::parse(&row.target))?)
            }
            FILE => Target::File(parsed("file path", HomePath::parse(&row.target))?),
            other => {
                return Err(Error::internal(
                    "stored entry kind",
                    format!("{other:?} is neither variable nor file"),
                ));
            }
        };
        Ok(Self {
            id: EntryId(row.id),
            owner: UserId(row.owner),
            target,
            template: row.template,
            apply_to_new: row.apply_to_new,
            value: Columns {
                content: row.content,
                secret_id: row.secret_id,
                size: row.size,
                glimpse_length: row.glimpse_length,
                glimpse_head: row.glimpse_head,
                glimpse_tail: row.glimpse_tail,
                set_at: row.set_at,
            }
            .value()?,
            created_at: row.created_at,
        })
    }
}

impl TryFrom<AttachmentRow> for Attachment {
    type Error = Error;

    fn try_from(row: AttachmentRow) -> Result<Self> {
        let overriding = match (row.override_size, row.override_set_at) {
            (Some(size), Some(set_at)) => Some(
                Columns {
                    content: row.override_content,
                    secret_id: row.override_secret_id,
                    size,
                    glimpse_length: row.override_glimpse_length,
                    glimpse_head: row.override_glimpse_head,
                    glimpse_tail: row.override_glimpse_tail,
                    set_at,
                }
                .value()?,
            ),
            _ => None,
        };
        Ok(Self {
            sandbox: SandboxId(row.sandbox_id),
            entry: EntryId(row.entry_id),
            overriding,
        })
    }
}

fn kind_and_target(target: &Target) -> (&'static str, &str) {
    match target {
        Target::Variable(name) => (VARIABLE, name.as_str()),
        Target::File(path) => (FILE, path.as_str()),
    }
}

macro_rules! entry_columns {
    () => {
        "id, owner, kind, target, template, apply_to_new, content, secret_id, size, \
         glimpse_length, glimpse_head, glimpse_tail, set_at, created_at"
    };
}

#[async_trait]
impl EnvironmentRepo for Store {
    async fn entries(&self, owner: UserId) -> Result<Vec<Entry>> {
        let rows: Vec<EntryRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(concat!(
                "select ",
                entry_columns!(),
                " from environment_entries where owner = $1 order by created_at, id"
            ))
            .bind(owner.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Entry::try_from).collect()
    }

    async fn insert(&self, entry: &Entry) -> Result<()> {
        let (kind, target) = kind_and_target(&entry.target);
        let value = Columns::of(&entry.value);
        on_pool!(self.pool, |pool| {
            sqlx::query(concat!(
                "insert into environment_entries (",
                entry_columns!(),
                ") values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"
            ))
            .bind(entry.id.0)
            .bind(entry.owner.0)
            .bind(kind)
            .bind(target)
            .bind(entry.template)
            .bind(entry.apply_to_new)
            .bind(&value.content)
            .bind(value.secret_id)
            .bind(value.size)
            .bind(value.glimpse_length)
            .bind(&value.glimpse_head)
            .bind(&value.glimpse_tail)
            .bind(value.set_at)
            .bind(entry.created_at)
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }

    async fn update(&self, entry: &Entry) -> Result<bool> {
        let value = Columns::of(&entry.value);
        let updated = on_pool!(self.pool, |pool| {
            sqlx::query(
                "update environment_entries set template = $3, apply_to_new = $4, content = $5, \
                 secret_id = $6, size = $7, glimpse_length = $8, glimpse_head = $9, \
                 glimpse_tail = $10, set_at = $11 where id = $1 and owner = $2",
            )
            .bind(entry.id.0)
            .bind(entry.owner.0)
            .bind(entry.template)
            .bind(entry.apply_to_new)
            .bind(&value.content)
            .bind(value.secret_id)
            .bind(value.size)
            .bind(value.glimpse_length)
            .bind(&value.glimpse_head)
            .bind(&value.glimpse_tail)
            .bind(value.set_at)
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(updated > 0)
    }

    async fn delete(&self, owner: UserId, id: EntryId) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from environment_entries where id = $1 and owner = $2")
                .bind(id.0)
                .bind(owner.0)
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn attachments(&self, owner: UserId) -> Result<Vec<Attachment>> {
        let rows: Vec<AttachmentRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select a.sandbox_id, a.entry_id, a.override_content, a.override_secret_id, \
                 a.override_size, a.override_glimpse_length, a.override_glimpse_head, \
                 a.override_glimpse_tail, a.override_set_at \
                 from environment_attachments a \
                 join environment_entries e on e.id = a.entry_id where e.owner = $1",
            )
            .bind(owner.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Attachment::try_from).collect()
    }

    async fn attach(&self, attachment: &Attachment) -> Result<()> {
        let value = attachment.overriding.as_ref().map(Columns::of);
        let value = value.as_ref();
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into environment_attachments (sandbox_id, entry_id, override_content, \
                 override_secret_id, override_size, override_glimpse_length, \
                 override_glimpse_head, override_glimpse_tail, override_set_at) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 on conflict (sandbox_id, entry_id) do update set \
                 override_content = excluded.override_content, \
                 override_secret_id = excluded.override_secret_id, \
                 override_size = excluded.override_size, \
                 override_glimpse_length = excluded.override_glimpse_length, \
                 override_glimpse_head = excluded.override_glimpse_head, \
                 override_glimpse_tail = excluded.override_glimpse_tail, \
                 override_set_at = excluded.override_set_at",
            )
            .bind(&attachment.sandbox.0)
            .bind(attachment.entry.0)
            .bind(value.and_then(|value| value.content.as_ref()))
            .bind(value.and_then(|value| value.secret_id))
            .bind(value.map(|value| value.size))
            .bind(value.and_then(|value| value.glimpse_length))
            .bind(value.and_then(|value| value.glimpse_head.as_ref()))
            .bind(value.and_then(|value| value.glimpse_tail.as_ref()))
            .bind(value.map(|value| value.set_at))
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }

    async fn detach(&self, sandbox: &SandboxId, entry: EntryId) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query(
                "delete from environment_attachments where sandbox_id = $1 and entry_id = $2",
            )
            .bind(&sandbox.0)
            .bind(entry.0)
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn awaiting_adoption(&self) -> Result<Vec<UserId>> {
        let users: Vec<Uuid> = on_pool!(self.pool, |pool| {
            sqlx::query_scalar("select user_id from credentials_to_adopt")
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        Ok(users.into_iter().map(UserId).collect())
    }

    async fn adopted(&self, user: UserId) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("delete from credentials_to_adopt where user_id = $1")
                .bind(user.0)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }
}
