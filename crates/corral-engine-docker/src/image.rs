use bollard::Docker;
use bollard::query_parameters::CreateImageOptionsBuilder;
use corral_core::{Error, Result};
use futures_util::TryStreamExt;
use tracing::warn;

use crate::docker_error::is_not_found;

/// The ID of the image `reference` names, pulled first if the daemon does not have it.
pub(crate) async fn ensure_present(docker: &Docker, reference: &str) -> Result<String> {
    if let Some(id) = local_id(docker, reference).await? {
        return Ok(id);
    }
    pull(docker, reference).await?;
    local_id(docker, reference)
        .await?
        .ok_or_else(|| Error::internal(format!("image {reference}"), "is missing after its pull"))
}

/// The ID of the image `reference` names once pulled again, or of the copy the daemon has when
/// the pull fails.
pub(crate) async fn refresh(docker: &Docker, reference: &str) -> Result<String> {
    let pulled = pull(docker, reference).await;
    match (pulled, local_id(docker, reference).await?) {
        (Ok(()), Some(id)) => Ok(id),
        (Err(error), Some(id)) => {
            warn!(image = reference, %error, "could not pull, using the image the daemon has");
            Ok(id)
        }
        (Err(error), None) => Err(error),
        (Ok(()), None) => Err(Error::internal(
            format!("image {reference}"),
            "is missing after its pull",
        )),
    }
}

async fn local_id(docker: &Docker, reference: &str) -> Result<Option<String>> {
    match docker.inspect_image(reference).await {
        Ok(found) => id_of(reference, found.id).map(Some),
        Err(err) if is_not_found(&err) => Ok(None),
        Err(err) => Err(Error::internal(format!("inspect image {reference}"), err)),
    }
}

async fn pull(docker: &Docker, reference: &str) -> Result<()> {
    let options = CreateImageOptionsBuilder::default()
        .from_image(&with_default_tag(reference))
        .build();
    docker
        .create_image(Some(options), None, None)
        .try_collect::<Vec<_>>()
        .await
        .map(drop)
        .map_err(|err| Error::internal(format!("pull image {reference}"), err))
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
