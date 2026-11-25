use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_activity::{Busy, Settings, Signals, Sweepable};
use corral_core::Error;
use corral_core::domain::{
    Allocation, FileMode, Handle, HomeFile, HomeFiles, HomePath, IdleLimits, Resources, SandboxId,
    SandboxName, SandboxStatus, UserId, VariableName,
};
use corral_core::fakes::MemorySandboxRepo;
use corral_core::ports::{
    ActivitySignals, Boot, EventKind, Inspection, Readiness, SandboxEngine, SandboxSpec,
    SandboxStreams, Usage, VariableMap,
};
use corral_engine_docker::{DockerConfig, DockerEngine, DockerStreams};
use secrecy::{SecretSlice, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::sync::CancellationToken;

const WAKE: Duration = Duration::from_secs(60);

fn image() -> String {
    std::env::var("CORRAL_SANDBOX_IMAGE").unwrap_or_else(|_| "corral-sandbox:dev".to_owned())
}

fn overlay() -> String {
    std::env::var("CORRAL_OVERLAY_IMAGE").unwrap_or_else(|_| "corral-overlay:dev".to_owned())
}

fn docker(args: &[&str]) -> Output {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run docker")
}

fn docker_ok(args: &[&str]) -> String {
    let output = docker(args);
    assert!(
        output.status.success(),
        "docker {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
}

fn env(pairs: &[(&str, &str)]) -> VariableMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                VariableName::parse(name).unwrap(),
                SecretString::from(*value),
            )
        })
        .collect()
}

fn token(token: &str) -> VariableMap {
    env(&[("FOO_TOKEN", token)])
}

fn spec(name: &str) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("adam").unwrap(),
        user: UserId(uuid::Uuid::nil()),
        name: SandboxName::parse(name).unwrap(),
        image: image(),
        resources: Resources {
            cpu_millis: Allocation::new(250, 1000).unwrap(),
            memory_bytes: Allocation::new(256 << 20, 1 << 30).unwrap(),
            home_bytes: NonZeroU64::new(1 << 30).unwrap(),
        },
    }
}

fn container_name(id: &SandboxId) -> String {
    format!("corral-{}", id.0)
}

fn exists(kind: &str, name: &str) -> bool {
    docker(&[kind, "inspect", name]).status.success()
}

fn docker_answers(args: &[&str]) -> bool {
    Command::new("docker")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A network for the sandboxes the test makes, which the gate reaches through Docker. Everything
/// it and the sandboxes it records create is removed when it drops, even after a failed assertion.
struct Lab {
    network: String,
    sandboxes: Mutex<Vec<SandboxId>>,
    streams: Arc<DockerStreams>,
    key: std::path::PathBuf,
    public_key: String,
}

impl Lab {
    fn open(test: &str) -> Option<Self> {
        for needed in [&image(), &overlay()] {
            if !docker_answers(&["image", "inspect", needed]) {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "CI needs a Docker daemon and image {needed}"
                );
                eprintln!(
                    "skipped: no Docker daemon or image {needed} (build them with \
                     images/sandbox/test.sh and images/overlay/test.sh)"
                );
                return None;
            }
        }
        let network = format!("corral-test-docker-{test}-{}", std::process::id());
        docker_ok(&["network", "create", &network]);
        let key = std::env::temp_dir().join(format!("{network}-key"));
        let _ = std::fs::remove_file(&key);
        let made = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .status()
            .unwrap();
        assert!(made.success());
        let public_key = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        let public_key = public_key
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        Some(Self {
            network,
            sandboxes: Mutex::default(),
            streams: Arc::new(DockerStreams::connect().unwrap()),
            key,
            public_key,
        })
    }

    async fn engine(&self, max_running: usize) -> Arc<DockerEngine> {
        self.engine_with(max_running, &overlay()).await
    }

    async fn engine_with(&self, max_running: usize, overlay: &str) -> Arc<DockerEngine> {
        let engine = DockerEngine::connect(
            DockerConfig {
                network: self.network.clone(),
                overlay_image: overlay.to_owned(),
            },
            max_running,
            Vec::new(),
        )
        .await
        .unwrap();
        Arc::new(engine)
    }

    async fn create(&self, engine: &DockerEngine, name: &str) -> SandboxId {
        self.create_from(engine, &spec(name)).await
    }

    async fn create_from(&self, engine: &DockerEngine, spec: &SandboxSpec) -> SandboxId {
        let id = SandboxId::generate(&spec.owner, &spec.name);
        engine.create(&id, spec).await.unwrap();
        self.sandboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id.clone());
        id
    }

    /// An SSH session the way the gate opens one: OpenSSH to `corral-sshd`, run in the sandbox
    /// through the engine's stream, here behind a local port.
    async fn ssh_session(&self, id: &SandboxId, command: &str) -> tokio::process::Child {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (streams, id, public_key) = (self.streams.clone(), id.clone(), self.public_key.clone());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let argv = ["/opt/corral/libexec/corral-sshd", public_key.as_str()];
            let mut stream = streams.open(&id, &argv).await.unwrap().compat();
            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
        });
        tokio::process::Command::new("ssh")
            .args(["-F", "/dev/null", "-i"])
            .arg(&self.key)
            .args(["-p", &port.to_string()])
            .args(["-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes"])
            .args([
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
            ])
            .args(["-o", "LogLevel=ERROR", "sandbox@127.0.0.1", command])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    /// What `command` prints, its last line break left out, run in an SSH session.
    async fn ssh(&self, id: &SandboxId, command: &str) -> String {
        let output = self
            .ssh_session(id, command)
            .await
            .wait_with_output()
            .await
            .unwrap();
        assert!(output.status.success(), "ssh {command}: {output:?}");
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for id in self
            .sandboxes
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let name = container_name(id);
            docker(&["rm", "--force", &name, &format!("{name}-next")]);
            docker(&["volume", "rm", &format!("corral-home-{}", id.0)]);
        }
        docker(&["network", "rm", &self.network]);
        let _ = std::fs::remove_file(&self.key);
        let _ = std::fs::remove_file(self.key.with_extension("pub"));
    }
}

async fn state(engine: &DockerEngine, id: &SandboxId) -> SandboxStatus {
    engine.status(id).await.unwrap()
}

async fn wake(engine: &DockerEngine, id: &SandboxId, env: VariableMap) {
    wake_with(
        engine,
        id,
        Boot {
            env,
            ..Boot::default()
        },
    )
    .await;
}

/// Whether the start booted the sandbox.
async fn wake_with(engine: &DockerEngine, id: &SandboxId, boot: Boot) -> bool {
    let started = engine.start(id, None, boot, WAKE).await.unwrap();
    match started.readiness {
        Readiness::Ready => started.booted,
        Readiness::Waking => panic!("sandbox {} was not ready within {WAKE:?}", id.0),
    }
}

fn home_files(files: &[(&str, u32, &str)]) -> HomeFiles {
    files
        .iter()
        .map(|(path, mode, content)| {
            (
                HomePath::parse(path).unwrap(),
                HomeFile {
                    mode: FileMode::new(*mode).unwrap(),
                    content: SecretSlice::from(content.as_bytes().to_vec()),
                },
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sandbox_lifecycle_over_real_ssh() {
    let Some(lab) = Lab::open("lifecycle") else {
        return;
    };
    let engine = lab.engine(10).await;

    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    assert_eq!(
        state(&engine, &id).await,
        SandboxStatus::Sleeping,
        "create only provisions"
    );
    engine.create(&id, &spec("main")).await.unwrap();
    let networks = docker_ok(&[
        "inspect",
        "--format",
        "{{range $network, $_ := .NetworkSettings.Networks}}{{$network}} {{end}}",
        &name,
    ]);
    assert_eq!(
        networks, lab.network,
        "a sandbox joins only the sandbox network"
    );

    wake(&engine, &id, VariableMap::new()).await;
    assert_eq!(state(&engine, &id).await, SandboxStatus::Running);
    assert_eq!(
        docker_ok(&["exec", &name, "/opt/corral/bin/busybox", "ps", "-o", "args"])
            .lines()
            .filter(|args| args.contains("sshd"))
            .count(),
        0,
        "no sshd runs in the sandbox"
    );
    assert_eq!(lab.ssh(&id, "id -un").await, "sandbox");
    assert_eq!(
        lab.ssh(&id, "env | grep ^CORRAL_; true").await,
        "",
        "no CORRAL_ variable reaches the session"
    );
    lab.ssh(&id, "echo kept > ~/persist-check").await;

    engine.stop(&id).await.unwrap();
    assert_eq!(state(&engine, &id).await, SandboxStatus::Sleeping);
    engine.stop(&id).await.unwrap();
    wake(&engine, &id, VariableMap::new()).await;
    assert_eq!(lab.ssh(&id, "cat ~/persist-check").await, "kept");

    engine.delete(&id).await.unwrap();
    assert!(!exists("container", &name));
    assert!(!exists("volume", &format!("corral-home-{}", id.0)));
    engine.delete(&id).await.unwrap();
    assert!(matches!(engine.status(&id).await, Err(Error::NotFound(_))));
    assert!(matches!(
        engine.start(&id, None, Boot::default(), WAKE).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(engine.stop(&id).await, Err(Error::NotFound(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn variables_apply_at_each_start_and_stay_out_of_the_container_config() {
    let Some(lab) = Lab::open("variables") else {
        return;
    };
    let engine = lab.engine(10).await;
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    let secret = |word: &str| format!("corral-test-{word}-{}", std::process::id());

    wake(&engine, &id, token(&secret("one"))).await;
    assert_eq!(lab.ssh(&id, "printenv FOO_TOKEN").await, secret("one"));
    lab.ssh(&id, "echo kept > ~/persist-check").await;

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, token(&secret("two"))).await;
    assert_eq!(
        lab.ssh(&id, "printenv FOO_TOKEN; cat ~/persist-check")
            .await,
        format!("{}\nkept", secret("two")),
        "a start after a stop applies the changed variable and keeps the home"
    );

    wake(&engine, &id, token(&secret("ignored"))).await;
    assert_eq!(
        lab.ssh(&id, "printenv FOO_TOKEN").await,
        secret("two"),
        "a running sandbox keeps its variables"
    );

    let odd = r#"p#ss "word" it's $HOME \x\"#;
    engine.stop(&id).await.unwrap();
    wake(&engine, &id, env(&[("ODD_TOKEN", odd)])).await;
    assert_eq!(lab.ssh(&id, "printenv ODD_TOKEN").await, odd);
    assert_eq!(
        lab.ssh(&id, "printenv FOO_TOKEN; true").await,
        "",
        "a removed variable is gone"
    );

    let inspected = docker_ok(&["inspect", &name]);
    for word in ["one", "two", "ignored"] {
        assert!(
            !inspected.contains(&secret(word)),
            "{word} in docker inspect"
        );
    }
    assert!(!inspected.contains("ODD_TOKEN"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_files_are_written_at_each_start_from_stopped() {
    let Some(lab) = Lab::open("home-files") else {
        return;
    };
    let engine = lab.engine(10).await;
    let id = lab.create(&engine, "main").await;
    let boot = |files| Boot {
        env: VariableMap::new(),
        files: home_files(files),
    };
    let listing = "for f in .config/corral/sandbox.json '.config/corral' 'notes dir/notes.txt'; do \
                   stat -c '%a %U %G %n' ~/\"$f\"; done; cat ~/.config/corral/sandbox.json";

    let booted = wake_with(
        &engine,
        &id,
        boot(&[
            (".config/corral/sandbox.json", 0o600, "first"),
            ("notes dir/notes.txt", 0o644, "notes"),
        ]),
    )
    .await;
    assert!(booted);
    assert_eq!(
        lab.ssh(&id, listing).await,
        "600 sandbox sandbox /home/sandbox/.config/corral/sandbox.json\n\
         755 sandbox sandbox /home/sandbox/.config/corral\n\
         644 sandbox sandbox /home/sandbox/notes dir/notes.txt\n\
         first"
    );
    lab.ssh(
        &id,
        "echo edited > ~/.config/corral/sandbox.json && echo mine > ~/kept",
    )
    .await;

    let booted = wake_with(
        &engine,
        &id,
        boot(&[(".config/corral/sandbox.json", 0o600, "ignored")]),
    )
    .await;
    assert!(!booted, "a running sandbox is not booted again");
    assert_eq!(
        lab.ssh(&id, "cat ~/.config/corral/sandbox.json").await,
        "edited"
    );

    engine.stop(&id).await.unwrap();
    let booted = wake_with(
        &engine,
        &id,
        boot(&[(".config/corral/sandbox.json", 0o600, "second")]),
    )
    .await;
    assert!(booted);
    assert_eq!(
        lab.ssh(
            &id,
            "cat ~/.config/corral/sandbox.json ~/kept; test -e ~/'notes dir/notes.txt' || echo gone"
        )
        .await,
        "secondmine\ngone",
        "a start rewrites the files, removes those it no longer writes, and leaves the rest of \
         the home alone"
    );
    assert_eq!(
        lab.ssh(
            &id,
            "stat -c %a /etc/corral/home-files; cat /etc/corral/home-files/manifest"
        )
        .await,
        "700\n0 0600 .config/corral/sandbox.json"
    );
}

/// Image tags a test makes, removed when it drops. Declared before the lab, it drops after the
/// containers that use them.
#[derive(Default)]
struct Tags(Mutex<Vec<String>>);

impl Tags {
    fn named(test: &str, suffix: &str) -> String {
        format!("corral-test-docker-{test}-{}:{suffix}", std::process::id())
    }

    /// Points `tag` at the image `reference` names.
    fn tag(&self, reference: &str, tag: &str) {
        docker_ok(&["tag", reference, tag]);
        let mut tags = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !tags.iter().any(|known| known == tag) {
            tags.push(tag.to_owned());
        }
    }

    /// The sandbox image under a new ID, tagged `tag`; returns that ID.
    fn variant(&self, tag: &str) -> String {
        self.variant_of(&image(), tag)
    }

    /// The image `reference` names under a new ID, tagged `tag`; returns that ID.
    fn variant_of(&self, reference: &str, tag: &str) -> String {
        // The overlay has no command of its own, and a create needs one, never run.
        let source = docker_ok(&["create", reference, "/bin/true"]);
        docker_ok(&[
            "commit",
            "--change",
            "LABEL corral.test=variant",
            &source,
            tag,
        ]);
        docker_ok(&["rm", &source]);
        self.tag(tag, tag);
        image_id(tag)
    }
}

impl Drop for Tags {
    fn drop(&mut self) {
        for tag in self.0.get_mut().unwrap_or_else(PoisonError::into_inner) {
            docker(&["rmi", tag]);
        }
    }
}

fn image_id(reference: &str) -> String {
    docker_ok(&["image", "inspect", "--format", "{{.Id}}", reference])
}

fn inspect(container: &str, format: &str) -> String {
    docker_ok(&["inspect", "--format", format, container])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_from_stopped_remakes_an_outdated_container_keeping_its_home() {
    let tags = Tags::default();
    let Some(lab) = Lab::open("converge") else {
        return;
    };
    let engine = lab.engine(10).await;
    let (current, newer) = (
        Tags::named("converge", "current"),
        Tags::named("converge", "newer"),
    );
    tags.tag(&image(), &current);
    let newer_id = tags.variant(&newer);
    let spec = SandboxSpec {
        image: current.clone(),
        ..spec("main")
    };
    let id = lab.create_from(&engine, &spec).await;
    let name = container_name(&id);
    wake(&engine, &id, VariableMap::new()).await;
    lab.ssh(&id, "echo kept > ~/kept").await;
    engine.stop(&id).await.unwrap();

    // What an engine from before this one made: no definition label, and an env var it dropped.
    let labels: BTreeMap<String, String> =
        serde_json::from_str(&inspect(&name, "{{json .Config.Labels}}")).unwrap();
    let volume = format!(
        "type=volume,source=corral-home-{},target=/home/sandbox",
        id.0
    );
    let mut old = vec![
        "create".to_owned(),
        format!("--name={name}"),
        format!("--network={}", lab.network),
        format!("--mount={volume}"),
        "--env=CORRAL_SSHD=1".to_owned(),
        "--env=CORRAL_RETIRED=1".to_owned(),
        "--cpus=1".to_owned(),
        "--memory=1g".to_owned(),
    ];
    old.extend(
        labels
            .iter()
            .filter(|(key, _)| key.as_str() != "corral.adamaq01.fr/definition")
            .map(|(key, value)| format!("--label={key}={value}")),
    );
    old.push(current.clone());
    docker_ok(&["rm", &name]);
    docker_ok(&old.iter().map(String::as_str).collect::<Vec<_>>());

    let booted = wake_with(&engine, &id, Boot::default()).await;
    assert!(booted);
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");
    let env = inspect(&name, "{{json .Config.Env}}");
    assert!(!env.contains("CORRAL_RETIRED"), "{env}");
    let remade = inspect(&name, "{{.Id}}");

    tags.tag(&newer, &current);
    let booted = wake_with(&engine, &id, Boot::default()).await;
    assert!(!booted);
    assert_eq!(
        inspect(&name, "{{.Id}}"),
        remade,
        "a running sandbox is left as it is"
    );

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, VariableMap::new()).await;
    assert_eq!(
        inspect(&name, "{{.Image}}"),
        newer_id,
        "the next start takes the image its reference names now"
    );
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");
    let remade = inspect(&name, "{{.Id}}");

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, VariableMap::new()).await;
    assert_eq!(
        inspect(&name, "{{.Id}}"),
        remade,
        "a container that is up to date is kept"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_from_stopped_remakes_the_sandbox_from_the_spec_it_is_given() {
    let tags = Tags::default();
    let Some(lab) = Lab::open("respec") else {
        return;
    };
    let engine = lab.engine(10).await;
    let newer = Tags::named("respec", "newer");
    let newer_id = tags.variant(&newer);
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    wake(&engine, &id, VariableMap::new()).await;
    lab.ssh(&id, "echo kept > ~/kept").await;

    let mut edited = spec("main");
    edited.image.clone_from(&newer);
    edited.resources.memory_bytes = Allocation::new(256 << 20, 2 << 30).unwrap();
    let running = engine
        .start(&id, Some(&edited), Boot::default(), WAKE)
        .await
        .unwrap();
    assert!(!running.booted, "a running sandbox is left as it is");
    assert_eq!(inspect(&name, "{{.Config.Image}}"), image());

    engine.stop(&id).await.unwrap();
    let started = engine
        .start(&id, Some(&edited), Boot::default(), WAKE)
        .await
        .unwrap();
    assert!(started.booted);
    assert_eq!(started.readiness, Readiness::Ready);
    assert_eq!(
        (
            inspect(&name, "{{.Image}}"),
            inspect(&name, "{{.HostConfig.Memory}}")
        ),
        (newer_id, (2u64 << 30).to_string())
    );
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_from_stopped_mounts_the_overlay_the_engine_pinned() {
    let tags = Tags::default();
    let Some(lab) = Lab::open("overlay") else {
        return;
    };
    let newer = Tags::named("overlay", "newer");
    let newer_id = tags.variant_of(&overlay(), &newer);
    let overlay_id = image_id(&overlay());
    let engine = lab.engine(10).await;
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    wake(&engine, &id, VariableMap::new()).await;
    lab.ssh(&id, "echo kept > ~/kept").await;
    let first = inspect(&name, "{{.Id}}");
    let mounts = inspect(&name, "{{json .HostConfig.Mounts}}");
    assert!(mounts.contains(&overlay_id), "{mounts}");

    let upgraded = lab.engine_with(10, &newer).await;
    wake(&upgraded, &id, VariableMap::new()).await;
    assert_eq!(
        inspect(&name, "{{.Id}}"),
        first,
        "a running sandbox keeps its overlay"
    );
    upgraded.stop(&id).await.unwrap();
    wake(&upgraded, &id, VariableMap::new()).await;
    let mounts = inspect(&name, "{{json .HostConfig.Mounts}}");
    assert!(mounts.contains(&newer_id), "{mounts}");
    assert_ne!(inspect(&name, "{{.Id}}"), first);
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remake_cut_short_finishes_at_the_next_start() {
    let tags = Tags::default();
    let Some(lab) = Lab::open("remake") else {
        return;
    };
    let engine = lab.engine(10).await;
    let (current, newer) = (
        Tags::named("remake", "current"),
        Tags::named("remake", "newer"),
    );
    tags.tag(&image(), &current);
    let newer_id = tags.variant(&newer);
    let spec = SandboxSpec {
        image: current.clone(),
        ..spec("main")
    };
    let id = lab.create_from(&engine, &spec).await;
    let name = container_name(&id);
    let replacement = format!("{name}-next");
    wake(&engine, &id, VariableMap::new()).await;
    lab.ssh(&id, "echo kept > ~/kept").await;
    engine.stop(&id).await.unwrap();

    // Cut short before the old container went: its replacement is left over.
    tags.tag(&newer, &current);
    docker_ok(&["create", "--name", &replacement, &image()]);
    wake(&engine, &id, VariableMap::new()).await;
    assert_eq!(inspect(&name, "{{.Image}}"), newer_id);
    assert!(!exists("container", &replacement));
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");

    // Cut short after: the complete replacement is all there is.
    engine.stop(&id).await.unwrap();
    docker_ok(&["rename", &name, &replacement]);
    wake(&engine, &id, VariableMap::new()).await;
    assert!(!exists("container", &replacement));
    assert_eq!(lab.ssh(&id, "cat ~/kept").await, "kept");

    engine.stop(&id).await.unwrap();
    docker_ok(&["create", "--name", &replacement, &image()]);
    engine.delete(&id).await.unwrap();
    assert!(!exists("container", &name));
    assert!(!exists("container", &replacement));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_starts_never_exceed_max_running() {
    const MAX: usize = 2;
    let Some(lab) = Lab::open("capacity") else {
        return;
    };
    let engine = lab.engine(MAX).await;
    let mut ids = Vec::new();
    for n in 0..2 * MAX {
        ids.push(lab.create(&engine, &format!("box-{n}")).await);
    }

    let mut starts = JoinSet::new();
    for id in ids.clone() {
        let engine = engine.clone();
        starts.spawn(async move {
            engine
                .start(&id, None, Boot::default(), Duration::ZERO)
                .await
        });
    }
    let (mut started, mut refused) = (0, 0);
    while let Some(outcome) = starts.join_next().await {
        match outcome.unwrap() {
            Ok(_) => started += 1,
            Err(Error::LimitReached(_)) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((started, refused), (MAX, MAX));
    let mut running = Vec::new();
    for id in &ids {
        if state(&engine, id).await == SandboxStatus::Running {
            running.push(id.clone());
        }
    }
    assert_eq!(running.len(), MAX);

    let asleep = ids.iter().find(|id| !running.contains(id)).unwrap();
    assert!(matches!(
        engine
            .start(asleep, None, Boot::default(), Duration::ZERO)
            .await,
        Err(Error::LimitReached(_))
    ));
    engine.stop(&running[0]).await.unwrap();
    engine
        .start(asleep, None, Boot::default(), Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(state(&engine, asleep).await, SandboxStatus::Running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_sandboxes_sleep_and_ones_at_work_stay_awake() {
    let Some(lab) = Lab::open("reaper") else {
        return;
    };
    let timeout = Duration::from_secs(3);
    let engine = lab.engine(10).await;
    let idle = lab.create(&engine, "idle").await;
    let worked = lab.create(&engine, "worked").await;
    let busy = lab.create(&engine, "busy").await;
    for id in [&idle, &worked, &busy] {
        wake(&engine, id, VariableMap::new()).await;
    }

    let every = Duration::from_secs(1);
    let busy_above = Busy {
        cpu_percent: 5.0,
        network_bytes_per_sec: 4096,
    };
    let signals = Signals::new(engine.activity().clone(), busy_above, every);
    let shutdown = CancellationToken::new();
    let feeding = {
        let (idle, worked, busy) = (idle.clone(), worked.clone(), busy.clone());
        tokio::spawn(shutdown.clone().run_until_cancelled_owned(async move {
            loop {
                let usage = |cpu_millis| Usage {
                    interval: every,
                    cpu: Duration::from_millis(cpu_millis),
                    network_bytes: 0,
                };
                signals.worked(std::slice::from_ref(&worked));
                for id in [&idle, &worked] {
                    signals.measured(id, usage(0));
                }
                signals.measured(&busy, usage(500));
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }))
    };
    let settings = Settings {
        limits: IdleLimits {
            default_timeout: timeout,
            max_timeout: timeout,
            max_keep_awake: timeout,
        },
        check_every: every,
        report_every: every,
    };
    let lifecycle = corral_activity::spawn(
        engine.clone(),
        Arc::new(MemorySandboxRepo::default()),
        settings,
        shutdown.clone(),
    );
    let started = Instant::now();
    while state(&engine, &idle).await != SandboxStatus::Sleeping {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the idle sandbox never slept"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(timeout + Duration::from_secs(3)).await;
    assert_eq!(
        state(&engine, &worked).await,
        SandboxStatus::Running,
        "work through the front door keeps it awake"
    );
    assert_eq!(
        state(&engine, &busy).await,
        SandboxStatus::Running,
        "a busy report keeps it awake"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), lifecycle)
        .await
        .expect("the lifecycle stops on shutdown")
        .unwrap();
    feeding.await.unwrap();
}

#[tokio::test]
async fn connect_makes_a_missing_network() {
    if !docker_answers(&["image", "inspect", &overlay()]) {
        assert!(std::env::var_os("CI").is_none(), "CI needs a Docker daemon");
        eprintln!("skipped: no Docker daemon or overlay");
        return;
    }
    let missing = format!("corral-test-docker-missing-{}", std::process::id());
    let connected = DockerEngine::connect(
        DockerConfig {
            network: missing.clone(),
            overlay_image: overlay(),
        },
        1,
        Vec::new(),
    )
    .await;
    let label = docker_ok(&[
        "network",
        "inspect",
        "--format",
        "{{index .Labels \"app.kubernetes.io/managed-by\"}}",
        &missing,
    ]);
    docker(&["network", "rm", &missing]);
    assert!(connected.is_ok(), "{:?}", connected.err());
    assert_eq!(label, "corral");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_shows_the_container_its_image_home_and_events() {
    let Some(lab) = Lab::open("inspection") else {
        return;
    };
    let engine = lab.engine(10).await;
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    let volume = format!("corral-home-{}", id.0);
    let image_id = docker_ok(&["image", "inspect", "--format", "{{.Id}}", &image()]);
    let container_id = docker_ok(&["inspect", "--format", "{{.Id}}", &name]);

    let created = engine.inspect(&id).await.unwrap();
    for (label, value) in [
        ("Container", Some(name.as_str())),
        ("Container ID", Some(&container_id)),
        ("State", Some("created")),
        ("Started", None),
        ("Exit code", None),
        ("Network", Some(&lab.network)),
        ("Image", Some(&image())),
        ("Image ID", Some(&image_id)),
        ("Home volume", Some(&volume)),
        ("Volume driver", Some("local")),
    ] {
        assert_eq!(fact(&created, label), value, "{label}: {created:#?}");
    }

    wake(&engine, &id, VariableMap::new()).await;
    let running = engine.inspect(&id).await.unwrap();
    assert_eq!(fact(&running, "State"), Some("running"));
    assert!(fact(&running, "Started").is_some_and(|at| at.ends_with(" UTC")));
    assert_eq!(fact(&running, "Finished"), None);

    engine.stop(&id).await.unwrap();
    let stopped = engine.inspect(&id).await.unwrap();
    assert_eq!(fact(&stopped, "State"), Some("exited"));
    assert!(fact(&stopped, "Exit code").is_some(), "{stopped:#?}");
    assert!(fact(&stopped, "Finished").is_some(), "{stopped:#?}");
    let events: Vec<(&str, EventKind)> = stopped
        .events
        .iter()
        .map(|event| (event.reason.as_str(), event.kind))
        .collect();
    let started = events
        .iter()
        .position(|event| *event == ("Started", EventKind::Normal));
    let exited = events
        .iter()
        .position(|event| *event == ("Exited", EventKind::Normal));
    assert!(
        matches!((exited, started), (Some(exited), Some(started)) if exited < started),
        "a stop's exit is normal and newer than the start: {events:?}"
    );
    let object = format!("Container {name}");
    assert!(stopped.events.iter().all(|event| event.object == object));

    engine.delete(&id).await.unwrap();
    let gone = engine.inspect(&id).await.unwrap();
    assert!(
        gone.facts.iter().all(|fact| fact.value.is_none()),
        "{gone:#?}"
    );
}

fn fact<'a>(inspection: &'a Inspection, label: &str) -> Option<&'a str> {
    inspection
        .facts
        .iter()
        .find(|fact| fact.label == label)
        .and_then(|fact| fact.value.as_deref())
}
