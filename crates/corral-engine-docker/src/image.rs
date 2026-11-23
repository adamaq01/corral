use bollard::Docker;
use bollard::query_parameters::CreateImageOptionsBuilder;
use corral_core::{Error, Result};
use futures_util::TryStreamExt;

use crate::docker_error::is_not_found;

/// The ID of the image `reference` names, pulled first if the daemon does not have it.
pub(crate) async fn ensure_present(docker: &Docker, reference: &str) -> Result<String> {
    match docker.inspect_image(reference).await {
        Ok(found) => return id_of(reference, found.id),
        Err(err) if is_not_found(&err) => {}
        Err(err) => {
            return Err(Error::internal(format!("inspect image {reference}"), err));
        }
    }
    let options = CreateImageOptionsBuilder::default()
        .from_image(&with_default_tag(reference))
        .build();
    docker
        .create_image(Some(options), None, None)
        .try_collect::<Vec<_>>()
        .await
        .map_err(|err| Error::internal(format!("pull image {reference}"), err))?;
    let pulled = docker
        .inspect_image(reference)
        .await
        .map_err(|err| Error::internal(format!("inspect image {reference}"), err))?;
    id_of(reference, pulled.id)
}

fn id_of(reference: &str, id: Option<String>) -> Result<String> {
    id.ok_or_else(|| Error::internal(format!("image {reference}"), "has no ID"))
}

// The API pulls every tag of an image named without a tag or digest.
fn with_default_tag(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    if last.contains(':') || last.contains('@') {
        reference.to_owned()
    } else {
        format!("{reference}:latest")
    }
}

#[cfg(test)]
mod tests {
    use super::with_default_tag;

    #[test]
    fn untagged_references_pull_latest_only() {
        for (reference, expected) in [
            ("ubuntu", "ubuntu:latest"),
            (
                "ghcr.io/adamaq01/corral-sandbox",
                "ghcr.io/adamaq01/corral-sandbox:latest",
            ),
            ("localhost:5000/sandbox", "localhost:5000/sandbox:latest"),
            (
                "ghcr.io/adamaq01/corral-sandbox:1.0",
                "ghcr.io/adamaq01/corral-sandbox:1.0",
            ),
            ("localhost:5000/sandbox:dev", "localhost:5000/sandbox:dev"),
            ("ubuntu@sha256:abc", "ubuntu@sha256:abc"),
        ] {
            assert_eq!(with_default_tag(reference), expected);
        }
    }
}
