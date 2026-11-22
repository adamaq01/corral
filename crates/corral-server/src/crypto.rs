/// The Kubernetes client brings rustls's ring provider and the Vault client its aws-lc-rs one.
/// With both compiled in, rustls picks neither by itself, so the process installs one up front.
pub fn install_provider() {
    // An error means a provider is already installed, which is all this needs.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_kubernetes_client_builds_for_an_https_cluster() {
        super::install_provider();
        let config = kube::Config::new("https://127.0.0.1:6443".parse().unwrap());
        kube::Client::try_from(config).unwrap();
    }
}
