use secrecy::{ExposeSecret, SecretSlice};

use super::{EntryChange, FILE_LIMIT, NewEntry};
use crate::Error;
use crate::domain::{EntryId, Glimpse, HomePath, Kept, Sandbox, SandboxName, Target, User};
use crate::fakes::{World, bytes, credential, entry, exposed, file, sandbox_name, variable};
use crate::ports::SecretStore;

fn flags(template: bool, apply_to_new: bool) -> EntryChange {
    EntryChange {
        value: None,
        template,
        apply_to_new,
    }
}

async fn attached(w: &World, owner: &User, sandbox: &Sandbox) -> Vec<String> {
    w.environment
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
        .filter(|(path, _)| path.as_str() != super::SANDBOX_FILE)
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

#[tokio::test]
async fn a_new_sandbox_gets_each_entry_that_applies_to_new_sandboxes() {
    let w = World::new();
    let adam = w.member("adam").await;
    let eve = w.member("eve").await;
    let gh = w.entry(&adam, variable("GH_TOKEN"), "ghp_1", true).await;
    let notes = w.entry(&adam, file("notes.txt"), "hello", false).await;
    w.environment
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

    w.environment
        .change(&adam, notes.id, flags(false, false))
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
        .environment
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| (view.entry.target.to_string(), view.sandboxes))
        .collect();
    assert_eq!(
        listed,
        [
            (
                "variable GH_TOKEN".to_owned(),
                vec![main.name.clone(), dev.name.clone()]
            ),
            ("variable LATER".to_owned(), vec![dev.name.clone()]),
            ("variable OPTIONAL".to_owned(), vec![]),
            ("file notes.txt".to_owned(), vec![main.name.clone()]),
        ]
    );
    assert_eq!(
        w.environment
            .get(&adam, gh.id)
            .await
            .unwrap()
            .sandboxes
            .len(),
        2
    );
    for missing in [later.id, EntryId::generate()] {
        assert!(matches!(
            w.environment.get(&eve, missing).await,
            Err(Error::NotFound(_))
        ));
    }
}

#[tokio::test]
async fn colliding_entries_never_meet_in_one_sandbox() {
    let w = World::new();
    let adam = w.member("adam").await;
    let work = w.entry(&adam, variable("GH_TOKEN"), "work", false).await;
    w.entry(&adam, file("tools/a"), "dir", false).await;
    for collides in [variable("GH_TOKEN"), file("tools"), file("tools/a/b")] {
        assert!(
            matches!(
                w.environment.create(&adam, entry(collides.clone(), "x")).await,
                Err(Error::Conflict(message)) if message.contains("applies to new sandboxes")
            ),
            "{collides}"
        );
    }
    let personal = w
        .environment
        .create(
            &adam,
            NewEntry {
                apply_to_new: false,
                ..entry(variable("GH_TOKEN"), "personal")
            },
        )
        .await
        .unwrap()
        .entry;
    assert!(matches!(
        w.environment
            .change(&adam, personal.id, flags(false, true))
            .await,
        Err(Error::Conflict(_))
    ));

    let main = w.sandbox(&adam, "main").await;
    let error = refused(w.environment.attach(&adam, &main.name, personal.id).await);
    assert!(
        matches!(&error, Error::Conflict(message) if message.contains("variable GH_TOKEN") && message.contains("main")),
        "{error}"
    );
    w.environment
        .detach(&adam, &main.name, work.id)
        .await
        .unwrap();
    w.environment
        .attach(&adam, &main.name, personal.id)
        .await
        .unwrap();
    w.environment
        .attach(&adam, &main.name, personal.id)
        .await
        .unwrap();
    assert_eq!(restarted(&w, &adam, &main).await.0, ["GH_TOKEN=personal"]);
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
    w.environment
        .set_override(
            &adam,
            &main.name,
            gh.id,
            Some(bytes("ghp_main_only_0123456789")),
        )
        .await
        .unwrap();
    let held = w.environment.of_sandbox(&adam, &main.name).await.unwrap();
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
    assert_eq!(w.secrets.count(), 2);

    w.environment
        .change(
            &adam,
            gh.id,
            EntryChange {
                value: Some(bytes("ghp_rotated")),
                ..flags(false, true)
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

    w.environment
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
        w.environment
            .set_override(&adam, &main.name, notes.id, Some(bytes("x")))
            .await,
        Err(Error::NotFound(message)) if message.contains("not attached to sandbox main")
    ));
    w.environment
        .attach(&adam, &main.name, notes.id)
        .await
        .unwrap();
    w.environment
        .set_override(&adam, &main.name, notes.id, Some(bytes("main's notes")))
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 1, "a plain entry's override is plain");
    w.environment
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
async fn templates_take_the_variables_attached_to_the_sandbox_and_nothing_else() {
    let w = World::new();
    let adam = w.member("adam").await;
    let name = w.entry(&adam, variable("GIT_NAME"), "Adam", false).await;
    let token = w.entry(&adam, variable("GH_TOKEN"), "ghp_x", true).await;
    let config = w
        .environment
        .create(
            &adam,
            NewEntry {
                template: true,
                ..entry(
                    file(".gitconfig"),
                    "name = {{ GIT_NAME }}\ntoken = {{GH_TOKEN}}\n",
                )
            },
        )
        .await
        .unwrap()
        .entry;
    let main = w.sandbox(&adam, "main").await;
    assert_eq!(
        restarted(&w, &adam, &main).await.1,
        [".gitconfig 600 name = Adam\ntoken = ghp_x\n"],
        "a template that takes a secret is the sandbox user's alone"
    );

    w.environment
        .detach(&adam, &main.name, token.id)
        .await
        .unwrap();
    let start = refused(w.sandboxes.restart(&adam, &main.name).await);
    assert!(
        matches!(&start, Error::Invalid(message) if message.contains(".gitconfig") && message.contains("GH_TOKEN") && message.contains("cannot start")),
        "{start}"
    );
    w.environment
        .detach(&adam, &main.name, config.id)
        .await
        .unwrap();
    let error = refused(w.environment.attach(&adam, &main.name, config.id).await);
    assert!(
        matches!(&error, Error::Invalid(message) if message.contains("GH_TOKEN") && message.contains("sandbox main")),
        "{error}"
    );
    w.environment
        .attach(&adam, &main.name, token.id)
        .await
        .unwrap();
    w.environment
        .attach(&adam, &main.name, config.id)
        .await
        .unwrap();
    let error = refused(
        w.environment
            .set_override(&adam, &main.name, config.id, Some(bytes("{{ NOWHERE }}")))
            .await,
    );
    assert!(
        matches!(&error, Error::Invalid(message) if message.contains("NOWHERE")),
        "{error}"
    );
    w.environment
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
    w.environment
        .change(
            &adam,
            name.id,
            EntryChange {
                value: Some(bytes("Adam Q")),
                ..flags(false, true)
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
async fn a_template_names_only_variables_of_its_owner() {
    let w = World::new();
    let adam = w.member("adam").await;
    let unknown = refused(
        w.environment
            .create(
                &adam,
                NewEntry {
                    template: true,
                    ..entry(file(".gitconfig"), "{{ GIT_NAME }}")
                },
            )
            .await,
    );
    assert!(
        matches!(&unknown, Error::Invalid(message) if message.contains("GIT_NAME") && message.contains("not among your variables")),
        "{unknown}"
    );
    w.entry(&adam, variable("GIT_NAME"), "Adam", false).await;
    let plain = w
        .entry(&adam, file("plain"), "{{ GIT_NAME }} {{ x }}", false)
        .await;
    assert!(!plain.template);
    let error = refused(
        w.environment
            .change(&adam, plain.id, flags(true, true))
            .await,
    );
    assert!(
        matches!(&error, Error::Invalid(message) if message.contains("{{ x }}")),
        "{error}"
    );
}

#[tokio::test]
async fn values_their_target_cannot_take_are_refused() {
    let w = World::new();
    let adam = w.member("adam").await;
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
                template: true,
                value: SecretSlice::from(vec![0xff]),
                ..entry(file("binary"), "")
            },
            "must be text",
        ),
        (
            entry(file(".config/corral/sandbox.json"), "{}"),
            "Corral writes",
        ),
        (entry(file(".config/corral"), "{}"), "Corral writes"),
        (
            NewEntry {
                value: SecretSlice::from(vec![0; FILE_LIMIT + 1]),
                ..entry(file("big"), "")
            },
            "256 KiB",
        ),
    ];
    for (attempt, culprit) in attempts {
        let target = attempt.target.clone();
        let error = refused(
            w.environment
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
    assert_eq!(w.environment.list(&adam).await.unwrap(), []);
    assert_eq!(w.secrets.count(), 0);
    w.environment
        .create(
            &adam,
            NewEntry {
                value: SecretSlice::from(vec![0; FILE_LIMIT]),
                ..entry(file("big"), "")
            },
        )
        .await
        .unwrap();
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
            .environment
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
        w.environment.attach(&adam, &main.name, *id).await.unwrap();
    }
    let error = refused(w.environment.attach(&adam, &main.name, entries[3]).await);
    assert!(
        matches!(&error, Error::LimitReached(message) if message.contains("1 MiB") && message.contains("sandbox main")),
        "{error}"
    );
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
    w.environment
        .set_override(&adam, &dev.name, gh.id, Some(bytes("dev's")))
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 2);
    w.environment.delete(&adam, gh.id).await.unwrap();
    assert_eq!(w.secrets.count(), 0);
    for sandbox in [&main, &dev] {
        assert_eq!(attached(&w, &adam, sandbox).await, [] as [&str; 0]);
        assert_eq!(restarted(&w, &adam, sandbox).await.0, [] as [&str; 0]);
    }
    assert!(matches!(
        w.environment.delete(&adam, gh.id).await,
        Err(Error::NotFound(_))
    ));
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
    let glimpse = |entry: &crate::domain::Entry| match &entry.value.kept {
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
    let debug = format!("{:?}", w.environment.list(&adam).await.unwrap());
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
        w.secrets.keep_legacy(adam.id, &credential(name), value);
    }
    w.secrets.keep_legacy(eve.id, &credential("EVES"), "eve");
    for user in [&adam, &eve] {
        w.environment_repo.await_adoption(user.id);
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
    w.environment
        .attach(&adam, &main.name, interrupted.id)
        .await
        .unwrap();

    assert_eq!(w.environment.adopt_credentials().await.unwrap(), 2);
    assert_eq!(w.environment.adopt_credentials().await.unwrap(), 0);
    let adopted: Vec<(Target, bool, bool, Vec<SandboxName>)> = w
        .environment
        .list(&adam)
        .await
        .unwrap()
        .into_iter()
        .map(|view| {
            (
                view.entry.target.clone(),
                view.entry.is_secret(),
                view.entry.apply_to_new,
                view.sandboxes,
            )
        })
        .collect();
    let both = vec![main.name.clone(), dev.name.clone()];
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
    assert_eq!(w.environment.list(&eve).await.unwrap().len(), 1);
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
        .environment
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
    w.environment
        .attach(&adam, &main.name, config.id)
        .await
        .unwrap();
    let token = w.entry(&adam, variable("GH_TOKEN"), "applies", false).await;
    w.environment
        .attach(&adam, &main.name, token.id)
        .await
        .unwrap();
    w.entry(&adam, variable("OTHER"), "elsewhere", false).await;
    let template = w
        .environment
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
    w.environment
        .attach(&adam, &main.name, template.id)
        .await
        .unwrap();
    assert_eq!(w.secrets.count(), 1);
    let error = refused(
        w.environment
            .set_override(&adam, &main.name, template.id, Some(bytes("{{ OTHER }}")))
            .await,
    );
    assert!(
        matches!(&error, Error::Invalid(message) if message.contains("OTHER")),
        "{error}"
    );
    let error = refused(
        w.environment
            .create(
                &adam,
                NewEntry {
                    secret: true,
                    ..entry(variable("GH_TOKEN"), "again")
                },
            )
            .await,
    );
    assert!(matches!(error, Error::Conflict(_)));
    let error = refused(
        w.environment
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
        w.environment
            .attach(&adam, &sandbox_name("missing"), config.id)
            .await,
        Err(Error::NotFound(_))
    ));
}
