use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;
use uuid::Uuid;

use super::lock;
use crate::domain::{
    CredentialName, Handle, LocalAccount, Sandbox, SandboxName, SshKey, SshPublicKey, User, UserId,
};
use crate::ports::{CredentialRepo, KeyRepo, LocalAccountRepo, SandboxRepo, UserRepo};
use crate::{Error, Result};

#[derive(Default)]
pub struct MemoryUserRepo {
    users: Mutex<Vec<User>>,
}

#[derive(Default)]
pub struct MemoryKeyRepo {
    keys: Mutex<Vec<SshKey>>,
}

#[derive(Default)]
pub struct MemorySandboxRepo {
    sandboxes: Mutex<Vec<Sandbox>>,
}

#[derive(Default)]
pub struct MemoryCredentialRepo {
    names: Mutex<BTreeSet<(UserId, CredentialName)>>,
}

#[derive(Default)]
pub struct MemoryLocalAccountRepo {
    accounts: Mutex<BTreeMap<Handle, LocalAccount>>,
}

fn find<T: Clone>(items: &Mutex<Vec<T>>, matches: impl Fn(&T) -> bool) -> Option<T> {
    lock(items).iter().find(|item| matches(item)).cloned()
}

fn oldest_first<T: Clone, K: Ord>(
    items: &Mutex<Vec<T>>,
    keep: impl Fn(&T) -> bool,
    created_at: impl Fn(&T) -> K,
) -> Vec<T> {
    let mut kept: Vec<T> = lock(items)
        .iter()
        .filter(|item| keep(item))
        .cloned()
        .collect();
    kept.sort_by_key(created_at);
    kept
}

#[async_trait]
impl UserRepo for MemoryUserRepo {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>> {
        Ok(find(&self.users, |u| u.subject == subject))
    }

    async fn by_handle(&self, handle: &Handle) -> Result<Option<User>> {
        Ok(find(&self.users, |u| u.handle == *handle))
    }

    async fn get(&self, id: UserId) -> Result<Option<User>> {
        Ok(find(&self.users, |u| u.id == id))
    }

    async fn insert(&self, user: &User) -> Result<()> {
        let mut users = lock(&self.users);
        if users
            .iter()
            .any(|u| u.id == user.id || u.subject == user.subject || u.handle == user.handle)
        {
            return Err(Error::Conflict(format!("user {}", user.handle)));
        }
        users.push(user.clone());
        Ok(())
    }

    async fn update(&self, user: &User) -> Result<()> {
        let mut users = lock(&self.users);
        if users
            .iter()
            .any(|u| u.id != user.id && (u.subject == user.subject || u.handle == user.handle))
        {
            return Err(Error::Conflict(format!("user {}", user.handle)));
        }
        let stored = users
            .iter_mut()
            .find(|u| u.id == user.id)
            .ok_or_else(|| Error::NotFound(format!("user {}", user.handle)))?;
        *stored = user.clone();
        Ok(())
    }

    async fn list(&self) -> Result<Vec<User>> {
        Ok(oldest_first(&self.users, |_| true, |u| u.created_at))
    }
}

#[async_trait]
impl KeyRepo for MemoryKeyRepo {
    async fn list(&self, user: UserId) -> Result<Vec<SshKey>> {
        Ok(oldest_first(
            &self.keys,
            |k| k.user_id == user,
            |k| k.created_at,
        ))
    }

    async fn insert(&self, key: &SshKey) -> Result<()> {
        let mut keys = lock(&self.keys);
        if keys
            .iter()
            .any(|k| k.id == key.id || k.public_key == key.public_key)
        {
            return Err(Error::Conflict(format!(
                "key {}",
                key.public_key.fingerprint()
            )));
        }
        keys.push(key.clone());
        Ok(())
    }

    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool> {
        let mut keys = lock(&self.keys);
        let before = keys.len();
        keys.retain(|k| !(k.user_id == user && k.id == id));
        Ok(keys.len() < before)
    }

    async fn by_public_key(&self, public_key: &SshPublicKey) -> Result<Option<SshKey>> {
        Ok(find(&self.keys, |k| k.public_key == *public_key))
    }
}

#[async_trait]
impl SandboxRepo for MemorySandboxRepo {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
        Ok(oldest_first(
            &self.sandboxes,
            |s| s.owner == owner,
            |s| s.created_at,
        ))
    }

    async fn list_all(&self) -> Result<Vec<Sandbox>> {
        Ok(oldest_first(&self.sandboxes, |_| true, |s| s.created_at))
    }

    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
        Ok(find(&self.sandboxes, |s| {
            s.owner == owner && s.name == *name
        }))
    }

    async fn insert(&self, sandbox: &Sandbox) -> Result<()> {
        let mut sandboxes = lock(&self.sandboxes);
        if sandboxes
            .iter()
            .any(|s| s.id == sandbox.id || (s.owner == sandbox.owner && s.name == sandbox.name))
        {
            return Err(Error::Conflict(format!("sandbox {}", sandbox.name)));
        }
        sandboxes.push(sandbox.clone());
        Ok(())
    }

    async fn update(&self, sandbox: &Sandbox) -> Result<()> {
        let mut sandboxes = lock(&self.sandboxes);
        if sandboxes
            .iter()
            .any(|s| s.id != sandbox.id && s.owner == sandbox.owner && s.name == sandbox.name)
        {
            return Err(Error::Conflict(format!("sandbox {}", sandbox.name)));
        }
        let stored = sandboxes
            .iter_mut()
            .find(|s| s.id == sandbox.id)
            .ok_or_else(|| Error::NotFound(format!("sandbox {}", sandbox.name)))?;
        *stored = sandbox.clone();
        Ok(())
    }

    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool> {
        let mut sandboxes = lock(&self.sandboxes);
        let before = sandboxes.len();
        sandboxes.retain(|s| !(s.owner == owner && s.name == *name));
        Ok(sandboxes.len() < before)
    }
}

#[async_trait]
impl CredentialRepo for MemoryCredentialRepo {
    async fn list(&self, user: UserId) -> Result<Vec<CredentialName>> {
        Ok(lock(&self.names)
            .iter()
            .filter(|(owner, _)| *owner == user)
            .map(|(_, name)| name.clone())
            .collect())
    }

    async fn upsert(&self, user: UserId, name: &CredentialName) -> Result<()> {
        lock(&self.names).insert((user, name.clone()));
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        Ok(lock(&self.names).remove(&(user, name.clone())))
    }
}

#[async_trait]
impl LocalAccountRepo for MemoryLocalAccountRepo {
    async fn get(&self, username: &Handle) -> Result<Option<LocalAccount>> {
        Ok(lock(&self.accounts).get(username).cloned())
    }

    async fn insert(&self, account: &LocalAccount) -> Result<()> {
        match lock(&self.accounts).entry(account.username.clone()) {
            Entry::Occupied(_) => Err(Error::Conflict(format!(
                "local account {}",
                account.username
            ))),
            Entry::Vacant(slot) => {
                slot.insert(account.clone());
                Ok(())
            }
        }
    }

    async fn update(&self, account: &LocalAccount) -> Result<()> {
        let mut accounts = lock(&self.accounts);
        let stored = accounts
            .get_mut(&account.username)
            .ok_or_else(|| Error::NotFound(format!("local account {}", account.username)))?;
        *stored = account.clone();
        Ok(())
    }

    async fn delete(&self, username: &Handle) -> Result<bool> {
        Ok(lock(&self.accounts).remove(username).is_some())
    }

    async fn list(&self) -> Result<Vec<LocalAccount>> {
        Ok(lock(&self.accounts).values().cloned().collect())
    }
}
