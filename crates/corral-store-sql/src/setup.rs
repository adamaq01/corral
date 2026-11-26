use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{
    Attachment, Entry, EntryId, Glimpse, HomePath, Kept, Owner, SandboxId, SecretId, SecretRef,
    Shared, StoredValue, Target, UserId, VariableName,
};
use corral_core::ports::SetupRepo;
use corral_core::{Error, Result};
use corral_sql::{db_error, on_pool, parsed};
use uuid::Uuid;

use crate::Store;

const VARIABLE: &str = "variable";
const FILE: &str = "file";
const REQUIRED: &str = "required";
const OFFERED: &str = "offered";

#[derive(sqlx::FromRow)]
struct EntryRow {
    id: Uuid,
    owner: Option<Uuid>,
    shared: Option<String>,
    kind: String,
    target: String,
    template: bool,
    executable: bool,
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

/// The owner as its columns hold it: a member's id, or how the entry is shared.
fn owner_columns(owner: &Owner) -> (Option<Uuid>, Option<&'static str>) {
    match owner {
        Owner::Member(id) => (Some(id.0), None),
        Owner::Everyone(Shared::Required) => (None, Some(REQUIRED)),
        Owner::Everyone(Shared::Offered) => (None, Some(OFFERED)),
    }
}

fn owner(id: Option<Uuid>, shared: Option<&str>) -> Result<Owner> {
    match (id, shared) {
        (Some(id), None) => Ok(Owner::Member(UserId(id))),
        (None, Some(REQUIRED)) => Ok(Owner::Everyone(Shared::Required)),
        (None, Some(OFFERED)) => Ok(Owner::Everyone(Shared::Offered)),
        (id, shared) => Err(Error::internal(
            "stored entry owner",
            format!("{id:?} sharing as {shared:?} is neither a member's nor everyone's"),
        )),
    }
}

impl TryFrom<EntryRow> for Entry {
    type Error = Error;

    fn try_from(row: EntryRow) -> Result<Self> {
        let target = match row.kind.as_str() {
            VARIABLE => {
                Target::Variable(parsed("variable name", VariableName::parse(&row.target))?)
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
            owner: owner(row.owner, row.shared.as_deref())?,
            target,
            template: row.template,
            executable: row.executable,
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
         glimpse_length, glimpse_head, glimpse_tail, set_at, created_at, executable, shared"
    };
}

/// The attachments `a` that `$rest` after the `from` picks.
macro_rules! select_attachments {
    ($rest:literal) => {
        concat!(
            "select a.sandbox_id, a.entry_id, a.override_content, a.override_secret_id, \
             a.override_size, a.override_glimpse_length, a.override_glimpse_head, \
             a.override_glimpse_tail, a.override_set_at from setup_attachments a ",
            $rest
        )
    };
}

/// The entries `$rest` after the `from` picks.
macro_rules! select_entries {
    ($rest:literal) => {
        concat!("select ", entry_columns!(), " from setup_entries ", $rest)
    };
}

#[async_trait]
impl SetupRepo for Store {
    async fn entries(&self, owner: UserId) -> Result<Vec<Entry>> {
        let rows: Vec<EntryRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select_entries!("where owner = $1 order by created_at, id"))
                .bind(owner.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Entry::try_from).collect()
    }

    async fn shared_entries(&self) -> Result<Vec<Entry>> {
        let rows: Vec<EntryRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select_entries!(
                "where shared is not null order by created_at, id"
            ))
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Entry::try_from).collect()
    }

    async fn every_member_entry(&self) -> Result<Vec<Entry>> {
        let rows: Vec<EntryRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select_entries!(
                "where owner is not null order by created_at, id"
            ))
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Entry::try_from).collect()
    }

    async fn insert(&self, entry: &Entry) -> Result<()> {
        let (kind, target) = kind_and_target(&entry.target);
        let (owner, shared) = owner_columns(&entry.owner);
        let value = Columns::of(&entry.value);
        on_pool!(self.pool, |pool| {
            sqlx::query(concat!(
                "insert into setup_entries (",
                entry_columns!(),
                ") values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"
            ))
            .bind(entry.id.0)
            .bind(owner)
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
            .bind(entry.executable)
            .bind(shared)
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }

    async fn update(&self, entry: &Entry, overrides: &[Attachment]) -> Result<bool> {
        let (kind, target) = kind_and_target(&entry.target);
        let (owner, shared) = owner_columns(&entry.owner);
        let value = Columns::of(&entry.value);
        let overrides: Vec<(&Attachment, Option<Columns>)> = overrides
            .iter()
            .map(|attachment| (attachment, attachment.overriding.as_ref().map(Columns::of)))
            .collect();
        on_pool!(self.pool, |pool| async {
            let mut tx = pool.begin().await?;
            let updated = sqlx::query(
                "update setup_entries set kind = $3, target = $4, template = $5, \
                 executable = $6, apply_to_new = $7, content = $8, secret_id = $9, size = $10, \
                 glimpse_length = $11, glimpse_head = $12, glimpse_tail = $13, set_at = $14, \
                 shared = $15 where id = $1 and owner is not distinct from $2",
            )
            .bind(entry.id.0)
            .bind(owner)
            .bind(kind)
            .bind(target)
            .bind(entry.template)
            .bind(entry.executable)
            .bind(entry.apply_to_new)
            .bind(&value.content)
            .bind(value.secret_id)
            .bind(value.size)
            .bind(value.glimpse_length)
            .bind(&value.glimpse_head)
            .bind(&value.glimpse_tail)
            .bind(value.set_at)
            .bind(shared)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if updated == 0 {
                return Ok(false);
            }
            for (attachment, value) in &overrides {
                let value = value.as_ref();
                sqlx::query(
                    "update setup_attachments set override_content = $3, \
                     override_secret_id = $4, override_size = $5, override_glimpse_length = $6, \
                     override_glimpse_head = $7, override_glimpse_tail = $8, \
                     override_set_at = $9 where sandbox_id = $1 and entry_id = $2",
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
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            Ok(true)
        }
        .await)
        .map_err(db_error)
    }

    async fn delete(&self, owner: &Owner, id: EntryId) -> Result<bool> {
        let (owner, shared) = owner_columns(owner);
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query(
                "delete from setup_entries where id = $1 and owner is not distinct from $2 \
                 and shared is not distinct from $3",
            )
            .bind(id.0)
            .bind(owner)
            .bind(shared)
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn attachments(&self, owner: UserId) -> Result<Vec<Attachment>> {
        let rows: Vec<AttachmentRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select_attachments!(
                "join setup_entries e on e.id = a.entry_id where e.owner = $1"
            ))
            .bind(owner.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Attachment::try_from).collect()
    }

    async fn attachments_of(&self, entry: EntryId) -> Result<Vec<Attachment>> {
        let rows: Vec<AttachmentRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select_attachments!("where a.entry_id = $1"))
                .bind(entry.0)
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
                "insert into setup_attachments (sandbox_id, entry_id, override_content, \
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
            sqlx::query("delete from setup_attachments where sandbox_id = $1 and entry_id = $2")
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
