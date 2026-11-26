use std::collections::BTreeSet;

use chrono::Utc;
use secrecy::{ExposeSecret, SecretSlice};
use serde_json::{Value, json};

use super::{
    EntryChange, FILE_LIMIT, NewEntry, NewSharedEntry, Preview, SharedEntryChange, Use,
    VARIABLE_LIMIT,
};
use crate::Error;
use crate::app::{CreateSandbox, SANDBOX_FILE};
use crate::domain::{
    Entry, EntryId, Glimpse, HomePath, Kept, Repository, Sandbox, SandboxName, SandboxToken,
    SecretId, Shared, Target, User, UserId,
};
use crate::fakes::{
    World, admin_proof, bytes, entry, exposed, file, request, sandbox_name, shared_file,
    shared_variable, unchanged, variable, variable_name,
};
use crate::ports::{SecretStore, SetupRepo};

async fn attached(w: &World, owner: &User, sandbox: &Sandbox) -> Vec<String> {
    w.setup
        .of_sandbox(owner, &sandbox.name)
        .await
        .unwrap()
        .into_iter()
        .map(|attached| attached.view.entry.target.to_string())
        .collect()
}

/// What the sandbox's sessions and home get at a restart.
async fn restarted(w: &World, owner: &User, sandbox: &Sandbox) -> (Vec<String>, Vec<String>) {
    w.sandboxes.restart(owner, &sandbox.name).await.unwrap();
    let files = w
        .engine
        .files_of(&sandbox.id)
        .unwrap()
        .into_iter()
        .filter(|(path, _)| path.as_str() != SANDBOX_FILE)
        .map(|(path, file)| {
            format!(
                "{path} {:o} {}",
                file.mode.bits(),
                String::from_utf8_lossy(file.content.expose_secret())
            )
        })
        .collect();
    (exposed(&w.engine.env_of(&sandbox.id).unwrap()), files)
}

fn refused(result: crate::Result<impl std::fmt::Debug>) -> Error {
    match result {
        Err(error) => error,
        Ok(done) => panic!("accepted: {done:?}"),
    }
}

fn refused_saying(result: crate::Result<impl std::fmt::Debug>, parts: &[&str]) -> Error {
    let error = refused(result);
    let message = error.to_string();
    for part in parts {
        assert!(message.contains(part), "{part:?} not in {message}");
    }
    error
}

fn names(names: &[&str]) -> BTreeSet<crate::domain::VariableName> {
    names.iter().map(|name| variable_name(name)).collect()
}

#[tokio::test]
async fn a_new_sandbox_gets_each_entry_that_applies_to_new_sandboxes() {
    let w = World::new();
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    let gh = w.entry(&adam, variable("GH_TOKEN"), "ghp_1", true).await;
    let notes = w.entry(&adam, file("notes.txt"), "hello", false).await;
    w.setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(variable("OPTIONAL"), "x")
            },
        )
        .await
        .unwrap();
    w.entry(&eve, variable("EVES"), "e", false).await;

    let main = w.sandbox(&adam, "main").await;
    assert_eq!(
        attached(&w, &adam, &main).await,
        ["variable GH_TOKEN", "file notes.txt"]
    );
    assert_eq!(
        exposed(&w.engine.env_of(&main.id).unwrap()),
        ["GH_TOKEN=ghp_1"]
    );

    w.setup
        .change(
            &adam,
            notes.id,
            EntryChange {
                apply_to_new: false,
                ..unchanged(&notes)
            },
        )
        .await
        .unwrap();
    let later = w.entry(&adam, variable("LATER"), "l", false).await;
    assert_eq!(
        attached(&w, &adam, &main).await,
        ["variable GH_TOKEN", "file notes.txt"],
        "flags and new entries reach only sandboxes created afterwards"
    );
    let dev = w.sandbox(&adam, "dev").await;
    assert_eq!(
        attached(&w, &adam, &dev).await,
        ["variable GH_TOKEN", "variable LATER"]
    );

    let listed: Vec<(String, Vec<SandboxName>)> = w
        .setup
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| {
            let uses = view.sandboxes.into_iter().map(|u| u.sandbox).collect();
            (view.entry.target.to_string(), uses)
        })
        .collect();
    assert_eq!(
        listed,
        [
            (
                "variable GH_TOKEN".to_owned(),
                vec![dev.name.clone(), main.name.clone()]
            ),
            ("variable LATER".to_owned(), vec![dev.name.clone()]),
            ("variable OPTIONAL".to_owned(), vec![]),
            ("file notes.txt".to_owned(), vec![main.name.clone()]),
        ]
    );
    assert_eq!(w.setup.get(&adam, gh.id).await.unwrap().sandboxes.len(), 2);
    for missing in [later.id, EntryId::generate()] {
        assert!(matches!(
            w.setup.get(&eve, missing).await,
            Err(Error::NotFound(_))
        ));
    }
}

#[tokio::test]
async fn one_entry_per_target_and_colliding_files_never_meet() {
    let w = World::new();
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    w.entry(&adam, variable("GH_TOKEN"), "work", false).await;
    w.entry(&eve, variable("GH_TOKEN"), "eve's", false).await;
    let inner = w.entry(&adam, file("tools/a"), "dir", false).await;
    for (again, also) in [
        (variable("GH_TOKEN"), "override it there"),
        (file("~/tools/a"), "already have file tools/a"),
    ] {
        let error = refused_saying(
            w.setup
                .create(
                    &adam,
                    NewEntry {
                        apply_to_new: false,
                        ..entry(again.clone(), "x")
                    },
                )
                .await,
            &["you already have", also],
        );
        assert!(matches!(error, Error::Conflict(_)), "{again}");
    }
    w.entry(&adam, variable("gh_token"), "lower", false).await;
    for collides in [file("tools"), file("tools/a/b")] {
        refused_saying(
            w.setup.create(&adam, entry(collides, "x")).await,
            &["cannot both be attached to new sandboxes"],
        );
    }
    let outer = w
        .setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(file("tools"), "x")
            },
        )
        .await
        .unwrap()
        .entry;
    refused_saying(
        w.setup
            .change(
                &adam,
                outer.id,
                EntryChange {
                    apply_to_new: true,
                    ..unchanged(&outer)
                },
            )
            .await,
        &["new sandboxes"],
    );
    let renamed = EntryChange {
        target: file("tools/a"),
        ..unchanged(&outer)
    };
    refused_saying(
        w.setup.change(&adam, outer.id, renamed).await,
        &["already have file tools/a"],
    );

    let main = w.sandbox(&adam, "main").await;
    let error = refused_saying(
        w.setup.attach(&adam, &main.name, outer.id).await,
        &["file tools/a", "sandbox main"],
    );
    assert!(matches!(error, Error::Conflict(_)));
    w.setup.detach(&adam, &main.name, inner.id).await.unwrap();
    for _ in 0..2 {
        w.setup.attach(&adam, &main.name, outer.id).await.unwrap();
    }
    assert_eq!(
        restarted(&w, &adam, &main).await,
        (
            vec!["GH_TOKEN=work".to_owned(), "gh_token=lower".to_owned()],
            vec!["tools 644 x".to_owned()]
        )
    );
}

#[tokio::test]
async fn entries_made_before_targets_were_unique_are_marked_and_can_be_renamed() {
    let w = World::new();
    let adam = w.member("adam").await;
    let first = w.entry(&adam, variable("GH_TOKEN"), "first", false).await;
    let second = Entry {
        id: EntryId::generate(),
        created_at: Utc::now(),
        apply_to_new: false,
        ..first.clone()
    };
    w.setup_repo.insert(&second).await.unwrap();
    let duplicates: Vec<(EntryId, bool)> = w
        .setup
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| (view.entry.id, view.duplicate))
        .collect();
    assert_eq!(duplicates, [(first.id, false), (second.id, true)]);
    let renamed = w
        .setup
        .change(
            &adam,
            second.id,
            EntryChange {
                target: variable("GH_TOKEN_OLD"),
                ..unchanged(&second)
            },
        )
        .await
        .unwrap();
    assert!(!renamed.duplicate);
}

#[tokio::test]
async fn an_override_reaches_one_sandbox_and_stays_as_secret_as_its_entry() {
    let w = World::new();
    let adam = w.member("adam").await;
    let gh = w
        .entry(&adam, variable("GH_TOKEN"), "ghp_everyone", true)
        .await;
    let main = w.sandbox(&adam, "main").await;
    let dev = w.sandbox(&adam, "dev").await;
    w.setup
        .set_override(
            &adam,
            &main.name,
            gh.id,
            Some(bytes("ghp_main_only_0123456789")),
        )
        .await
        .unwrap();
    let held = w.setup.of_sandbox(&adam, &main.name).await.unwrap();
    let Some(Kept::Secret(secret)) = held[0].overriding.as_ref().map(|value| &value.kept) else {
        panic!("{held:?}");
    };
    assert_eq!(
        secret.glimpse,
        Some(Glimpse {
            length: 24,
            ends: Some(("ghp_".to_owned(), "89".to_owned()))
        })
    );
    let uses: Vec<(String, bool)> = held[0]
        .view
        .sandboxes
        .iter()
        .map(|u| (u.sandbox.to_string(), u.overridden))
        .collect();
    assert_eq!(uses, [("dev".to_owned(), false), ("main".to_owned(), true)]);
    assert_eq!(w.secrets.count(), 2);

    w.setup
        .change(
            &adam,
            gh.id,
            EntryChange {
                value: Some(bytes("ghp_rotated")),
                ..unchanged(&gh)
            },
        )
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 2, "the replaced value left the store");
    assert_eq!(
        restarted(&w, &adam, &main).await.0,
        ["GH_TOKEN=ghp_main_only_0123456789"]
    );
    assert_eq!(restarted(&w, &adam, &dev).await.0, ["GH_TOKEN=ghp_rotated"]);

    w.setup
        .set_override(&adam, &main.name, gh.id, None)
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 1);
    assert_eq!(
        restarted(&w, &adam, &main).await.0,
        ["GH_TOKEN=ghp_rotated"]
    );

    let notes = w.entry(&adam, file("notes"), "plain", false).await;
    assert!(matches!(
        w.setup
            .set_override(&adam, &main.name, notes.id, Some(bytes("x")))
            .await,
        Err(Error::NotFound(message)) if message.contains("not attached to sandbox main")
    ));
    w.setup.attach(&adam, &main.name, notes.id).await.unwrap();
    w.setup
        .set_override(&adam, &main.name, notes.id, Some(bytes("main's notes")))
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 1, "a plain entry's override is plain");
    w.setup
        .set_override(&adam, &dev.name, gh.id, Some(bytes("dev's")))
        .await
        .unwrap();
    w.sandboxes.delete(&adam, &dev.name).await.unwrap();
    assert_eq!(
        w.secrets.count(),
        1,
        "a deleted sandbox's overrides leave the store"
    );
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        ["notes 644 main's notes"]
    );
}

#[tokio::test]
async fn an_entry_changes_name_kind_and_secrecy_keeping_its_attachments_and_overrides() {
    let w = World::new();
    let adam = w.member("adam").await;
    let main = w.sandbox(&adam, "main").await;
    let dev = w.sandbox(&adam, "dev").await;
    let token = w.entry(&adam, variable("TOKEN"), "everyone", false).await;
    for sandbox in [&main, &dev] {
        w.setup
            .attach(&adam, &sandbox.name, token.id)
            .await
            .unwrap();
    }
    w.setup
        .set_override(&adam, &main.name, token.id, Some(bytes("main's")))
        .await
        .unwrap();
    let change = |entry: &Entry, edit: fn(&mut EntryChange)| {
        let mut change = unchanged(entry);
        edit(&mut change);
        change
    };

    let secret = w
        .setup
        .change(
            &adam,
            token.id,
            change(&token, |c| {
                c.target = variable("GH_TOKEN");
                c.secret = true;
            }),
        )
        .await
        .unwrap();
    assert!(secret.entry.is_secret());
    assert_eq!(secret.sandboxes.len(), 2);
    assert_eq!(w.secrets.count(), 2, "the value and the override moved");
    assert_eq!(restarted(&w, &adam, &main).await.0, ["GH_TOKEN=main's"]);
    assert_eq!(restarted(&w, &adam, &dev).await.0, ["GH_TOKEN=everyone"]);
    let held = w.setup.of_sandbox(&adam, &main.name).await.unwrap();
    assert!(held[0].overriding.as_ref().unwrap().is_secret());

    let as_file = w
        .setup
        .change(
            &adam,
            token.id,
            change(&secret.entry, |c| {
                c.target = file("~/.config/gh/token");
                c.executable = true;
            }),
        )
        .await
        .unwrap()
        .entry;
    assert_eq!(as_file.target, file(".config/gh/token"));
    assert_eq!(
        restarted(&w, &adam, &main).await,
        (vec![], vec![".config/gh/token 700 main's".to_owned()])
    );

    let plain = w
        .setup
        .change(&adam, token.id, change(&as_file, |c| c.secret = false))
        .await
        .unwrap()
        .entry;
    assert_eq!(plain.value.kept, Kept::Plain(b"everyone".to_vec()));
    assert_eq!(w.secrets.count(), 0, "stale secret copies are dropped");
    assert_eq!(
        restarted(&w, &adam, &dev).await.1,
        [".config/gh/token 755 everyone"]
    );

    w.setup
        .set_override(&adam, &main.name, token.id, Some(bytes("two\nlines")))
        .await
        .unwrap();
    let back = change(&plain, |c| {
        c.target = variable("GH_TOKEN");
        c.executable = false;
    });
    refused_saying(
        w.setup.change(&adam, token.id, back).await,
        &["override in sandbox main", "one line"],
    );
    refused_saying(
        w.setup
            .change(
                &adam,
                token.id,
                change(&plain, |c| c.target = variable("GH_TOKEN")),
            )
            .await,
        &["cannot be executable"],
    );
    assert_eq!(
        w.setup.get(&adam, token.id).await.unwrap().entry,
        plain,
        "a refused change leaves the entry as it was"
    );
}

#[tokio::test]
async fn templates_take_the_variables_attached_to_the_sandbox_and_nothing_else() {
    let w = World::new();
    let adam = w.member("adam").await;
    let name = w.entry(&adam, variable("GIT_NAME"), "Adam", false).await;
    let token = w.entry(&adam, variable("GH_TOKEN"), "ghp_x", true).await;
    let config = w
        .setup
        .create(
            &adam,
            NewEntry {
                template: true,
                ..entry(
                    file(".gitconfig"),
                    "name = {{ GIT_NAME }}\ntoken = {{GH_TOKEN}}\n{{ end }}",
                )
            },
        )
        .await
        .unwrap();
    assert_eq!(config.references, names(&["GH_TOKEN", "GIT_NAME"]));
    let config = config.entry;
    let main = w.sandbox(&adam, "main").await;
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".gitconfig 600 name = Adam\ntoken = ghp_x\n{{ end }}"],
        "a template that takes a secret is the sandbox user's alone"
    );
    assert_eq!(
        w.setup.preview(&adam, &main.name, config.id).await.unwrap(),
        Preview {
            text: "name = Adam\ntoken = ••••••••\n{{ end }}".to_owned(),
            unresolved: BTreeSet::new()
        }
    );

    refused_saying(
        w.setup.detach(&adam, &main.name, token.id).await,
        &["sandbox main", "~/.gitconfig", "GH_TOKEN"],
    );
    refused_saying(
        w.setup.delete(&adam, token.id).await,
        &["~/.gitconfig", "GH_TOKEN"],
    );
    refused_saying(
        w.setup
            .change(
                &adam,
                name.id,
                EntryChange {
                    target: variable("GIT_USER"),
                    ..unchanged(&name)
                },
            )
            .await,
        &["~/.gitconfig", "GIT_NAME"],
    );
    refused_saying(
        w.setup
            .change(
                &adam,
                name.id,
                EntryChange {
                    apply_to_new: false,
                    ..unchanged(&name)
                },
            )
            .await,
        &["new sandboxes", "GIT_NAME"],
    );
    w.setup.detach(&adam, &main.name, config.id).await.unwrap();
    w.setup.detach(&adam, &main.name, token.id).await.unwrap();
    refused_saying(
        w.setup.attach(&adam, &main.name, config.id).await,
        &["GH_TOKEN", "sandbox main"],
    );
    w.setup.attach(&adam, &main.name, token.id).await.unwrap();
    w.setup.attach(&adam, &main.name, config.id).await.unwrap();
}

#[tokio::test]
async fn a_templates_override_and_variables_reach_its_rendering() {
    let w = World::new();
    let adam = w.member("adam").await;
    let name = w.entry(&adam, variable("GIT_NAME"), "Adam", false).await;
    let config = w
        .setup
        .create(
            &adam,
            NewEntry {
                template: true,
                ..entry(file(".gitconfig"), "name = {{ GIT_NAME }}")
            },
        )
        .await
        .unwrap()
        .entry;
    let main = w.sandbox(&adam, "main").await;
    refused_saying(
        w.setup
            .set_override(&adam, &main.name, config.id, Some(bytes("{{ NOWHERE }}")))
            .await,
        &["NOWHERE"],
    );
    w.setup
        .set_override(
            &adam,
            &main.name,
            config.id,
            Some(bytes("only {{ GIT_NAME }}")),
        )
        .await
        .unwrap();
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".gitconfig 644 only Adam"]
    );
    w.setup
        .change(
            &adam,
            name.id,
            EntryChange {
                value: Some(bytes("Adam Q")),
                ..unchanged(&name)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".gitconfig 644 only Adam Q"]
    );
}

#[tokio::test]
async fn a_template_may_come_before_its_variables_but_not_reach_a_sandbox_without_them() {
    let w = World::new();
    let adam = w.member("adam").await;
    refused_saying(
        w.setup
            .create(
                &adam,
                NewEntry {
                    template: true,
                    ..entry(file(".gitconfig"), "{{ GIT_NAME }}")
                },
            )
            .await,
        &["new sandboxes", "GIT_NAME"],
    );
    let config = w
        .setup
        .create(
            &adam,
            NewEntry {
                template: true,
                apply_to_new: false,
                ..entry(file(".gitconfig"), "{{ GIT_NAME }} {{ x }} {{ .Values.y }}")
            },
        )
        .await
        .unwrap()
        .entry;
    let main = w.sandbox(&adam, "main").await;
    refused_saying(
        w.setup.attach(&adam, &main.name, config.id).await,
        &["GIT_NAME", "sandbox main"],
    );
    let git_name = w
        .setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(variable("GIT_NAME"), "Adam")
            },
        )
        .await
        .unwrap()
        .entry;
    w.setup
        .attach(&adam, &main.name, git_name.id)
        .await
        .unwrap();
    w.setup.attach(&adam, &main.name, config.id).await.unwrap();
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".gitconfig 644 Adam {{ x }} {{ .Values.y }}"]
    );

    let plain = w
        .entry(&adam, file("plain"), "\u{ff}{{ GIT_NAME }}", false)
        .await;
    let utf8 = SecretSlice::from(vec![0xff, b'{']);
    refused_saying(
        w.setup
            .change(
                &adam,
                plain.id,
                EntryChange {
                    template: true,
                    value: Some(utf8),
                    ..unchanged(&plain)
                },
            )
            .await,
        &["must be text"],
    );
}

#[tokio::test]
async fn views_tell_what_a_template_refers_to_and_what_a_sandbox_lacks() {
    let w = World::new();
    let adam = w.member("adam").await;
    let main = w.sandbox(&adam, "main").await;
    let dev = w.sandbox(&adam, "dev").await;
    let token = w
        .setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(variable("GH_TOKEN"), "t")
            },
        )
        .await
        .unwrap()
        .entry;
    let config = w
        .setup
        .create(
            &adam,
            NewEntry {
                secret: true,
                template: true,
                apply_to_new: false,
                ..entry(file("cfg"), "{{ GH_TOKEN }}")
            },
        )
        .await
        .unwrap()
        .entry;
    for sandbox in [&main, &dev] {
        w.setup
            .attach(&adam, &sandbox.name, token.id)
            .await
            .unwrap();
        w.setup
            .attach(&adam, &sandbox.name, config.id)
            .await
            .unwrap();
    }
    w.setup
        .set_override(&adam, &dev.name, config.id, Some(bytes("plain")))
        .await
        .unwrap();
    // A start refuses this state, which only a sandbox from before checks can be in.
    w.setup_repo.detach(&main.id, token.id).await.unwrap();
    let view = w.setup.get(&adam, config.id).await.unwrap();
    assert_eq!(view.references, names(&["GH_TOKEN"]));
    assert_eq!(
        view.sandboxes,
        [
            Use {
                sandbox: dev.name.clone(),
                overridden: true,
                unresolved: BTreeSet::new()
            },
            Use {
                sandbox: main.name.clone(),
                overridden: false,
                unresolved: names(&["GH_TOKEN"])
            },
        ]
    );
    refused_saying(
        w.setup.preview(&adam, &main.name, config.id).await,
        &["only a template that is not secret"],
    );
    w.setup
        .set_override(
            &adam,
            &main.name,
            config.id,
            Some(bytes("{{ GH_TOKEN }} again")),
        )
        .await
        .expect("a state already broken does not block a change that does not worsen it");
}

#[tokio::test]
async fn values_their_target_cannot_take_are_refused() {
    let w = World::new();
    let adam = w.member("adam").await;
    let longest = VARIABLE_LIMIT - "LONG".len() - 2;
    let attempts: Vec<(NewEntry, &str)> = vec![
        (entry(variable("MULTI"), "one\ntwo"), "one line"),
        (
            NewEntry {
                value: SecretSlice::from(vec![0xff, 0xfe]),
                ..entry(variable("BINARY"), "")
            },
            "one line",
        ),
        (
            NewEntry {
                template: true,
                ..entry(variable("TEMPLATED"), "x")
            },
            "only a file",
        ),
        (
            NewEntry {
                executable: true,
                ..entry(variable("RUN"), "x")
            },
            "only a file",
        ),
        (
            NewEntry {
                template: true,
                value: SecretSlice::from(vec![0xff]),
                ..entry(file("binary"), "")
            },
            "must be text",
        ),
        (
            entry(file(".config/corral/sandbox.json"), "{}"),
            "Corral keeps",
        ),
        (entry(file(".config/corral"), "{}"), "Corral keeps"),
        (entry(file(".config/corral/notes"), "{}"), "Corral keeps"),
        (
            NewEntry {
                value: SecretSlice::from(vec![0; FILE_LIMIT + 1]),
                ..entry(file("big"), "")
            },
            "256 KiB",
        ),
        (entry(variable("LONG"), &"v".repeat(longest + 1)), "128 KiB"),
    ];
    for (attempt, culprit) in attempts {
        let target = attempt.target.clone();
        let error = refused(
            w.setup
                .create(
                    &adam,
                    NewEntry {
                        secret: true,
                        ..attempt
                    },
                )
                .await,
        );
        let message = error.to_string();
        assert!(message.contains(culprit), "{target}: {message}");
        assert!(!message.contains("one\ntwo"), "{message}");
    }
    assert_eq!(w.setup.list(&adam).await.unwrap(), []);
    assert_eq!(w.secrets.count(), 0);
    w.setup
        .create(
            &adam,
            NewEntry {
                value: SecretSlice::from(vec![0; FILE_LIMIT]),
                ..entry(file("big"), "")
            },
        )
        .await
        .unwrap();
    w.entry(&adam, variable("LONG"), &"v".repeat(longest), false)
        .await;
}

#[tokio::test]
async fn a_sandbox_takes_no_more_files_than_one_mebibyte_once_staged() {
    let w = World::new();
    let adam = w.member("adam").await;
    let main = w.sandbox(&adam, "main").await;
    let full = "x".repeat(FILE_LIMIT);
    let mut entries = Vec::new();
    for index in 0..4 {
        let held = w
            .setup
            .create(
                &adam,
                NewEntry {
                    apply_to_new: false,
                    secret: index % 2 == 0,
                    ..entry(file(&format!("f{index}")), &full)
                },
            )
            .await
            .unwrap()
            .entry;
        entries.push(held.id);
    }
    for id in &entries[..3] {
        w.setup.attach(&adam, &main.name, *id).await.unwrap();
    }
    let error = refused_saying(
        w.setup.attach(&adam, &main.name, entries[3]).await,
        &["1 MiB", "sandbox main"],
    );
    assert!(matches!(error, Error::LimitReached(_)));
    assert_eq!(attached(&w, &adam, &main).await.len(), 3);
    restarted(&w, &adam, &main).await;
}

#[tokio::test]
async fn deleting_an_entry_detaches_it_everywhere_with_its_values() {
    let w = World::new();
    let adam = w.member("adam").await;
    let gh = w.entry(&adam, variable("GH_TOKEN"), "ghp", true).await;
    let main = w.sandbox(&adam, "main").await;
    let dev = w.sandbox(&adam, "dev").await;
    w.setup
        .set_override(&adam, &dev.name, gh.id, Some(bytes("dev's")))
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 2);
    w.setup.delete(&adam, gh.id).await.unwrap();
    assert_eq!(w.secrets.count(), 0);
    for sandbox in [&main, &dev] {
        assert_eq!(attached(&w, &adam, sandbox).await, [] as [&str; 0]);
        assert_eq!(restarted(&w, &adam, sandbox).await.0, [] as [&str; 0]);
    }
    assert!(matches!(
        w.setup.delete(&adam, gh.id).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn files_are_written_executable_on_request() {
    let w = World::new();
    let adam = w.member("adam").await;
    for (path, secret) in [("bin/tool", false), ("bin/secret-tool", true)] {
        w.setup
            .create(
                &adam,
                NewEntry {
                    executable: true,
                    secret,
                    ..entry(file(path), "#!/bin/sh")
                },
            )
            .await
            .unwrap();
    }
    let main = w.sandbox(&adam, "main").await;
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        ["bin/secret-tool 700 #!/bin/sh", "bin/tool 755 #!/bin/sh"]
    );
}

#[tokio::test]
async fn secret_entries_keep_their_values_in_the_store_and_show_a_glimpse() {
    let w = World::new();
    let adam = w.member("adam").await;
    let long = w
        .entry(&adam, variable("LONG"), "sk-live-0123456789abcdef", true)
        .await;
    let short = w.entry(&adam, variable("SHORT"), "hunter2", true).await;
    let key = w
        .entry(&adam, file(".ssh/id"), "-----BEGIN-----", true)
        .await;
    let shown = w.entry(&adam, variable("SHOWN"), "visible", false).await;
    let glimpse = |entry: &Entry| match &entry.value.kept {
        Kept::Secret(secret) => (secret.size, secret.glimpse.clone()),
        Kept::Plain(_) => panic!("{entry:?}"),
    };
    assert_eq!(
        glimpse(&long),
        (
            24,
            Some(Glimpse {
                length: 24,
                ends: Some(("sk-l".to_owned(), "ef".to_owned()))
            })
        )
    );
    assert_eq!(
        glimpse(&short),
        (
            7,
            Some(Glimpse {
                length: 7,
                ends: None
            })
        )
    );
    assert_eq!(glimpse(&key), (15, None));
    assert_eq!(shown.value.kept, Kept::Plain(b"visible".to_vec()));
    assert_eq!(w.secrets.count(), 3);
    let debug = format!("{:?}", w.setup.list(&adam).await.unwrap());
    assert!(
        !debug.contains("0123456789") && !debug.contains("hunter2"),
        "{debug}"
    );

    let main = w.sandbox(&adam, "main").await;
    let (env, files) = restarted(&w, &adam, &main).await;
    assert_eq!(
        env,
        [
            "LONG=sk-live-0123456789abcdef",
            "SHORT=hunter2",
            "SHOWN=visible"
        ]
    );
    assert_eq!(files, [".ssh/id 600 -----BEGIN-----"]);
    let path = HomePath::parse(".ssh/id").unwrap();
    assert!(w.engine.files_of(&main.id).unwrap().contains_key(&path));
}

#[tokio::test]
async fn credentials_kept_before_entries_become_secret_variables_on_every_sandbox() {
    let w = World::new();
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    let main = w.sandbox(&adam, "main").await;
    let dev = w.sandbox(&adam, "dev").await;
    for (name, value) in [
        ("GH_TOKEN", "ghp_0123456789abcdefghij"),
        ("SHORT", "hunter2"),
    ] {
        w.secrets.keep_legacy(adam.id, &variable_name(name), value);
    }
    w.secrets.keep_legacy(eve.id, &variable_name("EVES"), "eve");
    for user in [&adam, &eve] {
        w.setup_repo.await_adoption(user.id);
    }
    // An earlier run made GH_TOKEN and attached it to main before it stopped.
    let interrupted = w
        .entry(
            &adam,
            variable("GH_TOKEN"),
            "ghp_0123456789abcdefghij",
            true,
        )
        .await;
    w.setup
        .attach(&adam, &main.name, interrupted.id)
        .await
        .unwrap();

    assert_eq!(w.setup.adopt_credentials().await.unwrap(), 2);
    assert_eq!(w.setup.adopt_credentials().await.unwrap(), 0);
    let adopted: Vec<(Target, bool, bool, Vec<SandboxName>)> = w
        .setup
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| {
            (
                view.entry.target.clone(),
                view.entry.is_secret(),
                view.entry.apply_to_new,
                view.sandboxes.into_iter().map(|u| u.sandbox).collect(),
            )
        })
        .collect();
    let both = vec![dev.name.clone(), main.name.clone()];
    assert_eq!(
        adopted,
        [
            (variable("GH_TOKEN"), true, true, both.clone()),
            (variable("SHORT"), true, true, both),
        ]
    );
    for sandbox in [&main, &dev] {
        assert_eq!(
            restarted(&w, &adam, sandbox).await.0,
            ["GH_TOKEN=ghp_0123456789abcdefghij", "SHORT=hunter2"]
        );
    }
    assert_eq!(w.setup.list(&eve).await.unwrap().len(), 1);
    assert!(
        w.secrets
            .legacy_credentials(adam.id)
            .await
            .unwrap()
            .is_empty()
    );
    let new = w.sandbox(&adam, "new").await;
    assert_eq!(attached(&w, &adam, &new).await.len(), 2);
}

#[tokio::test]
async fn a_failed_save_leaves_no_secret_behind() {
    let w = World::new();
    let adam = w.member("adam").await;
    let main = w.sandbox(&adam, "main").await;
    let config = w
        .setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(file("c"), "c")
            },
        )
        .await
        .unwrap()
        .entry;
    w.setup.attach(&adam, &main.name, config.id).await.unwrap();
    let token = w.entry(&adam, variable("GH_TOKEN"), "applies", false).await;
    w.setup.attach(&adam, &main.name, token.id).await.unwrap();
    w.entry(&adam, variable("OTHER"), "elsewhere", false).await;
    let template = w
        .setup
        .create(
            &adam,
            NewEntry {
                secret: true,
                template: true,
                apply_to_new: false,
                ..entry(file("t"), "{{ GH_TOKEN }}")
            },
        )
        .await
        .unwrap()
        .entry;
    w.setup
        .attach(&adam, &main.name, template.id)
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 1);
    refused_saying(
        w.setup
            .set_override(&adam, &main.name, template.id, Some(bytes("{{ OTHER }}")))
            .await,
        &["OTHER"],
    );
    refused_saying(
        w.setup
            .create(
                &adam,
                NewEntry {
                    secret: true,
                    template: true,
                    ..entry(file("t2"), "{{ MISSING }}")
                },
            )
            .await,
        &["MISSING"],
    );
    let token_now = w.setup.get(&adam, token.id).await.unwrap().entry;
    refused_saying(
        w.setup
            .change(
                &adam,
                token.id,
                EntryChange {
                    secret: true,
                    target: variable("RENAMED"),
                    ..unchanged(&token_now)
                },
            )
            .await,
        &["GH_TOKEN"],
    );
    let error = refused(
        w.setup
            .set_override(
                &adam,
                &main.name,
                config.id,
                Some(SecretSlice::from(vec![0; FILE_LIMIT + 1])),
            )
            .await,
    );
    assert!(matches!(error, Error::LimitReached(_)));
    assert_eq!(w.secrets.count(), 1, "only the template's own value");
    assert!(matches!(
        w.setup
            .attach(&adam, &sandbox_name("missing"), config.id)
            .await,
        Err(Error::NotFound(_))
    ));
}

/// The owner's sandboxes that run without their latest setup change.
async fn stale(w: &World, owner: &User) -> Vec<String> {
    w.sandboxes
        .list(owner)
        .await
        .unwrap()
        .into_iter()
        .filter(|view| view.stale_setup)
        .map(|view| view.sandbox.name.to_string())
        .collect()
}

fn secret_id(entry: &Entry) -> SecretId {
    match &entry.value.kept {
        Kept::Secret(secret) => secret.id,
        Kept::Plain(_) => panic!("{entry:?}"),
    }
}

#[tokio::test]
async fn a_required_entry_reaches_every_sandbox_and_a_member_cannot_touch_it() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    let main = w.sandbox(&adam, "main").await;
    let lab = w.sandbox(&eve, "lab").await;
    let gateway = w
        .setup
        .create_shared(
            &root,
            shared_variable("GATEWAY_URL", "https://llm", Shared::Required),
        )
        .await
        .unwrap()
        .entry;
    assert_eq!(stale(&w, &adam).await, ["main"]);
    assert_eq!(stale(&w, &eve).await, ["lab"]);
    assert_eq!(
        restarted(&w, &adam, &main).await.0,
        ["GATEWAY_URL=https://llm"]
    );
    assert_eq!(
        restarted(&w, &eve, &lab).await.0,
        ["GATEWAY_URL=https://llm"]
    );
    let later = w.sandbox(&adam, "later").await;
    assert_eq!(
        exposed(&w.engine.env_of(&later.id).unwrap()),
        ["GATEWAY_URL=https://llm"]
    );

    let required = "variable GATEWAY_URL is required in every sandbox";
    for refusal in [
        refused(w.setup.attach(&adam, &main.name, gateway.id).await),
        refused(w.setup.detach(&adam, &main.name, gateway.id).await),
        refused(
            w.setup
                .set_override(&adam, &main.name, gateway.id, Some(bytes("x")))
                .await,
        ),
    ] {
        assert!(
            matches!(&refusal, Error::Invalid(message) if message == required),
            "{refusal:?}"
        );
    }
    let mine = refused(
        w.setup
            .create(&adam, entry(variable("GATEWAY_URL"), "x"))
            .await,
    );
    assert!(
        matches!(&mine, Error::Conflict(message)
            if message == "variable GATEWAY_URL is set for every sandbox by the admin"),
        "{mine:?}"
    );
    let missing = format!("setup entry {}", gateway.id.0);
    assert!(matches!(
        w.setup.change(&adam, gateway.id, unchanged(&gateway)).await,
        Err(Error::NotFound(message)) if message == missing
    ));
    assert!(matches!(
        w.setup.delete(&adam, gateway.id).await,
        Err(Error::NotFound(message)) if message == missing
    ));
    assert_eq!(w.setup.get(&adam, gateway.id).await.unwrap().entry, gateway);
}

#[tokio::test]
async fn making_an_entry_required_is_refused_while_members_have_its_name() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let mut theirs = Vec::new();
    for handle in ["carol", "bob", "alice"] {
        let member = w.member(handle).await;
        let entry = w
            .entry(&member, variable("ANTHROPIC_BASE_URL"), handle, false)
            .await;
        theirs.push((member, entry));
    }
    let required = shared_variable("ANTHROPIC_BASE_URL", "https://llm", Shared::Required);
    let error = refused(w.setup.create_shared(&root, required).await);
    assert!(
        matches!(&error, Error::Conflict(message) if message
            == "variable ANTHROPIC_BASE_URL is also set by alice, bob and carol; \
                ask them to rename theirs first"),
        "{error:?}"
    );
    assert_eq!(w.setup.list_shared(&root).await.unwrap(), []);

    let (carol, carols) = &theirs[0];
    w.setup.delete(carol, carols.id).await.unwrap();
    let offered = w
        .setup
        .create_shared(
            &root,
            shared_variable("ANTHROPIC_BASE_URL", "https://llm", Shared::Offered),
        )
        .await
        .unwrap()
        .entry;
    let error = refused(
        w.setup
            .change_shared(
                &root,
                offered.id,
                SharedEntryChange {
                    shared: Shared::Required,
                    change: unchanged(&offered),
                },
            )
            .await,
    );
    assert!(
        matches!(&error, Error::Conflict(message) if message
            == "variable ANTHROPIC_BASE_URL is also set by alice and bob; \
                ask them to rename theirs first"),
        "{error:?}"
    );
    let listed = w.setup.list_shared(&root).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].entry, offered);
    assert_eq!(
        listed[0]
            .shadowed_by
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["alice", "bob"]
    );
}

#[tokio::test]
async fn an_offered_entry_is_attached_detached_overridden_and_shadowed() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let adam = w.member("adam").await;
    let offered = w
        .setup
        .create_shared(
            &root,
            shared_variable("OPENAI_BASE_URL", "https://llm/v1", Shared::Offered),
        )
        .await
        .unwrap()
        .entry;
    let new = w.sandbox(&adam, "new").await;
    assert_eq!(
        exposed(&w.engine.env_of(&new.id).unwrap()),
        ["OPENAI_BASE_URL=https://llm/v1"]
    );
    w.setup.detach(&adam, &new.name, offered.id).await.unwrap();
    assert_eq!(restarted(&w, &adam, &new).await.0, [] as [&str; 0]);
    w.setup.attach(&adam, &new.name, offered.id).await.unwrap();
    w.setup
        .set_override(&adam, &new.name, offered.id, Some(bytes("https://mine")))
        .await
        .unwrap();
    assert_eq!(
        restarted(&w, &adam, &new).await.0,
        ["OPENAI_BASE_URL=https://mine"]
    );

    let own = w
        .entry(&adam, variable("OPENAI_BASE_URL"), "https://own", false)
        .await;
    let marks: Vec<(EntryId, bool, bool)> = w
        .setup
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| (view.entry.id, view.shadows, view.shadowed))
        .collect();
    assert_eq!(marks, [(offered.id, false, true), (own.id, true, false)]);
    w.setup.attach(&adam, &new.name, own.id).await.unwrap();
    assert_eq!(
        restarted(&w, &adam, &new).await.0,
        ["OPENAI_BASE_URL=https://own"]
    );
    w.setup.detach(&adam, &new.name, own.id).await.unwrap();
    assert_eq!(
        restarted(&w, &adam, &new).await.0,
        ["OPENAI_BASE_URL=https://mine"]
    );

    let next = w.sandbox(&adam, "next").await;
    assert_eq!(
        exposed(&w.engine.env_of(&next.id).unwrap()),
        ["OPENAI_BASE_URL=https://own"]
    );
    assert_eq!(
        attached(&w, &adam, &next).await,
        ["variable OPENAI_BASE_URL"]
    );
}

#[tokio::test]
async fn an_offered_template_resolves_across_layers() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let adam = w.member("adam").await;
    let main = w.sandbox(&adam, "main").await;
    let gateway = w
        .setup
        .create_shared(
            &root,
            shared_variable("GATEWAY_URL", "https://llm", Shared::Required),
        )
        .await
        .unwrap()
        .entry;
    let token = w
        .setup
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(variable("TOKEN"), "t0k")
            },
        )
        .await
        .unwrap()
        .entry;
    let template = w
        .setup
        .create_shared(
            &root,
            NewSharedEntry {
                shared: Shared::Offered,
                entry: NewEntry {
                    template: true,
                    apply_to_new: false,
                    ..entry(file(".config/tool.toml"), "{{ GATEWAY_URL }} {{ TOKEN }}")
                },
            },
        )
        .await
        .unwrap()
        .entry;
    let error = refused(w.setup.attach(&adam, &main.name, template.id).await);
    let Error::Invalid(message) = &error else {
        panic!("{error:?}");
    };
    assert!(
        message.contains("TOKEN") && !message.contains("GATEWAY_URL"),
        "{message}"
    );
    w.setup.attach(&adam, &main.name, token.id).await.unwrap();
    w.setup
        .attach(&adam, &main.name, template.id)
        .await
        .unwrap();
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".config/tool.toml 644 https://llm t0k"]
    );
    assert_eq!(
        w.setup
            .preview(&adam, &main.name, template.id)
            .await
            .unwrap()
            .text,
        "https://llm t0k"
    );
    refused_saying(
        w.setup.delete_shared(&root, gateway.id).await,
        &["sandbox main of adam", "~/.config/tool.toml", "GATEWAY_URL"],
    );
}

#[tokio::test]
async fn a_required_template_may_refer_only_to_required_variables() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let required_ca = || {
        let mut ca = shared_file("ca.pem", "{{ TOKEN }}", Shared::Required);
        ca.entry.template = true;
        ca
    };
    let refusal = "template ~/ca.pem applies to every sandbox and refers to TOKEN, which is \
                   not required";
    let error = refused(w.setup.create_shared(&root, required_ca()).await);
    assert!(
        matches!(&error, Error::Invalid(message) if message == refusal),
        "{error:?}"
    );
    let token = w
        .setup
        .create_shared(&root, shared_variable("TOKEN", "t0k", Shared::Offered))
        .await
        .unwrap()
        .entry;
    let error = refused(w.setup.create_shared(&root, required_ca()).await);
    assert!(
        matches!(&error, Error::Invalid(message) if message == refusal),
        "an offered variable is not required: {error:?}"
    );
    let token = w
        .setup
        .change_shared(
            &root,
            token.id,
            SharedEntryChange {
                shared: Shared::Required,
                change: unchanged(&token),
            },
        )
        .await
        .unwrap()
        .entry;
    w.setup.create_shared(&root, required_ca()).await.unwrap();
    let error = refused(
        w.setup
            .change_shared(
                &root,
                token.id,
                SharedEntryChange {
                    shared: Shared::Offered,
                    change: unchanged(&token),
                },
            )
            .await,
    );
    assert!(
        matches!(&error, Error::Invalid(message) if message == refusal),
        "{error:?}"
    );
    let error = refused(w.setup.delete_shared(&root, token.id).await);
    assert!(
        matches!(&error, Error::Invalid(message) if message == refusal),
        "{error:?}"
    );
}

#[tokio::test]
async fn deleting_an_offered_entry_forgets_overrides_and_marks_the_sandboxes() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    let editor = w.entry(&adam, variable("EDITOR"), "vim", true).await;
    let offered = w
        .setup
        .create_shared(
            &root,
            NewSharedEntry {
                shared: Shared::Offered,
                entry: NewEntry {
                    secret: true,
                    apply_to_new: false,
                    ..entry(variable("API_KEY"), "shared-key")
                },
            },
        )
        .await
        .unwrap()
        .entry;
    let main = w.sandbox(&adam, "main").await;
    let lab = w.sandbox(&eve, "lab").await;
    let mut kept = vec![secret_id(&offered)];
    for (owner, sandbox) in [(&adam, &main), (&eve, &lab)] {
        w.setup
            .attach(owner, &sandbox.name, offered.id)
            .await
            .unwrap();
        w.setup
            .set_override(
                owner,
                &sandbox.name,
                offered.id,
                Some(bytes(&format!("{}-key", owner.handle))),
            )
            .await
            .unwrap();
        let attached = w.setup.of_sandbox(owner, &sandbox.name).await.unwrap();
        let overriding = attached
            .iter()
            .find(|attached| attached.view.entry.id == offered.id)
            .and_then(|attached| attached.overriding.as_ref())
            .unwrap();
        let Kept::Secret(secret) = &overriding.kept else {
            panic!("{overriding:?}");
        };
        kept.push(secret.id);
        restarted(&w, owner, sandbox).await;
    }

    w.setup.delete_shared(&root, offered.id).await.unwrap();
    assert_eq!(stale(&w, &adam).await, ["main"]);
    assert_eq!(stale(&w, &eve).await, ["lab"]);
    for owner in [UserId::SHARED, adam.id, eve.id] {
        assert!(w.secrets.get(owner, &kept).await.unwrap().is_empty());
    }
    assert_eq!(w.secrets.count(), 1, "adam's EDITOR alone");
    let adams: Vec<Entry> = w
        .setup
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| view.entry)
        .collect();
    assert_eq!(adams, [editor]);
    assert_eq!(w.setup.list(&eve).await.unwrap(), []);
    assert_eq!(restarted(&w, &adam, &main).await.0, ["EDITOR=vim"]);
    assert_eq!(restarted(&w, &eve, &lab).await.0, [] as [&str; 0]);
}

#[tokio::test]
async fn shared_secret_values_live_under_the_shared_owner() {
    let w = World::new();
    let root = admin_proof(&w.admin("root").await);
    let adam = w.member("adam").await;
    let shared = w
        .setup
        .create_shared(
            &root,
            NewSharedEntry {
                shared: Shared::Required,
                entry: NewEntry {
                    secret: true,
                    ..entry(variable("GATEWAY_TOKEN"), "sk-shared-0123456789")
                },
            },
        )
        .await
        .unwrap()
        .entry;
    let id = secret_id(&shared);
    let held = w.secrets.get(UserId::SHARED, &[id]).await.unwrap();
    assert_eq!(held[&id].expose_secret(), b"sk-shared-0123456789");
    assert!(w.secrets.get(adam.id, &[id]).await.unwrap().is_empty());
    let listed = w.setup.list(&adam).await.unwrap();
    assert!(listed[0].entry.is_secret());
    let debug = format!("{listed:?}");
    assert!(!debug.contains("0123456789"), "{debug}");
    let main = w.sandbox(&adam, "main").await;
    assert_eq!(
        exposed(&w.engine.env_of(&main.id).unwrap()),
        ["GATEWAY_TOKEN=sk-shared-0123456789"]
    );
}

#[tokio::test]
async fn the_sandbox_file_carries_the_repository() {
    let w = World::new();
    let adam = w.member("adam").await;
    let url = "https://github.com/adamaq01/corral.git";
    let request = CreateSandbox {
        repository: Some(Repository::new(url, Some("main")).unwrap()),
        ..request("cloning", None, None)
    };
    let cloning = w.sandboxes.create(&adam, request).await.unwrap().sandbox;
    let plain = w.sandbox(&adam, "plain").await;
    for (sandbox, repository) in [
        (&cloning, json!({ "url": url, "branch": "main" })),
        (&plain, Value::Null),
    ] {
        let token = SandboxToken::generate().unwrap();
        let boot = w.setup.boot(sandbox, &token).await.unwrap();
        let own = &boot.files[&HomePath::parse(SANDBOX_FILE).unwrap()];
        let read: Value = serde_json::from_slice(own.content.expose_secret()).unwrap();
        assert_eq!(read["sandbox"], json!(sandbox.name.as_str()));
        assert_eq!(read["repository"], repository, "{}", sandbox.name);
    }
}
