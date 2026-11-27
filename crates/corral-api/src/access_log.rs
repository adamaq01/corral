use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use corral_core::domain::Handle;
use tracing::Level;

use crate::routes;

/// Who made the request, once an extractor learns it.
#[derive(Clone, Default)]
pub(crate) struct Requester(Arc<OnceLock<String>>);

impl Requester {
    pub(crate) fn signed_in(parts: &Parts, handle: &Handle) {
        if let Some(requester) = parts.extensions.get::<Self>() {
            let _ = requester.0.set(handle.to_string());
        }
    }
}

/// Why a request failed, for its log line alone.
#[derive(Clone)]
pub(crate) struct Failure(pub(crate) String);

/// Logs each request once answered, with its route rather than its path where it has one: the
/// API's at info, the web UI's files, probes and a sandbox's activity reports at debug, and a
/// failure at error with its cause. Never a query string, a header or a body.
pub async fn access_log(mut request: Request, next: Next) -> Response {
    let began = Instant::now();
    let method = request.method().clone();
    let path = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |matched| matched.as_str().to_owned(),
    );
    let requester = Requester::default();
    request.extensions_mut().insert(requester.clone());
    let response = next.run(request).await;
    let status = response.status().as_u16();
    let ms = began.elapsed().as_millis();
    let user = requester.0.get().map_or("anonymous", String::as_str);
    let error = response
        .extensions()
        .get::<Failure>()
        .map(|Failure(cause)| cause.as_str());
    macro_rules! answered {
        ($level:expr) => {
            tracing::event!($level, %method, %path, status, ms, %user, error, "answered a request")
        };
    }
    if response.status().is_server_error() {
        answered!(Level::ERROR);
    } else if routes::is_quiet(&path) {
        answered!(Level::DEBUG);
    } else {
        answered!(Level::INFO);
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use corral_core::domain::Role;
    use corral_core::fakes::{Logs, PROVIDER, World};
    use serde_json::json;

    use super::access_log;
    use crate::router;
    use crate::testing::{Browser, api, trusting_provider};

    fn corral(world: &World) -> Router {
        router(api(world), vec![trusting_provider(PROVIDER)])
            .route("/healthz", get(|| async { "ok" }))
            .fallback(|| async { "the web UI" })
            .layer(from_fn(access_log))
    }

    async fn signed_in(app: &Router, handle: &str) -> Browser {
        let mut browser = Browser::new(app, "192.0.2.10:50000");
        let login = json!({ "subject": handle, "username": handle, "role": Role::Member });
        browser
            .post(&format!("/auth/{PROVIDER}/login"), login)
            .await;
        browser
    }

    #[tokio::test]
    async fn each_request_is_one_line_with_its_route_status_time_and_user() {
        let world = World::new();
        let app = corral(&world);
        let logs = Logs::capture();
        let mut anonymous = Browser::new(&app, "192.0.2.10:50000");
        anonymous.get("/api/me?token=a-query-secret").await;
        let mut adam = signed_in(&app, "adam").await;
        adam.post("/api/sandboxes", json!({ "name": "main" })).await;
        adam.get("/api/sandboxes/main").await;
        anonymous.get("/healthz").await;
        anonymous.get("/sandboxes/main").await;

        let lines = logs.of("answered a request");
        let shown: Vec<_> = lines
            .iter()
            .map(|line| {
                let level = line.split_whitespace().next().unwrap();
                let fields = line.split(": answered a request ").nth(1).unwrap();
                let fields: Vec<_> = fields
                    .split(' ')
                    .filter(|field| !field.starts_with("ms="))
                    .collect();
                format!("{level} {}", fields.join(" "))
            })
            .collect();
        assert_eq!(
            shown,
            [
                "INFO method=GET path=/api/me status=401 user=anonymous",
                "INFO method=POST path=/auth/test/login status=200 user=anonymous",
                "INFO method=POST path=/api/sandboxes status=201 user=adam",
                "INFO method=GET path=/api/sandboxes/{name} status=200 user=adam",
                "DEBUG method=GET path=/healthz status=200 user=anonymous",
                "DEBUG method=GET path=/sandboxes/main status=200 user=anonymous",
            ],
            "{lines:#?}"
        );
        assert!(lines.iter().all(|line| line.contains(" ms=")), "{lines:#?}");
        let all = logs.lines().join("\n");
        for private in ["a-query-secret", "corral_session", "\"subject\""] {
            assert!(!all.contains(private), "{private} in {all}");
        }
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error_with_its_cause_on_the_same_line() {
        let world = World::new();
        let app = corral(&world);
        let mut adam = signed_in(&app, "adam").await;
        adam.post("/api/sandboxes", json!({ "name": "main" })).await;
        world.engine.fail_deletes_after(Some(0));
        let logs = Logs::capture();
        let reply = adam.call("DELETE", "/api/sandboxes/main", None).await;
        assert_eq!(reply.status.as_u16(), 500);
        assert_eq!(reply.json(), json!({ "error": "internal error" }));

        let failed = logs.one("answered a request");
        assert!(failed.starts_with("ERROR "), "{failed}");
        for shown in [
            "method=DELETE",
            "path=/api/sandboxes/{name}",
            "status=500",
            "user=adam",
            "error=\"fake engine: delete failed\"",
        ] {
            assert!(failed.contains(shown), "{shown} in {failed}");
        }
        assert_eq!(
            logs.lines()
                .iter()
                .filter(|line| line.contains("delete failed"))
                .count(),
            1,
            "{:#?}",
            logs.lines()
        );
    }
}
