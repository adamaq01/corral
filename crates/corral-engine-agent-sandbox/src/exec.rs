use corral_core::{Error, Result};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::Api;
use kube::api::AttachParams;
use tokio::io::AsyncReadExt;

use crate::error::kube_error;
use crate::manifest::CONTAINER;

pub async fn run(pods: &Api<Pod>, pod: &str, argv: &[&str]) -> Result<(i32, String)> {
    let params = AttachParams::default().container(CONTAINER).stderr(false);
    let mut process = pods
        .exec(pod, argv.to_vec(), &params)
        .await
        .map_err(kube_error)?;
    let mut stdout = Vec::new();
    if let Some(mut reader) = process.stdout() {
        reader
            .read_to_end(&mut stdout)
            .await
            .map_err(|error| Error::internal(format!("exec in {pod}"), error))?;
    }
    let status = match process.take_status() {
        Some(status) => status.await,
        None => None,
    };
    process
        .join()
        .await
        .map_err(|error| Error::internal(format!("exec in {pod}"), error))?;
    let code = exit_code(status.as_ref())
        .map_err(|message| Error::internal(format!("exec in {pod}"), message))?;
    Ok((code, String::from_utf8_lossy(&stdout).into_owned()))
}

fn exit_code(status: Option<&Status>) -> Result<i32, String> {
    let status = status.ok_or("ended without a status")?;
    if status.status.as_deref() == Some("Success") {
        return Ok(0);
    }
    status
        .details
        .iter()
        .flat_map(|details| details.causes.iter().flatten())
        .find(|cause| cause.reason.as_deref() == Some("ExitCode"))
        .and_then(|cause| cause.message.as_deref()?.parse().ok())
        .ok_or_else(|| status.message.clone().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
    use serde_json::json;

    use super::exit_code;

    fn status(value: serde_json::Value) -> Status {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn exit_code_comes_from_the_exec_status() {
        assert_eq!(
            exit_code(Some(&status(json!({"status": "Success"})))),
            Ok(0)
        );
        let failed = status(json!({
            "status": "Failure",
            "message": "command terminated with non-zero exit code: error executing command [sh -c exit 3], exit code 3",
            "reason": "NonZeroExitCode",
            "details": {"causes": [{"reason": "ExitCode", "message": "3"}]},
        }));
        assert_eq!(exit_code(Some(&failed)), Ok(3));
        let broken = status(json!({"status": "Failure", "message": "container not found"}));
        assert_eq!(
            exit_code(Some(&broken)),
            Err("container not found".to_owned())
        );
        assert!(exit_code(None).is_err());
    }
}
