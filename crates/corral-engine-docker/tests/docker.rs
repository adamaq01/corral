use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_core::Error;
use corral_core::domain::{
    Allocation, CredentialName, FileMode, Handle, HomeFile, HomeFiles, HomePath, Resources,
    SandboxId, SandboxName, SandboxStatus,
};
use corral_core::ports::{
    Boot, CredentialMap, EventKind, Inspection, Readiness, SandboxEngine, SandboxSpec, SshTarget,
};
use corral_engine_docker::{DockerConfig, DockerEngine};
use secrecy::{SecretSlice, SecretString};
use ssh_key::PublicKey;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const WAKE: Duration = Duration::from_secs(60);
const KNOWN_HOSTS: &str = "/home/sandbox/.ssh/known_hosts";

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

fn env(pairs: &[(&str, &str)]) -> CredentialMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                CredentialName::parse(name).unwrap(),
                SecretString::from(*value),
            )
        })
        .collect()
}

fn token(token: &str) -> CredentialMap {
    env(&[("FOO_TOKEN", token)])
}

fn spec(name: &str) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("adam").unwrap(),
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

/// A network with an SSH client container on it, standing in for the front door. Everything it
/// and the sandboxes it records create is removed when it drops, even after a failed assertion.
struct Lab {
    network: String,
    client: String,
    proxy_public_key: Option<PublicKey>,
    sandboxes: Mutex<Vec<SandboxId>>,
}

impl Lab {
    fn open(test: &str) -> Option<Self> {
        let image = image();
        for needed in [&image, &overlay()] {
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
        let mut lab = Self {
            client: format!("{network}-client"),
            network,
            proxy_public_key: None,
            sandboxes: Mutex::default(),
        };
        docker_ok(&["network", "create", &lab.network]);
        docker_ok(&[
            "run",
            "--detach",
            "--name",
            &lab.client,
            "--network",
            &lab.network,
            &image,
        ]);
        lab.client_sh(
            "mkdir -p -m 700 ~/.ssh && ssh-keygen -q -t ed25519 -N '' -f ~/.ssh/proxy_key",
        );
        let public_key = lab.client_sh("cat ~/.ssh/proxy_key.pub");
        lab.proxy_public_key = Some(PublicKey::from_openssh(&public_key).unwrap());
        Some(lab)
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
            self.proxy_public_key.clone().unwrap(),
            max_running,
        )
        .await
        .unwrap();
        Arc::new(engine)
    }

    async fn create(&self, engine: &DockerEngine, name: &str) -> SandboxId {
        self.create_from(engine, &spec(name)).await
    }

    async fn create_from(&self, engine: &DockerEngine, spec: &SandboxSpec) -> SandboxId {
        let id = SandboxId::generate();
        engine.create(&id, spec).await.unwrap();
        self.sandboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id.clone());
        id
    }

    fn client(&self, argv: &[&str]) -> String {
        let mut exec = vec!["exec", self.client.as_str()];
        exec.extend_from_slice(argv);
        docker_ok(&exec)
    }

    fn client_sh(&self, script: &str) -> String {
        self.client(&["sh", "-c", script])
    }

    /// Trusts the host key the sandbox serves only if its fingerprint is the one the engine
    /// reports, the way the front door pins it.
    fn pin_host_key(&self, target: &SshTarget) {
        let SshTarget::SshServer {
            host,
            host_key_fingerprint,
            ..
        } = target
        else {
            panic!("not an SSH server target: {target:?}");
        };
        let scanned = self.client(&["ssh-keyscan", "-t", "ed25519", host]);
        self.client(&[
            "sh",
            "-c",
            "printf '%s\\n' \"$1\" > \"$2\"",
            "sh",
            &scanned,
            KNOWN_HOSTS,
        ]);
        let listed = self.client(&["ssh-keygen", "-lf", KNOWN_HOSTS]);
        assert_eq!(
            listed.split_whitespace().nth(1),
            Some(host_key_fingerprint.as_str()),
            "{listed}"
        );
    }

    fn ssh_args<'a>(host: &'a str, command: &'a str) -> Vec<String> {
        [
            "ssh",
            "-i",
            "/home/sandbox/.ssh/proxy_key",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={KNOWN_HOSTS}"),
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            &format!("sandbox@{host}"),
            command,
        ]
        .map(ToOwned::to_owned)
        .to_vec()
    }

    fn ssh(&self, host: &str, command: &str) -> String {
        let args = Self::ssh_args(host, command);
        self.client(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn ssh_in_background(&self, host: &str, command: &str) {
        let mut args = vec!["exec", "--detach", self.client.as_str()];
        let ssh = Self::ssh_args(host, command);
        args.extend(ssh.iter().map(String::as_str));
        docker_ok(&args);
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
        docker(&["rm", "--force", &self.client]);
        docker(&["network", "rm", &self.network]);
    }
}

async fn state(engine: &DockerEngine, id: &SandboxId) -> SandboxStatus {
    engine.status(id).await.unwrap()
}

async fn wake(engine: &DockerEngine, id: &SandboxId, env: CredentialMap) -> SshTarget {
    wake_with(
        engine,
        id,
        Boot {
            env,
            ..Boot::default()
        },
    )
    .await
    .0
}

/// The target, and whether the start booted the sandbox.
async fn wake_with(engine: &DockerEngine, id: &SandboxId, boot: Boot) -> (SshTarget, bool) {
    let started = engine.start(id, None, boot, WAKE).await.unwrap();
    match started.readiness {
        Readiness::Ready(target) => (target, started.booted),
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

    let target = wake(&engine, &id, CredentialMap::new()).await;
    assert_eq!(state(&engine, &id).await, SandboxStatus::Running);
    let SshTarget::SshServer {
        host,
        port,
        username,
        ..
    } = &target
    else {
        panic!("not an SSH server target: {target:?}");
    };
    assert_eq!(
        (host.as_str(), *port, username.as_str()),
        (name.as_str(), 22, "sandbox")
    );
    lab.pin_host_key(&target);
    assert_eq!(lab.ssh(host, "id -un"), "sandbox");
    assert_eq!(
        lab.ssh(host, "env | grep ^CORRAL_; true"),
        "",
        "no CORRAL_ variable reaches the session"
    );
    lab.ssh(host, "echo kept > ~/persist-check");

    engine.stop(&id).await.unwrap();
    assert_eq!(state(&engine, &id).await, SandboxStatus::Sleeping);
    engine.stop(&id).await.unwrap();
    assert_eq!(wake(&engine, &id, CredentialMap::new()).await, target);
    assert_eq!(lab.ssh(host, "cat ~/persist-check"), "kept");

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
async fn credentials_apply_at_each_start_and_stay_out_of_the_container_config() {
    let Some(lab) = Lab::open("credentials") else {
        return;
    };
    let engine = lab.engine(10).await;
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    let secret = |word: &str| format!("corral-test-{word}-{}", std::process::id());

    let target = wake(&engine, &id, token(&secret("one"))).await;
    lab.pin_host_key(&target);
    assert_eq!(lab.ssh(&name, "printenv FOO_TOKEN"), secret("one"));
    lab.ssh(&name, "echo kept > ~/persist-check");

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, token(&secret("two"))).await;
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN; cat ~/persist-check"),
        format!("{}\nkept", secret("two")),
        "a start after a stop applies the changed credential and keeps the home"
    );

    wake(&engine, &id, token(&secret("ignored"))).await;
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN"),
        secret("two"),
        "a running sandbox keeps its credentials"
    );

    let odd = r#"p#ss "word" it's $HOME \x\"#;
    engine.stop(&id).await.unwrap();
    wake(&engine, &id, env(&[("ODD_TOKEN", odd)])).await;
    assert_eq!(lab.ssh(&name, "printenv ODD_TOKEN"), odd);
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN; true"),
        "",
        "a removed credential is gone"
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
    let name = container_name(&id);
    let boot = |files| Boot {
        env: CredentialMap::new(),
        files: home_files(files),
    };
    let listing = "for f in .config/corral/sandbox.json '.config/corral' 'notes dir/notes.txt'; do \
                   stat -c '%a %U %G %n' ~/\"$f\"; done; cat ~/.config/corral/sandbox.json";

    let (target, booted) = wake_with(
        &engine,
        &id,
        boot(&[
            (".config/corral/sandbox.json", 0o600, "first"),
            ("notes dir/notes.txt", 0o644, "notes"),
        ]),
    )
    .await;
    assert!(booted);
    lab.pin_host_key(&target);
    assert_eq!(
        lab.ssh(&name, listing),
        "600 sandbox sandbox /home/sandbox/.config/corral/sandbox.json\n\
         755 sandbox sandbox /home/sandbox/.config/corral\n\
         644 sandbox sandbox /home/sandbox/notes dir/notes.txt\n\
         first"
    );
    lab.ssh(
        &name,
        "echo edited > ~/.config/corral/sandbox.json && echo mine > ~/kept",
    );

    let (_, booted) = wake_with(
        &engine,
        &id,
        boot(&[(".config/corral/sandbox.json", 0o600, "ignored")]),
    )
    .await;
    assert!(!booted, "a running sandbox is not booted again");
    assert_eq!(
        lab.ssh(&name, "cat ~/.config/corral/sandbox.json"),
        "edited"
    );

    engine.stop(&id).await.unwrap();
    let (_, booted) = wake_with(
        &engine,
        &id,
        boot(&[(".config/corral/sandbox.json", 0o600, "second")]),
    )
    .await;
    assert!(booted);
    assert_eq!(
        lab.ssh(
            &name,
            "cat ~/.config/corral/sandbox.json ~/kept ~/'notes dir/notes.txt'"
        ),
        "secondmine\nnotes",
        "a start rewrites the files, and leaves the rest of the home alone"
    );
    assert_eq!(
        lab.ssh(
            &name,
            "stat -c %a /etc/corral/home-files; cat /etc/corral/home-files/manifest"
        ),
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
async fn a_start_from_stopped_remakes_an_outdated_container_keeping_home_and_host_key() {
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
    let target = wake(&engine, &id, CredentialMap::new()).await;
    lab.pin_host_key(&target);
    lab.ssh(&name, "echo kept > ~/kept");
    engine.stop(&id).await.unwrap();

    // What an engine from before this one made: no definition label, and an env var it dropped.
    let env: Vec<String> = serde_json::from_str(&inspect(&name, "{{json .Config.Env}}")).unwrap();
    let host_key = env
        .iter()
        .find(|entry| entry.starts_with("CORRAL_SSH_HOST_KEY="))
        .unwrap();
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
        format!("--env={host_key}"),
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

    let (again, booted) = wake_with(&engine, &id, Boot::default()).await;
    assert!(booted);
    assert_eq!(again, target, "the same name and host key");
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");
    let env = inspect(&name, "{{json .Config.Env}}");
    assert!(!env.contains("CORRAL_RETIRED"), "{env}");
    let remade = inspect(&name, "{{.Id}}");

    tags.tag(&newer, &current);
    let (_, booted) = wake_with(&engine, &id, Boot::default()).await;
    assert!(!booted);
    assert_eq!(
        inspect(&name, "{{.Id}}"),
        remade,
        "a running sandbox is left as it is"
    );

    engine.stop(&id).await.unwrap();
    assert_eq!(wake(&engine, &id, CredentialMap::new()).await, target);
    assert_eq!(
        inspect(&name, "{{.Image}}"),
        newer_id,
        "the next start takes the image its reference names now"
    );
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");
    let remade = inspect(&name, "{{.Id}}");

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, CredentialMap::new()).await;
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
    let target = wake(&engine, &id, CredentialMap::new()).await;
    lab.pin_host_key(&target);
    lab.ssh(&name, "echo kept > ~/kept");

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
    assert_eq!(started.readiness, Readiness::Ready(target));
    assert_eq!(
        (
            inspect(&name, "{{.Image}}"),
            inspect(&name, "{{.HostConfig.Memory}}")
        ),
        (newer_id, (2u64 << 30).to_string())
    );
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");
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
    let target = wake(&engine, &id, CredentialMap::new()).await;
    lab.pin_host_key(&target);
    lab.ssh(&name, "echo kept > ~/kept");
    let first = inspect(&name, "{{.Id}}");
    let mounts = inspect(&name, "{{json .HostConfig.Mounts}}");
    assert!(mounts.contains(&overlay_id), "{mounts}");

    let upgraded = lab.engine_with(10, &newer).await;
    wake(&upgraded, &id, CredentialMap::new()).await;
    assert_eq!(
        inspect(&name, "{{.Id}}"),
        first,
        "a running sandbox keeps its overlay"
    );
    upgraded.stop(&id).await.unwrap();
    assert_eq!(wake(&upgraded, &id, CredentialMap::new()).await, target);
    let mounts = inspect(&name, "{{json .HostConfig.Mounts}}");
    assert!(mounts.contains(&newer_id), "{mounts}");
    assert_ne!(inspect(&name, "{{.Id}}"), first);
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");
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
    let target = wake(&engine, &id, CredentialMap::new()).await;
    lab.pin_host_key(&target);
    lab.ssh(&name, "echo kept > ~/kept");
    engine.stop(&id).await.unwrap();

    // Cut short before the old container went: its replacement is left over.
    tags.tag(&newer, &current);
    docker_ok(&["create", "--name", &replacement, &image()]);
    assert_eq!(wake(&engine, &id, CredentialMap::new()).await, target);
    assert_eq!(inspect(&name, "{{.Image}}"), newer_id);
    assert!(!exists("container", &replacement));
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");

    // Cut short after: the complete replacement is all there is.
    engine.stop(&id).await.unwrap();
    docker_ok(&["rename", &name, &replacement]);
    assert_eq!(wake(&engine, &id, CredentialMap::new()).await, target);
    assert!(!exists("container", &replacement));
    assert_eq!(lab.ssh(&name, "cat ~/kept"), "kept");

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
async fn idle_sandboxes_sleep_and_busy_ones_stay_awake() {
    let Some(lab) = Lab::open("reaper") else {
        return;
    };
    let timeout = Duration::from_secs(3);
    let engine = lab.engine(10).await;
    let idle = lab.create(&engine, "idle").await;
    let ssh = lab.create(&engine, "ssh").await;
    let herdr = lab.create(&engine, "herdr").await;
    let mut targets = Vec::new();
    for id in [&idle, &ssh, &herdr] {
        targets.push(wake(&engine, id, CredentialMap::new()).await);
    }

    lab.pin_host_key(&targets[1]);
    lab.ssh_in_background(&container_name(&ssh), "sleep 120");
    let started = Instant::now();
    while !docker_ok(&[
        "exec",
        &container_name(&ssh),
        "/opt/corral/bin/busybox",
        "ps",
        "-o",
        "args",
    ])
    .lines()
    .any(|args| args.starts_with("sshd-session: sandbox@"))
    {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the SSH session never started"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    docker_ok(&[
        "exec",
        "--user",
        "1000",
        &container_name(&herdr),
        "/opt/corral/bin/hc",
        "run",
        "--bg",
        "sleep",
        "600",
    ]);

    let shutdown = CancellationToken::new();
    let lifecycle = corral_activity::spawn(
        engine.clone(),
        Duration::from_secs(1),
        timeout,
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
        state(&engine, &ssh).await,
        SandboxStatus::Running,
        "an SSH session keeps it awake"
    );
    assert_eq!(
        state(&engine, &herdr).await,
        SandboxStatus::Running,
        "a Herdr terminal keeps it awake"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), lifecycle)
        .await
        .expect("the lifecycle stops on shutdown")
        .unwrap();
}

#[tokio::test]
async fn connect_refuses_a_missing_network() {
    if !docker_answers(&["info"]) {
        assert!(std::env::var_os("CI").is_none(), "CI needs a Docker daemon");
        eprintln!("skipped: no Docker daemon");
        return;
    }
    let key =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone();
    let missing = format!("corral-test-docker-missing-{}", std::process::id());
    let refused = DockerEngine::connect(
        DockerConfig {
            network: missing.clone(),
            ..DockerConfig::default()
        },
        key,
        1,
    )
    .await;
    assert!(
        matches!(&refused, Err(error @ Error::Internal { .. }) if error.to_string().contains(&missing)),
        "{:?}",
        refused.err()
    );
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

    wake(&engine, &id, CredentialMap::new()).await;
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
