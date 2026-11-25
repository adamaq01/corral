use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use corral_core::app::Network;
use ipnet::IpNet;
use serde::Deserialize;

/// `[frontdoor]` with `kind = "gate"`: the gate's own settings, and the lockout the server keeps
/// for it.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GateConfig {
    /// Where the gate listens for SSH.
    pub listen_addr: SocketAddr,
    /// The SSH host private key, in OpenSSH or PEM form.
    pub host_key_file: PathBuf,
    /// Where the gate reaches the server's internal listener.
    pub server_url: String,
    /// Networks whose connections are never locked out by address.
    pub lockout_exempt_networks: Vec<Cidr>,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            listen_addr: (Ipv4Addr::UNSPECIFIED, 2222).into(),
            host_key_file: PathBuf::from("/etc/corral/ssh_host_key"),
            server_url: "http://corral:8081".to_owned(),
            lockout_exempt_networks: Vec::new(),
        }
    }
}

impl GateConfig {
    /// The network a failed connection from `source` counts against, if any.
    pub(crate) fn lockout_network(&self, source: IpAddr) -> Option<Network> {
        let source = source.to_canonical();
        let exempt = self
            .lockout_exempt_networks
            .iter()
            .any(|network| network.0.contains(&source));
        (!exempt).then(|| Network::from(source))
    }
}

/// A network in CIDR notation, such as `10.42.0.0/16` or `fd00::/8`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct Cidr(IpNet);

impl TryFrom<String> for Cidr {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse().map(Self).map_err(|_| {
            format!("{text:?} is not a network in CIDR notation, such as \"10.42.0.0/16\"")
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use corral_core::app::Network;

    use super::{Cidr, GateConfig};

    fn config(networks: &[&str]) -> GateConfig {
        GateConfig {
            lockout_exempt_networks: networks
                .iter()
                .map(|network| Cidr::try_from((*network).to_owned()).unwrap())
                .collect(),
            ..GateConfig::default()
        }
    }

    fn lockout_network(config: &GateConfig, source: &str) -> Option<Network> {
        config.lockout_network(source.parse().unwrap())
    }

    fn network(address: &str) -> Network {
        Network::from(address.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn every_address_counts_without_exempt_networks() {
        let config = config(&[]);
        assert_eq!(
            lockout_network(&config, "10.42.0.7"),
            Some(network("10.42.0.7"))
        );
        assert_eq!(
            lockout_network(&config, "2001:db8::1"),
            Some(network("2001:db8::1"))
        );
    }

    #[test]
    fn addresses_in_an_exempt_network_count_against_nothing() {
        let config = config(&["10.42.0.0/16", "fd00:42::/48"]);
        for exempt in [
            "10.42.0.7",
            "10.42.255.255",
            "::ffff:10.42.0.7",
            "fd00:42::9",
        ] {
            assert_eq!(lockout_network(&config, exempt), None, "{exempt}");
        }
        for counted in ["10.43.0.7", "192.168.1.20", "fd00:43::9"] {
            assert_eq!(
                lockout_network(&config, counted),
                Some(network(counted)),
                "{counted}"
            );
        }
    }

    #[test]
    fn a_network_with_host_bits_covers_its_whole_prefix() {
        let config = config(&["10.42.0.5/24"]);
        assert_eq!(lockout_network(&config, "10.42.0.200"), None);
        assert_eq!(
            lockout_network(&config, "10.42.1.5"),
            Some(network("10.42.1.5"))
        );
    }

    #[test]
    fn only_cidr_notation_is_accepted() {
        for bad in [
            "10.42.0.0",
            "10.42.0.0/33",
            "fd00::/129",
            "10.42.0/16",
            "pods",
            "",
        ] {
            let error = Cidr::try_from(bad.to_owned()).unwrap_err();
            assert!(error.contains(&format!("{bad:?}")), "{error}");
        }
    }
}
