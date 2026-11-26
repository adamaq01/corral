use uuid::Uuid;

use super::repos::new_sandbox;
use super::{at, seed_user};
use crate::domain::{
    Attachment, Entry, EntryId, Glimpse, HomePath, Kept, Sandbox, SecretId, SecretRef, StoredValue,
    Target, UserId, VariableName,
};
use crate::ports::{SandboxRepo, SetupRepo, UserRepo};

pub async fn setup_repo(users: &dyn UserRepo, sandboxes: &dyn SandboxRepo, setup: &dyn SetupRepo) {
    entries_round_trip_per_owner(users, setup).await;
    update_changes_an_entry_in_place(users, sandboxes, setup).await;
    attachments_hold_any_override(users, sandboxes, setup).await;
    deleting_an_entry_detaches_it(users, sandboxes, setup).await;
    fresh_users_await_no_adoption(users, setup).await;
}

fn plain(content: &[u8], second: i64) -> StoredValue {
    StoredValue {
        kept: Kept::Plain(content.to_vec()),
        set_at: at(second),
    }
}

fn secret(size: usize, glimpse: Option<Glimpse>, second: i64) -> StoredValue {
    StoredValue {
        kept: Kept::Secret(SecretRef {
            id: SecretId::generate(),
            size,
            glimpse,
        }),
        set_at: at(second),
    }
}

fn entry(owner: UserId, target: Target, value: StoredValue, created: i64) -> Entry {
    Entry {
        id: EntryId::generate(),
        owner,
        target,
        template: false,
        executable: false,
        apply_to_new: true,
        value,
        created_at: at(created),
    }
}

fn variable(name: &str) -> Target {
    Target::Variable(VariableName::parse(name).unwrap())
}

fn file(path: &str) -> Target {
    Target::File(HomePath::parse(path).unwrap())
}

fn sorted(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by_key(|entry| entry.id);
    entries
}

fn sorted_attachments(mut attachments: Vec<Attachment>) -> Vec<Attachment> {
    attachments.sort_by(|a, b| (&a.sandbox, a.entry).cmp(&(&b.sandbox, b.entry)));
    attachments
}

async fn sandbox(sandboxes: &dyn SandboxRepo, owner: UserId, name: &str) -> Sandbox {
    let sandbox = new_sandbox(owner, name, 1);
    sandboxes
        .insert(&sandbox, 10)
        .await
        .expect("insert sandbox");
    sandbox
}

async fn entries_round_trip_per_owner(users: &dyn UserRepo, setup: &dyn SetupRepo) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let adams = vec![
        entry(adam.id, variable("PLAIN"), plain(b"shown", 1), 1),
        entry(
            adam.id,
            variable("SECRET"),
            secret(
                40,
                Some(Glimpse {
                    length: 40,
                    ends: Some(("ghp_".to_owned(), "Xz".to_owned())),
                }),
                2,
            ),
            2,
        ),
        entry(
            adam.id,
            variable("SHORT"),
            secret(
                7,
                Some(Glimpse {
                    length: 7,
                    ends: None,
                }),
                2,
            ),
            2,
        ),
        Entry {
            template: true,
            apply_to_new: false,
            ..entry(adam.id, file(".gitconfig"), plain(b"{{ PLAIN }}", 3), 3)
        },
        Entry {
            executable: true,
            ..entry(adam.id, file("bin/tool"), plain(b"#!/bin/sh", 3), 3)
        },
        entry(
            adam.id,
            variable("http_proxy"),
            plain(b"http://proxy", 3),
            3,
        ),
        entry(adam.id, file("a dir/key"), plain(&[0, 255, 10], 4), 4),
        entry(adam.id, file(".ssh/id"), secret(411, None, 5), 5),
        entry(adam.id, file("empty"), plain(b"", 6), 6),
    ];
    for entry in &adams {
        setup.insert(entry).await.expect("insert entry");
    }
    let eves = entry(eve.id, variable("PLAIN"), plain(b"eves", 1), 1);
    setup.insert(&eves).await.expect("insert entry");

    assert_eq!(sorted(setup.entries(adam.id).await.unwrap()), sorted(adams));
    assert_eq!(setup.entries(eve.id).await.unwrap(), [eves]);
    assert_eq!(setup.entries(UserId(Uuid::new_v4())).await.unwrap(), []);
}

async fn update_changes_an_entry_in_place(
    users: &dyn UserRepo,
    sandboxes: &dyn SandboxRepo,
    setup: &dyn SetupRepo,
) {
    let adam = seed_user(users, "adam").await;
    let (main, dev) = (
        sandbox(sandboxes, adam.id, "main").await,
        sandbox(sandboxes, adam.id, "dev").await,
    );
    let original = entry(adam.id, file("notes"), plain(b"one", 1), 1);
    setup.insert(&original).await.unwrap();
    let attach = |sandbox: &Sandbox, overriding: Option<StoredValue>| Attachment {
        sandbox: sandbox.id.clone(),
        entry: original.id,
        overriding,
    };
    setup
        .attach(&attach(&main, Some(plain(b"main's", 1))))
        .await
        .unwrap();
    setup.attach(&attach(&dev, None)).await.unwrap();

    let changed = Entry {
        target: variable("notes"),
        template: true,
        executable: true,
        apply_to_new: false,
        value: secret(3, None, 2),
        ..original.clone()
    };
    let moved = attach(&main, Some(secret(6, None, 2)));
    let recreated = Entry {
        created_at: at(9),
        ..changed.clone()
    };
    assert!(
        setup
            .update(&recreated, std::slice::from_ref(&moved))
            .await
            .unwrap()
    );
    assert_eq!(
        setup.entries(adam.id).await.unwrap(),
        std::slice::from_ref(&changed)
    );
    assert_eq!(
        sorted_attachments(setup.attachments(adam.id).await.unwrap()),
        sorted_attachments(vec![moved, attach(&dev, None)])
    );

    let stranger = Entry {
        owner: UserId(Uuid::new_v4()),
        ..changed.clone()
    };
    assert!(!setup.update(&stranger, &[]).await.unwrap());
    let missing = Entry {
        id: EntryId::generate(),
        ..changed
    };
    assert!(!setup.update(&missing, &[]).await.unwrap());
}

async fn attachments_hold_any_override(
    users: &dyn UserRepo,
    sandboxes: &dyn SandboxRepo,
    setup: &dyn SetupRepo,
) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let (main, dev) = (
        sandbox(sandboxes, adam.id, "main").await,
        sandbox(sandboxes, adam.id, "dev").await,
    );
    let eves_main = sandbox(sandboxes, eve.id, "main").await;
    let token = entry(adam.id, variable("TOKEN"), secret(5, None, 1), 1);
    let notes = entry(adam.id, file("notes"), plain(b"n", 1), 1);
    let eves = entry(eve.id, variable("TOKEN"), plain(b"e", 1), 1);
    for entry in [&token, &notes, &eves] {
        setup.insert(entry).await.unwrap();
    }
    let attach = |sandbox: &Sandbox, entry: &Entry, overriding: Option<StoredValue>| Attachment {
        sandbox: sandbox.id.clone(),
        entry: entry.id,
        overriding,
    };
    let overridden = attach(&dev, &token, Some(secret(9, None, 2)));
    let plainly = attach(&main, &notes, Some(plain(&[1, 2], 2)));
    for attachment in [
        attach(&main, &token, None),
        attach(&dev, &token, None),
        overridden.clone(),
        attach(&main, &notes, None),
        plainly.clone(),
        attach(&eves_main, &eves, None),
    ] {
        setup.attach(&attachment).await.expect("attach");
    }
    assert_eq!(
        sorted_attachments(setup.attachments(adam.id).await.unwrap()),
        sorted_attachments(vec![
            attach(&main, &token, None),
            overridden.clone(),
            plainly.clone()
        ])
    );
    assert_eq!(
        setup.attachments(eve.id).await.unwrap(),
        [attach(&eves_main, &eves, None)]
    );

    assert!(setup.detach(&main.id, token.id).await.unwrap());
    assert!(!setup.detach(&main.id, token.id).await.unwrap());
    assert!(!setup.detach(&eves_main.id, token.id).await.unwrap());
    setup
        .attach(&attach(&main, &notes, None))
        .await
        .expect("clear the override");
    assert_eq!(
        sorted_attachments(setup.attachments(adam.id).await.unwrap()),
        sorted_attachments(vec![overridden, attach(&main, &notes, None)])
    );
}

async fn deleting_an_entry_detaches_it(
    users: &dyn UserRepo,
    sandboxes: &dyn SandboxRepo,
    setup: &dyn SetupRepo,
) {
    let adam = seed_user(users, "adam").await;
    let main = sandbox(sandboxes, adam.id, "main").await;
    let (gone, kept) = (
        entry(adam.id, variable("GONE"), plain(b"g", 1), 1),
        entry(adam.id, variable("KEPT"), plain(b"k", 1), 1),
    );
    for entry in [&gone, &kept] {
        setup.insert(entry).await.unwrap();
        setup
            .attach(&Attachment {
                sandbox: main.id.clone(),
                entry: entry.id,
                overriding: Some(plain(b"o", 2)),
            })
            .await
            .unwrap();
    }
    assert!(!setup.delete(UserId(Uuid::new_v4()), gone.id).await.unwrap());
    assert!(setup.delete(adam.id, gone.id).await.unwrap());
    assert!(!setup.delete(adam.id, gone.id).await.unwrap());
    assert_eq!(
        setup.entries(adam.id).await.unwrap(),
        std::slice::from_ref(&kept)
    );
    let left: Vec<EntryId> = setup
        .attachments(adam.id)
        .await
        .unwrap()
        .into_iter()
        .map(|attachment| attachment.entry)
        .collect();
    assert_eq!(left, [kept.id]);
}

async fn fresh_users_await_no_adoption(users: &dyn UserRepo, setup: &dyn SetupRepo) {
    let adam = seed_user(users, "adam").await;
    assert!(!setup.awaiting_adoption().await.unwrap().contains(&adam.id));
    setup.adopted(adam.id).await.expect("adopt nobody");
    setup
        .adopted(UserId(Uuid::new_v4()))
        .await
        .expect("adopt a stranger");
}
