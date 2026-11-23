use crate::{Error, Result};

const DOCKER_HUB: &str = "docker.io";
const DIGEST: &str = "@sha256:";

/// An image a user names, written out in full: its registry, its repository and a tag or a
/// digest, as `docker.io/library/debian:13` for `debian:13`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageReference(String);

impl ImageReference {
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = || {
            Error::Invalid(format!(
                "{text:?} is not an image reference, such as docker.io/library/debian:13"
            ))
        };
        let (named, digest) = match text.split_once(DIGEST) {
            Some((named, hex)) => {
                let hex_digest = hex.len() == 64
                    && hex
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() && b <= b'f');
                if !hex_digest {
                    return Err(invalid());
                }
                (named, Some(hex))
            }
            None => (text, None),
        };
        let (name, tag) = match named.rsplit_once(':') {
            Some((name, tag)) if !tag.contains('/') => (name, Some(tag)),
            _ => (named, None),
        };
        let (registry, path) = match name.split_once('/') {
            Some((first, rest)) if first.contains(['.', ':']) || first == "localhost" => {
                (first.to_ascii_lowercase(), rest.to_owned())
            }
            Some(_) => (DOCKER_HUB.to_owned(), name.to_owned()),
            None => (DOCKER_HUB.to_owned(), format!("library/{name}")),
        };
        let registry_ok = registry
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b));
        let path_ok = path.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        });
        let tag_ok = tag.is_none_or(|tag| {
            (1..=128).contains(&tag.len())
                && !tag.starts_with(['.', '-'])
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        });
        if !(registry_ok && path_ok && tag_ok) {
            return Err(invalid());
        }
        let mut full = format!("{registry}/{path}");
        if let Some(tag) = tag {
            full.push(':');
            full.push_str(tag);
        }
        match digest {
            Some(hex) => {
                full.push_str(DIGEST);
                full.push_str(hex);
            }
            None if tag.is_none() => full.push_str(":latest"),
            None => {}
        }
        Ok(Self(full))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The registry's host, with its port if it has one.
    #[must_use]
    pub fn registry(&self) -> &str {
        self.0
            .split_once('/')
            .map_or(&self.0, |(registry, _)| registry)
    }

    /// Whether the image is in `place`: a registry, such as `ghcr.io`, or a repository path in
    /// one, such as `ghcr.io/acme`.
    #[must_use]
    pub fn is_in(&self, place: &str) -> bool {
        let place = place.trim_end_matches('/');
        let repository = self.repository();
        repository == place
            || repository
                .strip_prefix(place)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    fn repository(&self) -> &str {
        let named = self
            .0
            .split_once(DIGEST)
            .map_or(self.0.as_str(), |(named, _)| named);
        match named.rsplit_once(':') {
            Some((name, tag)) if !tag.contains('/') => name,
            _ => named,
        }
    }
}

impl std::fmt::Display for ImageReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ImageReference;

    fn full(text: &str) -> String {
        ImageReference::parse(text).unwrap().to_string()
    }

    #[test]
    fn a_reference_is_written_out_in_full() {
        let digest = format!("sha256:{}", "a".repeat(64));
        for (text, expected) in [
            ("debian", "docker.io/library/debian:latest"),
            ("debian:13", "docker.io/library/debian:13"),
            ("acme/tools:1.2", "docker.io/acme/tools:1.2"),
            ("ghcr.io/acme/tools", "ghcr.io/acme/tools:latest"),
            ("GHCR.io/acme/tools:v2", "ghcr.io/acme/tools:v2"),
            (
                "registry.lan:5000/team/img:dev",
                "registry.lan:5000/team/img:dev",
            ),
            ("localhost/img", "localhost/img:latest"),
            (
                &format!("debian@{digest}"),
                &format!("docker.io/library/debian@{digest}"),
            ),
            (
                &format!("ghcr.io/a/b:1@{digest}"),
                &format!("ghcr.io/a/b:1@{digest}"),
            ),
        ] {
            assert_eq!(full(text), expected, "{text}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for text in [
            "",
            "Debian",
            "debian:",
            "debian:-x",
            "ghcr.io//b",
            "ghcr.io/a/b@sha256:abc",
            "a b",
            "ghcr.io/a/b:1/2",
            "registry.lan/a/b;rm",
        ] {
            assert!(ImageReference::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn an_image_is_in_its_registry_and_its_repository_paths() {
        let image = ImageReference::parse("ghcr.io/acme/tools/cli:1").unwrap();
        assert_eq!(image.registry(), "ghcr.io");
        for place in [
            "ghcr.io",
            "ghcr.io/acme",
            "ghcr.io/acme/",
            "ghcr.io/acme/tools/cli",
        ] {
            assert!(image.is_in(place), "{place}");
        }
        for place in [
            "ghcr.i",
            "ghcr.io/acm",
            "docker.io",
            "ghcr.io/acme/tools/cli:1",
        ] {
            assert!(!image.is_in(place), "{place}");
        }
        let hub = ImageReference::parse("debian").unwrap();
        assert!(hub.is_in("docker.io") && hub.is_in("docker.io/library"));
        assert_eq!(
            ImageReference::parse("registry.lan:5000/a")
                .unwrap()
                .registry(),
            "registry.lan:5000"
        );
    }
}
