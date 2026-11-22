use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corral_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ssh_encoding::Decode;
use tokio::process::Command;
use ts_rs::TS;

/// opkssh reads its files from here; the path is compiled into it.
const SERVER_FILES: &str = "/etc/opk";
/// Where `--cache=on` keeps the issuer's JWKS; opkssh uses it only when the running user owns it.
const JWKS_CACHE: &str = "/var/cache/opkssh";
const BINARY: &str = "/usr/local/bin/opkssh";
/// Every login is verified as this principal, which `auth_id` lets in for the client id.
const PRINCIPAL: &str = "corral";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const CERTIFICATE_SUFFIX: &str = "-cert-v01@openssh.com";
const MAX_CERTIFICATE: usize = 16 * 1024;
// Other variables hold Corral's own secrets; verify only needs to trust and reach the issuer.
const PASSED_ENV: [&str; 8] = [
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpksshConfig {
    /// The issuer's client that `opkssh login` uses, the audience of its ID tokens.
    pub client_id: String,
    #[serde(default)]
    pub policy: ExpiryPolicy,
}

/// How long a certificate lasts after `opkssh login`. opkssh's policies that follow the ID token's
/// own expiry are left out: without a refresher, users could not renew ahead of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum ExpiryPolicy {
    #[serde(rename = "12h")]
    Hours12,
    #[default]
    #[serde(rename = "24h")]
    Hours24,
    #[serde(rename = "48h")]
    Hours48,
    #[serde(rename = "1week")]
    Week,
}

impl ExpiryPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hours12 => "12h",
            Self::Hours24 => "24h",
            Self::Hours48 => "48h",
            Self::Week => "1week",
        }
    }

    fn hours(self) -> u32 {
        match self {
            Self::Hours12 => 12,
            Self::Hours24 => 24,
            Self::Hours48 => 48,
            Self::Week => 7 * 24,
        }
    }
}

/// What a user's laptop needs to log in with opkssh, for the connect page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpksshLogin {
    pub issuer: String,
    pub client_id: String,
    /// How long a certificate lasts.
    pub lifetime_hours: u32,
}

/// Verifies opkssh certificates with the official `opkssh verify`.
pub(crate) struct Opkssh {
    binary: PathBuf,
    timeout: Duration,
    policy: ExpiryPolicy,
    login: OpksshLogin,
}

impl Opkssh {
    /// Writes opkssh's server files for `issuer`, and checks that opkssh runs.
    pub(crate) async fn install(issuer: &str, config: OpksshConfig) -> Result<Self> {
        let opkssh = Self::new(PathBuf::from(BINARY), VERIFY_TIMEOUT, issuer, config)?;
        opkssh.write_server_files(Path::new(SERVER_FILES))?;
        own_cache(Path::new(JWKS_CACHE), Path::new(SERVER_FILES));
        opkssh.check_version().await?;
        Ok(opkssh)
    }

    fn new(binary: PathBuf, timeout: Duration, issuer: &str, config: OpksshConfig) -> Result<Self> {
        let field = |name: &str, value: &str, forbidden: &[char]| {
            if value.is_empty()
                || value
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || forbidden.contains(&c))
            {
                Err(Error::Invalid(format!(
                    "opkssh needs a {name} without spaces{}",
                    if forbidden.is_empty() {
                        ""
                    } else {
                        " or quotes"
                    }
                )))
            } else {
                Ok(())
            }
        };
        field("issuer", issuer, &[])?;
        field("client id", &config.client_id, &['"'])?;
        Ok(Self {
            binary,
            timeout,
            policy: config.policy,
            login: OpksshLogin {
                issuer: issuer.to_owned(),
                client_id: config.client_id,
                lifetime_hours: config.policy.hours(),
            },
        })
    }

    pub(crate) fn login(&self) -> &OpksshLogin {
        &self.login
    }

    fn write_server_files(&self, dir: &Path) -> Result<()> {
        let OpksshLogin {
            issuer, client_id, ..
        } = &self.login;
        write_private(
            &dir.join("providers"),
            &format!("{issuer} {client_id} {}\n", self.policy.as_str()),
        )?;
        // Any token for the client id passes; the caller then checks its subject.
        write_private(
            &dir.join("auth_id"),
            &format!("{PRINCIPAL} oidc:\"aud\":\"{client_id}\" {issuer}\n"),
        )
    }

    async fn check_version(&self) -> Result<()> {
        let output = Command::new(&self.binary)
            .arg("--version")
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|err| Error::internal("running opkssh", err))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::internal("running opkssh", output.status.to_string()))
        }
    }

    /// The subject of the person the certificate proves, when opkssh verifies it.
    pub(crate) async fn subject(&self, presented: &str) -> Option<String> {
        let (kind, blob) = certificate(presented)?;
        let mut verify = Command::new(&self.binary);
        verify
            .args(["verify", "--cache=on", "--", PRINCIPAL, blob, kind])
            .env_clear()
            .envs(
                PASSED_ENV
                    .iter()
                    .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = match tokio::time::timeout(self.timeout, verify.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                tracing::error!(%error, "could not run opkssh verify");
                return None;
            }
            Err(_) => {
                tracing::warn!(timeout = ?self.timeout, "opkssh verify took too long");
                return None;
            }
        };
        if !output.status.success() {
            tracing::info!(
                reason = refusal(&output.stderr),
                "opkssh refused a certificate"
            );
            return None;
        }
        token_subject(presented)
    }
}

/// The certificate's type and base64 blob, when `presented` looks like an SSH certificate.
fn certificate(presented: &str) -> Option<(&str, &str)> {
    let mut fields = presented.split_whitespace();
    let (kind, blob) = (fields.next()?, fields.next()?);
    let is_blob = |blob: &str| {
        blob.len() <= MAX_CERTIFICATE
            && blob
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    };
    let is_kind = |kind: &str| {
        kind.ends_with(CERTIFICATE_SUFFIX)
            && kind.starts_with(|c: char| c.is_ascii_alphanumeric())
            && kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'@' | b'.'))
    };
    (is_kind(kind) && is_blob(blob)).then_some((kind, blob))
}

/// The `sub` of the PK token in the certificate's `openpubkey-pkt` extension.
fn token_subject(presented: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Claims {
        sub: String,
    }
    let (_, blob) = certificate(presented)?;
    let token = extension(&STANDARD.decode(blob).ok()?, "openpubkey-pkt")?;
    let payload = token.split(':').next()?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    Some(claims.sub)
}

/// A certificate extension's value. Read by hand because ssh-key refuses certificates valid
/// forever, which opkssh's always are.
fn extension(certificate: &[u8], name: &str) -> Option<String> {
    let mut reader = certificate;
    let kind = String::decode(&mut reader).ok()?;
    // The nonce, then the public key's fields, of the two key types opkssh makes.
    let fields = match kind.as_str() {
        "ssh-ed25519-cert-v01@openssh.com" => 2,
        kind if kind.starts_with("ecdsa-sha2-nistp") => 3,
        _ => return None,
    };
    for _ in 0..fields {
        Vec::<u8>::decode(&mut reader).ok()?;
    }
    u64::decode(&mut reader).ok()?;
    u32::decode(&mut reader).ok()?;
    Vec::<u8>::decode(&mut reader).ok()?;
    Vec::<u8>::decode(&mut reader).ok()?;
    u64::decode(&mut reader).ok()?;
    u64::decode(&mut reader).ok()?;
    Vec::<u8>::decode(&mut reader).ok()?;
    let extensions = Vec::<u8>::decode(&mut reader).ok()?;
    let mut rest = extensions.as_slice();
    while !rest.is_empty() {
        let key = String::decode(&mut rest).ok()?;
        let data = Vec::<u8>::decode(&mut rest).ok()?;
        if key == name {
            return String::decode(&mut data.as_slice()).ok();
        }
    }
    None
}

/// opkssh's reason, from its `Error:` line, kept short and printable: some reasons quote the
/// certificate's bytes.
fn refusal(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .find(|line| line.starts_with("Error:"))
        .or_else(|| text.lines().rev().find(|line| !line.trim().is_empty()));
    line.unwrap_or("no reason given")
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .take(200)
        .collect()
}

/// opkssh refuses files that others can read or write.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    let staged = path.with_extension("new");
    let write = || -> io::Result<()> {
        fs::write(&staged, contents)?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o640))?;
        fs::rename(&staged, path)
    };
    write().map_err(|err| Error::internal(format!("writing {}", path.display()), err))
}

/// Gives the JWKS cache to whoever runs Corral, which is whoever wrote `owned`, so opkssh uses it.
/// The image makes it for the unprivileged user; a deployment running as root takes it over.
fn own_cache(cache: &Path, owned: &Path) {
    let take = || -> io::Result<()> {
        let uid = fs::metadata(owned.join("providers"))?.uid();
        if !cache.exists() {
            fs::create_dir(cache)?;
        }
        if fs::metadata(cache)?.uid() != uid {
            chown(cache, Some(uid), None)?;
        }
        fs::set_permissions(cache, fs::Permissions::from_mode(0o700))
    };
    if let Err(error) = take() {
        tracing::warn!(%error, cache = %cache.display(), "opkssh's JWKS cache is off");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::LazyLock;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{ExpiryPolicy, Opkssh, OpksshConfig, certificate, refusal, token_subject};

    /// From a real `opkssh login` against `VoidAuth`.
    const CERTIFICATE: &str = include_str!("../tests/fixtures/opkssh-cert.pub");
    /// The same PK token on an ed25519 key, from `ssh-keygen -O extension:openpubkey-pkt=...`.
    const ED25519_CERTIFICATE: &str = include_str!("../tests/fixtures/ed25519-cert.pub");
    const SUBJECT: &str = "387ce46c-62d0-4283-ad46-505946081723";
    const ISSUER: &str = "https://auth.example/oidc";
    const PLAIN_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    // Written before any test runs one, so no fork inherits a script still open for writing.
    static FAKES: LazyLock<TempDir> = LazyLock::new(|| {
        let dir = TempDir::new().unwrap();
        for (name, body) in [
            (
                "accepts",
                "printf '%s\\n' \"$@\" >\"$0.args\"; env >\"$0.env\"; exit 0",
            ),
            (
                "refuses",
                "echo noise >&2; echo 'Error: audience does not contain clientID x' >&2; exit 1",
            ),
            ("hangs", "exec sleep 30"),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    });

    fn fake(name: &str) -> PathBuf {
        FAKES.path().join(name)
    }

    fn opkssh(binary: PathBuf) -> Opkssh {
        let config = OpksshConfig {
            client_id: "opkssh".to_owned(),
            policy: ExpiryPolicy::default(),
        };
        Opkssh::new(binary, Duration::from_millis(500), ISSUER, config).unwrap()
    }

    #[tokio::test]
    async fn a_verified_certificate_gives_its_tokens_subject() {
        let opkssh = opkssh(fake("accepts"));
        assert_eq!(opkssh.subject(CERTIFICATE).await.as_deref(), Some(SUBJECT));
        let (kind, blob) = certificate(CERTIFICATE).unwrap();
        let args = fs::read_to_string(fake("accepts.args")).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            ["verify", "--cache=on", "--", "corral", blob, kind]
        );
    }

    #[tokio::test]
    async fn verify_runs_without_corrals_environment() {
        let opkssh = opkssh(fake("accepts"));
        opkssh.subject(CERTIFICATE).await;
        let env = fs::read_to_string(fake("accepts.env")).unwrap();
        assert!(!env.contains("CARGO"), "{env}");
    }

    #[tokio::test]
    async fn a_refused_or_slow_or_missing_verify_vouches_for_nobody() {
        assert_eq!(opkssh(fake("refuses")).subject(CERTIFICATE).await, None);
        let started = Instant::now();
        assert_eq!(opkssh(fake("hangs")).subject(CERTIFICATE).await, None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(opkssh(fake("missing")).subject(CERTIFICATE).await, None);
    }

    #[tokio::test]
    async fn anything_but_a_certificate_never_reaches_opkssh() {
        let opkssh = opkssh(fake("hangs"));
        let blob = certificate(CERTIFICATE).unwrap().1;
        for presented in [
            PLAIN_KEY.to_owned(),
            String::new(),
            "ecdsa-sha2-nistp256-cert-v01@openssh.com".to_owned(),
            format!("--help-cert-v01@openssh.com {blob}"),
            format!("ecdsa-sha2-nistp256-cert-v01@openssh.com -{blob}"),
            format!(
                "ecdsa-sha2-nistp256-cert-v01@openssh.com {}",
                "A".repeat(20_000)
            ),
        ] {
            let started = Instant::now();
            assert_eq!(opkssh.subject(&presented).await, None, "{presented}");
            assert!(
                started.elapsed() < Duration::from_millis(100),
                "{presented}"
            );
        }
    }

    #[test]
    fn a_refusal_is_logged_as_opkssh_s_error_line_and_printable_only() {
        assert_eq!(
            refusal(b"noise\nError: bad sig \x01\xff\nmore\n"),
            "Error: bad sig ??"
        );
        assert_eq!(refusal(b"\n  \n"), "no reason given");
        assert_eq!(refusal(b"just this\n").as_str(), "just this");
        assert_eq!(refusal(&[b'x'; 1000]).len(), 200);
    }

    #[test]
    fn the_subject_is_read_from_the_certificates_pk_token() {
        assert_eq!(token_subject(CERTIFICATE).as_deref(), Some(SUBJECT));
        assert_eq!(token_subject(ED25519_CERTIFICATE).as_deref(), Some(SUBJECT));
        assert_eq!(token_subject(PLAIN_KEY), None);
        let (kind, blob) = certificate(CERTIFICATE).unwrap();
        let mut broken = blob.to_owned();
        broken.replace_range(400..404, "AAAA");
        assert_eq!(token_subject(&format!("{kind} {broken}")), None);
    }

    #[test]
    fn server_files_accept_the_client_id_under_the_issuer_and_only_their_owner_reads_them() {
        let dir = TempDir::new().unwrap();
        opkssh(fake("accepts"))
            .write_server_files(dir.path())
            .unwrap();
        let read = |name: &str| fs::read_to_string(dir.path().join(name)).unwrap();
        assert_eq!(read("providers"), format!("{ISSUER} opkssh 24h\n"));
        assert_eq!(
            read("auth_id"),
            format!("corral oidc:\"aud\":\"opkssh\" {ISSUER}\n")
        );
        for name in ["providers", "auth_id"] {
            let mode = fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o640, "{name}");
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn config_takes_a_client_id_and_one_of_opksshs_fixed_lifetimes() {
        let parse = |toml_text: &str| serde_json::from_str::<OpksshConfig>(toml_text);
        let config = parse(r#"{"client_id": "opkssh"}"#).unwrap();
        assert_eq!(config.policy, ExpiryPolicy::Hours24);
        for (text, hours) in [("12h", 12), ("24h", 24), ("48h", 48), ("1week", 168)] {
            let config = parse(&format!(r#"{{"client_id": "c", "policy": "{text}"}}"#)).unwrap();
            assert_eq!(config.policy.hours(), hours);
            assert_eq!(config.policy.as_str(), text);
        }
        for bad in [
            r#"{"client_id": "c", "policy": "oidc"}"#,
            r#"{"client_id": "c", "policy": "never"}"#,
            r#"{"client_id": "c", "lifetime": "24h"}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        for (issuer, client_id) in [
            (ISSUER, "two words"),
            (ISSUER, "quo\"te"),
            (ISSUER, ""),
            ("a b", "c"),
        ] {
            let config = OpksshConfig {
                client_id: client_id.to_owned(),
                policy: ExpiryPolicy::default(),
            };
            assert!(Opkssh::new(Path::new("x").into(), Duration::ZERO, issuer, config).is_err());
        }
    }
}
