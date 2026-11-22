use corral_secrets_vault::{VaultAuth, VaultConfig};
use secrecy::SecretString;
use testcontainers::core::{Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

pub const MOUNT: &str = "secret";
pub const PREFIX: &str = "corral";
const ROOT_TOKEN: &str = "root";

#[derive(Clone, Copy, Debug)]
pub enum Server {
    OpenBao,
    Vault,
}

#[macro_export]
macro_rules! against_each_server {
    ($($test:ident),* $(,)?) => {$(
        mod $test {
            #[tokio::test]
            async fn openbao() {
                super::$test($crate::support::Server::OpenBao).await;
            }

            #[tokio::test]
            async fn vault() {
                super::$test($crate::support::Server::Vault).await;
            }
        }
    )*};
}

impl Server {
    fn image(self) -> ContainerRequest {
        match self {
            Self::OpenBao => GenericImage::new("openbao/openbao", "2.7.3")
                .with_exposed_port(8200.tcp())
                .with_wait_for(WaitFor::message_on_stderr("upgrading keys finished"))
                .with_env_var("BAO_DEV_ROOT_TOKEN_ID", ROOT_TOKEN),
            Self::Vault => GenericImage::new("hashicorp/vault", "2.2.2")
                .with_exposed_port(8200.tcp())
                .with_wait_for(WaitFor::message_on_stderr("path=secret/"))
                .with_env_var("VAULT_DEV_ROOT_TOKEN_ID", ROOT_TOKEN)
                .with_env_var("SKIP_SETCAP", "1"),
        }
    }
}

type ContainerRequest = testcontainers::ContainerRequest<GenericImage>;

pub struct KvServer {
    pub address: String,
    pub root: VaultClient,
    _container: ContainerAsync<GenericImage>,
}

impl KvServer {
    pub async fn start(server: Server) -> Self {
        let container = server
            .image()
            .with_host("host.docker.internal", Host::HostGateway)
            .start()
            .await
            .unwrap_or_else(|err| panic!("{server:?} container starts: {err}"));
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

    pub fn config(&self, auth: VaultAuth) -> VaultConfig {
        VaultConfig {
            address: self.address.clone(),
            mount: MOUNT.into(),
            path_prefix: PREFIX.into(),
            auth,
        }
    }

    pub fn root_config(&self) -> VaultConfig {
        self.config(VaultAuth::Token {
            token: SecretString::from(ROOT_TOKEN),
        })
    }
}
