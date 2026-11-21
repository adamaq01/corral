use corral_secrets_openbao::{OpenBaoAuth, OpenBaoConfig};
use secrecy::SecretString;
use testcontainers::core::{Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

pub const MOUNT: &str = "secret";
pub const PREFIX: &str = "corral";
const ROOT_TOKEN: &str = "root";

pub struct OpenBao {
    pub address: String,
    pub root: VaultClient,
    _container: ContainerAsync<GenericImage>,
}

impl OpenBao {
    pub async fn start() -> Self {
        let container = GenericImage::new("openbao/openbao", "2.7.3")
            .with_exposed_port(8200.tcp())
            .with_wait_for(WaitFor::message_on_stderr("upgrading keys finished"))
            .with_env_var("BAO_DEV_ROOT_TOKEN_ID", ROOT_TOKEN)
            .with_host("host.docker.internal", Host::HostGateway)
            .start()
            .await
            .expect("openbao container starts");
        let host = container.get_host().await.expect("container host");
        let port = container
            .get_host_port_ipv4(8200.tcp())
            .await
            .expect("published port");
        let address = format!("http://{host}:{port}");
        let root = VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&address)
                .token(ROOT_TOKEN)
                .build()
                .expect("root client settings"),
        )
        .expect("root client");
        Self {
            address,
            root,
            _container: container,
        }
    }

    pub fn config(&self, auth: OpenBaoAuth) -> OpenBaoConfig {
        OpenBaoConfig {
            address: self.address.clone(),
            mount: MOUNT.into(),
            path_prefix: PREFIX.into(),
            auth,
        }
    }

    pub fn root_config(&self) -> OpenBaoConfig {
        self.config(OpenBaoAuth::Token(SecretString::from(ROOT_TOKEN)))
    }
}
