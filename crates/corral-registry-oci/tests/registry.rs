use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use corral_core::domain::ImageReference;
use corral_core::ports::{ImageRegistry, RegistryAccess, RegistryLogin, RegistryLogins};
use corral_core::{Error, Result};
use corral_registry_oci::OciRegistry;
use secrecy::SecretString;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const INDEX: &str = "application/vnd.oci.image.index.v1+json";

fn digest(n: u8) -> String {
    format!("sha256:{}", format!("{n:02x}").repeat(32))
}

fn digest_of(body: &Value) -> String {
    let hash = Sha256::digest(body.to_string().as_bytes());
    hash.iter()
        .fold("sha256:".to_owned(), |hex, byte| format!("{hex}{byte:02x}"))
}

fn manifest(layers: &[i64]) -> Value {
    json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST,
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": digest(1), "size": 7},
        "layers": layers.iter().enumerate().map(|(n, size)| json!({
            "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
            "digest": digest(10 + u8::try_from(n).unwrap()),
            "size": size,
        })).collect::<Vec<_>>(),
    })
}

fn served(body: &Value, media_type: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("Content-Type", media_type)
        .set_body_string(body.to_string())
}

/// Logins a test changes while the reader holds them.
struct Logins(Mutex<Vec<RegistryAccess>>);

#[async_trait]
impl RegistryLogins for Logins {
    async fn registries(&self) -> Result<Vec<RegistryAccess>> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn access(server: &MockServer, login: Option<RegistryLogin>) -> Vec<RegistryAccess> {
    vec![RegistryAccess {
        registry: server.address().to_string(),
        login,
        plain_http: true,
    }]
}

fn logins(server: &MockServer, login: Option<RegistryLogin>) -> Arc<Logins> {
    Arc::new(Logins(Mutex::new(access(server, login))))
}

fn image(server: &MockServer, path: &str) -> ImageReference {
    ImageReference::parse(&format!("{}/{path}", server.address())).unwrap()
}

#[tokio::test]
async fn the_download_size_is_the_layers_and_config_of_this_platform() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/team/tools/manifests/1"))
        .respond_with(served(&manifest(&[1000, 2000]), MANIFEST))
        .mount(&server)
        .await;
    let ours = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let theirs = if ours == "arm64" { "amd64" } else { "arm64" };
    let (other, own) = (manifest(&[9999, 9999]), manifest(&[300, 400]));
    let index = json!({
        "schemaVersion": 2,
        "mediaType": INDEX,
        "manifests": [
            {"mediaType": MANIFEST, "digest": digest_of(&other), "size": 1, "platform": {"os": "linux", "architecture": theirs}},
            {"mediaType": MANIFEST, "digest": digest_of(&own), "size": 1, "platform": {"os": "linux", "architecture": ours}},
        ],
    });
    Mock::given(method("GET"))
        .and(path("/v2/team/multi/manifests/2"))
        .respond_with(served(&index, INDEX))
        .mount(&server)
        .await;
    for body in [other, own] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/v2/team/multi/manifests/{}",
                digest_of(&body)
            )))
            .respond_with(served(&body, MANIFEST))
            .mount(&server)
            .await;
    }
    let registry = OciRegistry::new(logins(&server, None));

    assert_eq!(
        registry
            .download_size(&image(&server, "team/tools:1"))
            .await
            .unwrap(),
        3007
    );
    assert_eq!(
        registry
            .download_size(&image(&server, "team/multi:2"))
            .await
            .unwrap(),
        707
    );
    let missing = registry
        .download_size(&image(&server, "team/missing:1"))
        .await;
    assert!(
        matches!(&missing, Err(Error::Invalid(message)) if message.contains("cannot read")),
        "{missing:?}"
    );
}

#[tokio::test]
async fn a_private_registry_is_read_with_the_login_configured_for_it() {
    let server = MockServer::start().await;
    // "corral:s3cret"
    let authorized = header("Authorization", "Basic Y29ycmFsOnMzY3JldA==");
    Mock::given(method("GET"))
        .and(path("/v2/team/private/manifests/1"))
        .and(authorized)
        .respond_with(served(&manifest(&[50]), MANIFEST))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic realm=\"test\""),
        )
        .mount(&server)
        .await;
    let login = RegistryLogin {
        username: "corral".to_owned(),
        password: SecretString::from("s3cret"),
    };
    let image = image(&server, "team/private:1");
    let logins = logins(&server, None);
    let registry = OciRegistry::new(logins.clone());

    let anonymous = registry.download_size(&image).await;
    assert!(matches!(anonymous, Err(Error::Invalid(_))), "{anonymous:?}");
    *logins.0.lock().unwrap() = access(&server, Some(login));
    assert_eq!(registry.download_size(&image).await.unwrap(), 57);
}
