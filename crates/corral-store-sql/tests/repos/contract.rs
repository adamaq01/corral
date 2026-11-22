use std::sync::Arc;

use chrono::{DateTime, Utc};
use corral_core::Error;
use corral_core::domain::{
    Handle, LocalAccount, Role, Sandbox, SandboxId, SandboxName, SshKey, SshPublicKey, User, UserId,
};
use corral_core::ports::{KeyRepo, LocalAccountRepo, SandboxRepo, UserRepo};
use ssh_key::PublicKey;
use ssh_key::public::{Ed25519PublicKey, KeyData};
use tokio::task::JoinSet;
use uuid::Uuid;

pub async fn user_repo(repo: &dyn UserRepo) {
    let older = new_user("adam", at(1));
    let newer = User {
        email: None,
        ..new_user("eve", at(2))
    };
    repo.insert(&newer).await.expect("insert newer user");
    repo.insert(&older).await.expect("insert older user");

    assert_eq!(repo.get(older.id).await.unwrap(), Some(older.clone()));
    assert_eq!(repo.get(newer.id).await.unwrap(), Some(newer.clone()));
    assert_eq!(
        repo.by_subject(&older.subject).await.unwrap(),
        Some(older.clone())
    );
    assert_eq!(
        repo.by_handle(&older.handle).await.unwrap(),
        Some(older.clone())
    );
    assert_eq!(repo.get(UserId(Uuid::new_v4())).await.unwrap(), None);
    assert_eq!(repo.by_subject(&unique("nobody")).await.unwrap(), None);
    assert_eq!(
        repo.by_handle(&handle(&unique("nobody"))).await.unwrap(),
        None
    );
    let ours = |users: Vec<User>| -> Vec<User> {
        users
            .into_iter()
            .filter(|u| u.id == older.id || u.id == newer.id)
            .collect()
    };
    assert_eq!(
        ours(repo.list().await.unwrap()),
        vec![older.clone(), newer.clone()]
    );

    let duplicates = [
        User {
            subject: unique("subject"),
            handle: handle(&unique("other")),
            ..older.clone()
        },
        User {
            id: UserId(Uuid::new_v4()),
            handle: handle(&unique("other")),
            ..older.clone()
        },
        User {
            id: UserId(Uuid::new_v4()),
            subject: unique("subject"),
            ..older.clone()
        },
    ];
    for duplicate in duplicates {
        assert!(
            matches!(repo.insert(&duplicate).await, Err(Error::Conflict(_))),
            "{duplicate:?}"
        );
        assert_ne!(repo.get(duplicate.id).await.unwrap(), Some(duplicate));
    }

    let updated = User {
        handle: handle(&unique("renamed")),
        email: None,
        role: Role::Admin,
        enabled: false,
        ..older.clone()
    };
    repo.update(&updated).await.expect("update user");
    assert_eq!(repo.get(older.id).await.unwrap(), Some(updated.clone()));
    assert_eq!(repo.by_handle(&older.handle).await.unwrap(), None);
    assert_eq!(
        repo.by_handle(&updated.handle).await.unwrap(),
        Some(updated.clone())
    );

    let taking_a_handle = User {
        handle: newer.handle.clone(),
        ..updated.clone()
    };
    assert!(matches!(
        repo.update(&taking_a_handle).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(repo.get(older.id).await.unwrap(), Some(updated));

    let absent = new_user("ghost", at(3));
    assert!(matches!(
        repo.update(&absent).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(repo.get(absent.id).await.unwrap(), None);
}

pub async fn key_repo(users: &dyn UserRepo, keys: &dyn KeyRepo) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let older = new_key(adam.id, 1);
    let newer = new_key(adam.id, 2);
    let eves = new_key(eve.id, 3);
    for key in [&newer, &older, &eves] {
        keys.insert(key).await.expect("insert key");
    }

    assert_eq!(
        keys.list(adam.id).await.unwrap(),
        vec![older.clone(), newer.clone()]
    );
    assert_eq!(keys.list(eve.id).await.unwrap(), vec![eves.clone()]);
    assert_eq!(keys.list(UserId(Uuid::new_v4())).await.unwrap(), vec![]);
    assert_eq!(
        keys.by_public_key(&older.public_key).await.unwrap(),
        Some(older.clone())
    );
    assert_eq!(
        keys.by_public_key(&new_key(adam.id, 0).public_key)
            .await
            .unwrap(),
        None
    );

    let duplicates = [
        SshKey {
            id: Uuid::new_v4(),
            user_id: eve.id,
            ..older.clone()
        },
        SshKey {
            public_key: new_key(adam.id, 0).public_key,
            ..older.clone()
        },
    ];
    for duplicate in duplicates {
        assert!(
            matches!(keys.insert(&duplicate).await, Err(Error::Conflict(_))),
            "{duplicate:?}"
        );
    }
    assert_eq!(
        keys.list(adam.id).await.unwrap(),
        vec![older.clone(), newer.clone()]
    );
    assert_eq!(keys.list(eve.id).await.unwrap(), vec![eves.clone()]);

    assert!(!keys.delete(eve.id, older.id).await.unwrap());
    assert_eq!(
        keys.list(adam.id).await.unwrap(),
        vec![older.clone(), newer.clone()]
    );
    assert!(keys.delete(adam.id, older.id).await.unwrap());
    assert!(!keys.delete(adam.id, older.id).await.unwrap());
    assert_eq!(keys.list(adam.id).await.unwrap(), vec![newer]);
    assert_eq!(keys.by_public_key(&older.public_key).await.unwrap(), None);
}

pub async fn sandbox_repo(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let main = new_sandbox(adam.id, "main", 1);
    let eves_main = new_sandbox(eve.id, "main", 2);
    let dev = new_sandbox(adam.id, "dev", 3);
    for sandbox in [&dev, &eves_main, &main] {
        sandboxes.insert(sandbox, 2).await.expect("insert sandbox");
    }

    assert_eq!(
        sandboxes.list(adam.id).await.unwrap(),
        vec![main.clone(), dev.clone()]
    );
    let ours: Vec<Sandbox> = sandboxes
        .list_all()
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.owner == adam.id || s.owner == eve.id)
        .collect();
    assert_eq!(ours, vec![main.clone(), eves_main.clone(), dev.clone()]);
    assert_eq!(
        sandboxes.by_name(adam.id, &main.name).await.unwrap(),
        Some(main.clone())
    );
    assert_eq!(
        sandboxes.by_name(eve.id, &main.name).await.unwrap(),
        Some(eves_main.clone())
    );
    assert_eq!(
        sandboxes
            .by_name(adam.id, &sandbox_name("missing"))
            .await
            .unwrap(),
        None
    );

    let duplicates = [
        new_sandbox(adam.id, "main", 4),
        Sandbox {
            name: sandbox_name("other"),
            ..main.clone()
        },
    ];
    for duplicate in duplicates {
        assert!(
            matches!(
                sandboxes.insert(&duplicate, 5).await,
                Err(Error::Conflict(_))
            ),
            "{duplicate:?}"
        );
    }
    assert!(matches!(
        sandboxes.insert(&new_sandbox(adam.id, "third", 4), 2).await,
        Err(Error::LimitReached(_))
    ));
    assert_eq!(
        sandboxes.list(adam.id).await.unwrap(),
        vec![main.clone(), dev.clone()]
    );

    assert_eq!(sandboxes.default_id(adam.id).await.unwrap(), None);
    sandboxes.set_default(adam.id, &dev.id).await.unwrap();
    sandboxes.set_default(adam.id, &eves_main.id).await.unwrap();
    sandboxes.set_default(eve.id, &dev.id).await.unwrap();
    assert_eq!(
        sandboxes.default_id(adam.id).await.unwrap(),
        Some(dev.id.clone())
    );
    assert_eq!(sandboxes.default_id(eve.id).await.unwrap(), None);

    assert!(!sandboxes.delete(eve.id, &dev.name).await.unwrap());
    assert!(sandboxes.delete(adam.id, &dev.name).await.unwrap());
    assert!(!sandboxes.delete(adam.id, &dev.name).await.unwrap());
    assert_eq!(sandboxes.default_id(adam.id).await.unwrap(), None);
    assert_eq!(sandboxes.list(adam.id).await.unwrap(), vec![main]);
    assert_eq!(sandboxes.list(eve.id).await.unwrap(), vec![eves_main]);
}

pub async fn sandbox_limit_holds_under_concurrent_inserts<R>(
    users: &dyn UserRepo,
    sandboxes: Arc<R>,
) where
    R: SandboxRepo + 'static,
{
    const MAX: usize = 3;
    let adam = seed_user(users, "adam").await;
    let mut inserts = JoinSet::new();
    for n in 0..4 * MAX {
        let sandboxes = sandboxes.clone();
        let sandbox = new_sandbox(adam.id, &format!("box-{n}"), 1);
        inserts.spawn(async move { sandboxes.insert(&sandbox, MAX).await });
    }
    let (mut admitted, mut refused) = (0, 0);
    while let Some(outcome) = inserts.join_next().await {
        match outcome.unwrap() {
            Ok(()) => admitted += 1,
            Err(Error::LimitReached(_)) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((admitted, refused), (MAX, 3 * MAX));
    assert_eq!(sandboxes.list(adam.id).await.unwrap().len(), MAX);
}

pub async fn local_account_repo(repo: &dyn LocalAccountRepo) {
    let prefix = unique("acct");
    let account = |suffix: &str, role: Role, created: i64| LocalAccount {
        username: handle(&format!("{prefix}{suffix}")),
        password_hash: unique("hash"),
        role,
        created_at: at(created),
    };
    // Byte order puts '-' before digits, unlike most locale collations.
    let hyphen = account("-b", Role::Admin, 2);
    let digit = account("1", Role::Member, 1);
    repo.insert(&digit).await.expect("insert account");
    repo.insert(&hyphen).await.expect("insert account");

    assert_eq!(
        repo.get(&hyphen.username).await.unwrap(),
        Some(hyphen.clone())
    );
    assert_eq!(
        repo.get(&digit.username).await.unwrap(),
        Some(digit.clone())
    );
    assert_eq!(repo.get(&handle(&unique("nobody"))).await.unwrap(), None);
    let ours = |accounts: Vec<LocalAccount>| -> Vec<LocalAccount> {
        accounts
            .into_iter()
            .filter(|a| a.username.as_str().starts_with(&prefix))
            .collect()
    };
    assert_eq!(
        ours(repo.list().await.unwrap()),
        vec![hyphen.clone(), digit.clone()]
    );

    let duplicate = LocalAccount {
        password_hash: unique("other"),
        role: Role::Member,
        ..hyphen.clone()
    };
    assert!(matches!(
        repo.insert(&duplicate).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        repo.get(&hyphen.username).await.unwrap(),
        Some(hyphen.clone())
    );

    let updated = LocalAccount {
        password_hash: unique("rehashed"),
        role: Role::Member,
        created_at: at(3),
        ..hyphen.clone()
    };
    repo.update(&updated).await.expect("update account");
    assert_eq!(
        repo.get(&hyphen.username).await.unwrap(),
        Some(updated.clone())
    );
    assert_eq!(
        repo.get(&digit.username).await.unwrap(),
        Some(digit.clone())
    );
    let absent = account("-ghost", Role::Member, 4);
    assert!(matches!(
        repo.update(&absent).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(repo.get(&absent.username).await.unwrap(), None);

    assert!(repo.delete(&updated.username).await.unwrap());
    assert!(!repo.delete(&updated.username).await.unwrap());
    assert_eq!(repo.get(&updated.username).await.unwrap(), None);
    assert_eq!(ours(repo.list().await.unwrap()), vec![digit]);
}

fn handle(value: &str) -> Handle {
    Handle::parse(value).expect("valid handle")
}

fn sandbox_name(value: &str) -> SandboxName {
    SandboxName::parse(value).expect("valid sandbox name")
}

fn new_key(user_id: UserId, created: i64) -> SshKey {
    let id = Uuid::new_v4();
    SshKey {
        id,
        user_id,
        label: unique("laptop"),
        public_key: public_key(id),
        created_at: at(created),
    }
}

fn public_key(seed: Uuid) -> SshPublicKey {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(seed.as_bytes());
    let key = PublicKey::new(KeyData::Ed25519(Ed25519PublicKey(bytes)), "");
    SshPublicKey::parse(&key.to_openssh().expect("encodable key")).expect("valid key")
}

fn new_sandbox(owner: UserId, name: &str, created: i64) -> Sandbox {
    Sandbox {
        id: SandboxId(unique("sbx")),
        owner,
        name: sandbox_name(name),
        image: "default".to_owned(),
        size: "small".to_owned(),
        created_at: at(created),
    }
}

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
