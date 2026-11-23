use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use ssh_key::PublicKey;
use ssh_key::public::{Ed25519PublicKey, KeyData};
use uuid::Uuid;

use super::{at, new_user, seed_user, unique};
use crate::Error;
use crate::domain::{
    Handle, ImageKey, Role, Sandbox, SandboxDefaults, SandboxId, SandboxName, SizeKey, SshKey,
    SshPublicKey, TokenHash, User, UserId, UserStatus,
};
use crate::ports::{KeyRepo, SandboxRepo, UserRepo};

pub async fn every_repo(users: &dyn UserRepo, keys: &dyn KeyRepo, sandboxes: &dyn SandboxRepo) {
    user_repo(users).await;
    sandbox_defaults(users).await;
    key_repo(users, keys).await;
    sandbox_repo(users, sandboxes).await;
    sandbox_tokens(users, sandboxes).await;
    scheduled_deletions(users, sandboxes).await;
    environment_change_times(users, sandboxes).await;
    sandbox_limit_holds_under_concurrent_inserts(users, sandboxes).await;
    deleting_a_user_leaves_nothing_of_them(users, keys, sandboxes).await;
}

async fn user_repo(repo: &dyn UserRepo) {
    let older = new_user("adam", at(1));
    let newer = User {
        email: None,
        picture: None,
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
            matches!(
                repo.insert(&duplicate).await,
                Err(Error::Conflict(message))
                    if message == format!("user {} already exists", duplicate.handle)
            ),
            "{duplicate:?}"
        );
        assert_ne!(repo.get(duplicate.id).await.unwrap(), Some(duplicate));
    }

    let updated = User {
        handle: handle(&unique("renamed")),
        email: None,
        picture: None,
        role: Role::Admin,
        status: UserStatus::Deleting,
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
        Err(Error::Conflict(message)) if message == format!("user {} already exists", newer.handle)
    ));
    assert_eq!(repo.get(older.id).await.unwrap(), Some(updated));

    let absent = new_user("ghost", at(3));
    assert!(matches!(
        repo.update(&absent).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(repo.get(absent.id).await.unwrap(), None);
}

async fn sandbox_defaults(repo: &dyn UserRepo) {
    let adam = seed_user(repo, "adam").await;
    let eve = seed_user(repo, "eve").await;
    assert_eq!(repo.sandbox_defaults(adam.id).await.unwrap(), None);
    let picked = defaults("tools", "large");
    repo.set_sandbox_defaults(adam.id, &defaults("base", "small"))
        .await
        .expect("set defaults");
    repo.set_sandbox_defaults(adam.id, &picked)
        .await
        .expect("change defaults");
    let ghost = UserId(Uuid::new_v4());
    repo.set_sandbox_defaults(ghost, &picked)
        .await
        .expect("set defaults for nobody");
    repo.update(&User {
        email: None,
        ..adam.clone()
    })
    .await
    .expect("update the user");
    assert_eq!(repo.sandbox_defaults(adam.id).await.unwrap(), Some(picked));
    assert_eq!(repo.sandbox_defaults(eve.id).await.unwrap(), None);
    assert_eq!(repo.sandbox_defaults(ghost).await.unwrap(), None);
}

async fn key_repo(users: &dyn UserRepo, keys: &dyn KeyRepo) {
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
            matches!(
                keys.insert(&duplicate).await,
                Err(Error::Conflict(message)) if message == format!(
                    "key {} already registered",
                    duplicate.public_key.fingerprint()
                )
            ),
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

async fn sandbox_repo(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo) {
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
                Err(Error::Conflict(message))
                    if message == format!("sandbox {} already exists", duplicate.name)
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

    let (token, nothing) = (TokenHash::of(&unique("token")), SandboxId(unique("sbx")));
    for (id, second) in [(&main.id, 7), (&main.id, 8), (&nothing, 8)] {
        let started = sandboxes.record_start(id, at(second), &token).await;
        started.expect("record a start, of nothing too");
    }
    let main = Sandbox {
        started_at: Some(at(8)),
        ..main
    };
    assert_eq!(
        sandboxes.by_name(adam.id, &main.name).await.unwrap(),
        Some(main.clone())
    );
    assert_eq!(
        sandboxes.list(adam.id).await.unwrap(),
        vec![main.clone(), dev.clone()]
    );
    assert!(sandboxes.list_all().await.unwrap().contains(&main));
    assert_eq!(
        sandboxes.by_name(eve.id, &eves_main.name).await.unwrap(),
        Some(eves_main.clone())
    );

    assert!(!sandboxes.delete(eve.id, &dev.name).await.unwrap());
    assert!(sandboxes.delete(adam.id, &dev.name).await.unwrap());
    assert!(!sandboxes.delete(adam.id, &dev.name).await.unwrap());
    assert_eq!(sandboxes.default_id(adam.id).await.unwrap(), None);
    assert_eq!(sandboxes.list(adam.id).await.unwrap(), vec![main]);
    assert_eq!(sandboxes.list(eve.id).await.unwrap(), vec![eves_main]);
}

async fn environment_change_times(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo) {
    let adam = seed_user(users, "adam").await;
    let (main, dev, other) = (
        new_sandbox(adam.id, "main", 1),
        new_sandbox(adam.id, "dev", 2),
        new_sandbox(adam.id, "other", 3),
    );
    for sandbox in [&main, &dev, &other] {
        sandboxes.insert(sandbox, 5).await.expect("insert sandbox");
    }
    let nothing = SandboxId(unique("sbx"));
    sandboxes
        .record_environment_change(&[main.id.clone(), dev.id.clone(), nothing], at(5))
        .await
        .expect("record a change, of nothing too");
    sandboxes
        .record_environment_change(std::slice::from_ref(&dev.id), at(6))
        .await
        .expect("record a later change");
    sandboxes
        .record_environment_change(&[], at(7))
        .await
        .expect("record a change of no sandbox");
    let changed: Vec<(String, Option<DateTime<Utc>>)> = sandboxes
        .list(adam.id)
        .await
        .unwrap()
        .into_iter()
        .map(|sandbox| (sandbox.name.to_string(), sandbox.environment_changed_at))
        .collect();
    assert_eq!(
        changed,
        [
            ("main".to_owned(), Some(at(5))),
            ("dev".to_owned(), Some(at(6))),
            ("other".to_owned(), None)
        ]
    );
}

async fn sandbox_tokens(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let main = new_sandbox(adam.id, "main", 1);
    let dev = new_sandbox(adam.id, "dev", 2);
    let eves = new_sandbox(eve.id, "main", 3);
    for sandbox in [&main, &dev, &eves] {
        sandboxes.insert(sandbox, 5).await.expect("insert sandbox");
    }
    let token = |label: &str| TokenHash::of(&unique(label));
    let (first, second, theirs) = (token("first"), token("second"), token("theirs"));
    sandboxes
        .record_start(&main.id, at(7), &first)
        .await
        .unwrap();
    sandboxes
        .record_start(&eves.id, at(7), &theirs)
        .await
        .unwrap();
    sandboxes
        .record_start(&main.id, at(8), &second)
        .await
        .unwrap();
    let holder = async |token: &TokenHash| {
        sandboxes
            .by_token(token)
            .await
            .unwrap()
            .map(|sandbox| (sandbox.id, sandbox.started_at))
    };

    assert_eq!(holder(&second).await, Some((main.id.clone(), Some(at(8)))));
    assert_eq!(holder(&theirs).await, Some((eves.id.clone(), Some(at(7)))));
    assert_eq!(holder(&first).await, None, "a start replaces the token");
    assert_eq!(holder(&token("never")).await, None);

    sandboxes.revoke_token(&main.id).await.unwrap();
    sandboxes.revoke_token(&main.id).await.unwrap();
    assert_eq!(holder(&second).await, None);
    assert_eq!(holder(&theirs).await, Some((eves.id.clone(), Some(at(7)))));

    sandboxes
        .record_start(&dev.id, at(9), &first)
        .await
        .unwrap();
    assert_eq!(holder(&first).await, Some((dev.id.clone(), Some(at(9)))));
    assert!(sandboxes.delete(adam.id, &dev.name).await.unwrap());
    assert_eq!(
        holder(&first).await,
        None,
        "a deleted sandbox's token goes too"
    );
}

async fn scheduled_deletions(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo) {
    let adam = seed_user(users, "adam").await;
    assert!(!users.deletes_from_inside(adam.id).await.unwrap());
    users.set_deletes_from_inside(adam.id, true).await.unwrap();
    assert!(users.deletes_from_inside(adam.id).await.unwrap());
    users
        .set_deletes_from_inside(UserId(Uuid::new_v4()), true)
        .await
        .expect("allow it for nobody");
    users.set_deletes_from_inside(adam.id, false).await.unwrap();
    assert!(!users.deletes_from_inside(adam.id).await.unwrap());

    let (old, new, kept) = (
        new_sandbox(adam.id, "old", 1),
        new_sandbox(adam.id, "new", 2),
        new_sandbox(adam.id, "kept", 3),
    );
    for sandbox in [&new, &kept, &old] {
        sandboxes.insert(sandbox, 5).await.expect("insert sandbox");
    }
    sandboxes
        .set_delete_at(&new.id, Some(at(20)))
        .await
        .unwrap();
    sandboxes
        .set_delete_at(&old.id, Some(at(10)))
        .await
        .unwrap();
    sandboxes
        .set_delete_at(&kept.id, Some(at(10)))
        .await
        .unwrap();
    sandboxes.set_delete_at(&kept.id, None).await.unwrap();
    sandboxes
        .set_delete_at(&SandboxId(unique("sbx")), Some(at(0)))
        .await
        .expect("schedule nothing");
    let due = async |seconds: i64| -> Vec<(String, Option<DateTime<Utc>>)> {
        let swept = sandboxes.due_for_deletion(at(seconds)).await.unwrap();
        swept
            .into_iter()
            .filter(|sandbox| sandbox.owner == adam.id)
            .map(|sandbox| (sandbox.name.to_string(), sandbox.delete_at))
            .collect()
    };

    assert_eq!(due(9).await, []);
    assert_eq!(due(10).await, [("old".to_owned(), Some(at(10)))]);
    assert_eq!(
        due(30).await,
        [
            ("old".to_owned(), Some(at(10))),
            ("new".to_owned(), Some(at(20)))
        ]
    );
    assert_eq!(
        sandboxes
            .by_name(adam.id, &new.name)
            .await
            .unwrap()
            .unwrap()
            .delete_at,
        Some(at(20))
    );
    assert_eq!(
        sandboxes
            .by_name(adam.id, &kept.name)
            .await
            .unwrap()
            .unwrap()
            .delete_at,
        None
    );
}

async fn sandbox_limit_holds_under_concurrent_inserts(
    users: &dyn UserRepo,
    sandboxes: &dyn SandboxRepo,
) {
    const MAX: usize = 3;
    let adam = seed_user(users, "adam").await;
    let candidates: Vec<Sandbox> = (0..4 * MAX)
        .map(|n| new_sandbox(adam.id, &format!("box-{n}"), 1))
        .collect();
    let outcomes = join_all(
        candidates
            .iter()
            .map(|sandbox| sandboxes.insert(sandbox, MAX)),
    )
    .await;
    let (mut admitted, mut refused) = (0, 0);
    for outcome in outcomes {
        match outcome {
            Ok(()) => admitted += 1,
            Err(Error::LimitReached(_)) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((admitted, refused), (MAX, 3 * MAX));
    assert_eq!(sandboxes.list(adam.id).await.unwrap().len(), MAX);
}

/// Deletes the way `Accounts` does, sandboxes and keys before the user, and returns the user.
pub async fn deleting_a_user_leaves_nothing_of_them(
    users: &dyn UserRepo,
    keys: &dyn KeyRepo,
    sandboxes: &dyn SandboxRepo,
) -> User {
    let gone = seed_user(users, "gone").await;
    let kept = seed_user(users, "kept").await;
    let key = new_key(gone.id, 1);
    keys.insert(&key).await.expect("insert key");
    keys.insert(&new_key(kept.id, 1)).await.expect("insert key");
    let theirs = [
        new_sandbox(gone.id, "main", 1),
        new_sandbox(gone.id, "dev", 2),
    ];
    for sandbox in &theirs {
        sandboxes.insert(sandbox, 5).await.expect("insert sandbox");
    }
    sandboxes
        .insert(&new_sandbox(kept.id, "main", 1), 5)
        .await
        .expect("insert sandbox");
    sandboxes
        .set_default(gone.id, &theirs[1].id)
        .await
        .expect("set default");
    users
        .set_sandbox_defaults(gone.id, &defaults("tools", "large"))
        .await
        .expect("set sandbox defaults");

    for sandbox in &theirs {
        assert!(sandboxes.delete(gone.id, &sandbox.name).await.unwrap());
    }
    assert!(keys.delete(gone.id, key.id).await.unwrap());
    assert!(users.delete(gone.id).await.unwrap());
    assert!(!users.delete(gone.id).await.unwrap());

    assert_eq!(users.get(gone.id).await.unwrap(), None);
    assert_eq!(users.by_subject(&gone.subject).await.unwrap(), None);
    assert_eq!(users.by_handle(&gone.handle).await.unwrap(), None);
    assert!(!users.list().await.unwrap().contains(&gone));
    assert_eq!(keys.list(gone.id).await.unwrap(), []);
    assert_eq!(keys.by_public_key(&key.public_key).await.unwrap(), None);
    assert_eq!(sandboxes.list(gone.id).await.unwrap(), []);
    assert_eq!(sandboxes.default_id(gone.id).await.unwrap(), None);
    assert_eq!(users.get(kept.id).await.unwrap(), Some(kept.clone()));
    assert_eq!(keys.list(kept.id).await.unwrap().len(), 1);
    assert_eq!(sandboxes.list(kept.id).await.unwrap().len(), 1);

    let again = User {
        id: UserId(Uuid::new_v4()),
        ..gone.clone()
    };
    users
        .insert(&again)
        .await
        .expect("reuse the subject and handle");
    keys.insert(&SshKey {
        id: Uuid::new_v4(),
        user_id: again.id,
        ..key
    })
    .await
    .expect("reuse the key");
    assert_eq!(sandboxes.list(again.id).await.unwrap(), []);
    assert_eq!(users.sandbox_defaults(again.id).await.unwrap(), None);
    gone
}

fn handle(value: &str) -> Handle {
    Handle::parse(value).expect("valid handle")
}

fn sandbox_name(value: &str) -> SandboxName {
    SandboxName::parse(value).expect("valid sandbox name")
}

fn defaults(image: &str, size: &str) -> SandboxDefaults {
    SandboxDefaults {
        image: ImageKey::parse(image).unwrap(),
        size: SizeKey::parse(size).unwrap(),
    }
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

pub(super) fn new_sandbox(owner: UserId, name: &str, created: i64) -> Sandbox {
    Sandbox {
        id: SandboxId(unique("sbx")),
        owner,
        name: sandbox_name(name),
        image: ImageKey::parse("default").unwrap(),
        size: SizeKey::parse("small").unwrap(),
        created_at: at(created),
        started_at: None,
        environment_changed_at: None,
        delete_at: None,
    }
}
