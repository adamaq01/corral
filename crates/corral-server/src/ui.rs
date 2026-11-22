use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use tower_http::services::ServeDir;
use tracing::info;

/// Where the server image puts the web UI.
pub const DEFAULT_DIR: &str = "/usr/share/corral/web";
/// `SvelteKit`'s build output, whose content-hashed files under `immutable/` never change.
const APP_DIR: &str = "/_app/";
const IMMUTABLE_DIR: &str = "/_app/immutable/";

/// The web UI, a single-page app built into a directory. A path that names no file gets the
/// app's `index.html`, so the app routes it.
pub struct Ui {
    dir: PathBuf,
    page: Page,
}

struct Page {
    index: Bytes,
    csp: HeaderValue,
}

impl Ui {
    /// The UI in the configured directory, which must hold one, or else in `DEFAULT_DIR` if it
    /// holds one.
    pub fn find(configured: Option<&Path>) -> Result<Option<Self>> {
        let default = Path::new(DEFAULT_DIR);
        let dir = match configured {
            Some(dir) => dir,
            None if default.join("index.html").is_file() => default,
            None => {
                info!(dir = DEFAULT_DIR, "no web UI, serving the API only");
                return Ok(None);
            }
        };
        let ui = Self::open(dir)?;
        info!(dir = %dir.display(), "serving the web UI");
        Ok(Some(ui))
    }

    pub fn open(dir: &Path) -> Result<Self> {
        let index = dir.join("index.html");
        let html = std::fs::read_to_string(&index)
            .with_context(|| format!("reading the web UI's {}", index.display()))?;
        let csp = HeaderValue::try_from(content_security_policy(&html))
            .context("the web UI's index.html gives an invalid Content-Security-Policy")?;
        Ok(Self {
            dir: dir.to_owned(),
            page: Page {
                index: Bytes::from(html),
                csp,
            },
        })
    }

    pub fn router(self) -> Router {
        let page = Arc::new(self.page);
        let files = ServeDir::new(self.dir)
            .append_index_html_on_directories(false)
            .fallback(get(app_page).with_state(page.clone()));
        Router::new()
            .fallback_service(files)
            .layer(from_fn_with_state(page, headers))
    }
}

/// A missing build file stays a 404, so a stale page fails to load it instead of parsing HTML.
async fn app_page(State(page): State<Arc<Page>>, uri: Uri) -> Response {
    if uri.path().starts_with(APP_DIR) {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(CONTENT_TYPE, "text/html; charset=utf-8")],
        page.index.clone(),
    )
        .into_response()
}

async fn headers(State(page): State<Arc<Page>>, request: Request, next: Next) -> Response {
    let immutable = request.uri().path().starts_with(IMMUTABLE_DIR);
    let mut response = next.run(request).await;
    let status = response.status();
    let cache = if immutable && (status.is_success() || status == StatusCode::NOT_MODIFIED) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(CONTENT_SECURITY_POLICY, page.csp.clone());
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

/// Scripts and everything else from this origin, plus the page's own inline scripts by hash,
/// which is how `SvelteKit` starts the app. Pictures may come from any https origin, since they
/// are the identity provider's.
fn content_security_policy(html: &str) -> String {
    let mut scripts = vec!["'self'".to_owned()];
    scripts.extend(inline_scripts(html).into_iter().map(|script| {
        let hash = STANDARD.encode(Sha256::digest(script.as_bytes()));
        format!("'sha256-{hash}'")
    }));
    [
        "default-src 'self'".to_owned(),
        format!("script-src {}", scripts.join(" ")),
        "style-src 'self' 'unsafe-inline'".to_owned(),
        "img-src 'self' https: data:".to_owned(),
        "object-src 'none'".to_owned(),
        "base-uri 'none'".to_owned(),
        "form-action 'self'".to_owned(),
        "frame-ancestors 'none'".to_owned(),
    ]
    .join("; ")
}

/// The contents of each `<script>` element without a `src`.
fn inline_scripts(html: &str) -> Vec<&str> {
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        rest = &rest[start + "<script".len()..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let attributes = &rest[..open_end];
        rest = &rest[open_end + 1..];
        let Some(close) = rest.find("</script") else {
            break;
        };
        if !attributes.contains("src=") {
            scripts.push(&rest[..close]);
        }
        rest = &rest[close..];
    }
    scripts
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use corral_api::router;
    use corral_api::testing::{Browser, Reply, api, trusting_provider};
    use corral_core::fakes::{PROVIDER, World};
    use serde_json::json;
    use tempfile::TempDir;

    use super::Ui;

    const INDEX: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <link rel="modulepreload" href="/_app/immutable/entry/start.Bx1.js">
    <script type="module" src="/_app/immutable/entry/app.Cy2.js"></script>
  </head>
  <body>
    <div style="display: contents"><script>window.started = true;</script></div>
  </body>
</html>
"#;
    const SCRIPT_HASH: &str = "'sha256-zoVSi18jtsR7IKR3l6ObXCdaKf30QeIkXK8uzVa0DqY='";
    const START: &str = "export const start = 1;";
    const IMMUTABLE: &str = "public, max-age=31536000, immutable";

    fn built_ui() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, contents: &str| {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        write("index.html", INDEX);
        write("_app/immutable/entry/start.Bx1.js", START);
        write("_app/version.json", r#"{"version":"1"}"#);
        write("robots.txt", "User-agent: *\n");
        dir
    }

    fn corral(dir: &Path) -> Router {
        let ui = Ui::open(dir).unwrap();
        router(api(&World::new()), vec![trusting_provider(PROVIDER)]).fallback_service(ui.router())
    }

    fn browser(app: &Router) -> Browser {
        Browser::new(app, "192.0.2.10:50000")
    }

    fn header(reply: &Reply, name: header::HeaderName) -> &str {
        reply.headers[name].to_str().unwrap()
    }

    fn assert_app_page(reply: &Reply, uri: &str) {
        assert_eq!(reply.status, StatusCode::OK, "{uri}");
        assert_eq!(reply.text(), INDEX, "{uri}");
        assert_eq!(
            header(reply, header::CONTENT_TYPE),
            "text/html; charset=utf-8"
        );
        assert_eq!(header(reply, header::CACHE_CONTROL), "no-cache", "{uri}");
    }

    #[tokio::test]
    async fn every_path_outside_the_api_and_the_build_gets_the_app_page() {
        let dir = built_ui();
        let app = corral(dir.path());
        let mut browser = browser(&app);
        for uri in [
            "/",
            "/sandboxes",
            "/sandboxes/main",
            "/admin/users",
            "/keys/",
            "/sandboxes/main?tab=snippets",
            "/apis",
        ] {
            assert_app_page(&browser.get(uri).await, uri);
        }
        let head = browser
            .send(
                Request::head("/sandboxes/main")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(head.status, StatusCode::OK);
        assert_eq!(head.body, b"");
    }

    #[tokio::test]
    async fn files_are_served_and_only_hashed_build_files_are_cached_for_good() {
        let dir = built_ui();
        let app = corral(dir.path());
        let mut browser = browser(&app);

        let start = browser.get("/_app/immutable/entry/start.Bx1.js").await;
        assert_eq!(start.status, StatusCode::OK);
        assert_eq!(start.text(), START);
        assert_eq!(header(&start, header::CACHE_CONTROL), IMMUTABLE);
        assert!(header(&start, header::CONTENT_TYPE).contains("javascript"));

        for (uri, body) in [
            ("/_app/version.json", r#"{"version":"1"}"#),
            ("/robots.txt", "User-agent: *\n"),
            ("/index.html", INDEX),
        ] {
            let reply = browser.get(uri).await;
            assert_eq!(
                (reply.status, reply.text().as_str()),
                (StatusCode::OK, body),
                "{uri}"
            );
            assert_eq!(header(&reply, header::CACHE_CONTROL), "no-cache", "{uri}");
        }

        for uri in [
            "/_app/immutable/entry/gone.Dz3.js",
            "/_app/nope.json",
            "/_app/",
        ] {
            let missing = browser.get(uri).await;
            assert_eq!(missing.status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(missing.body, b"", "{uri}");
            assert_eq!(header(&missing, header::CACHE_CONTROL), "no-cache", "{uri}");
        }
    }

    #[tokio::test]
    async fn api_and_auth_paths_stay_the_apis_with_json_404s() {
        let dir = built_ui();
        let app = corral(dir.path());
        let mut browser = browser(&app);
        for uri in [
            "/api",
            "/api/nope",
            "/auth",
            "/auth/nope",
            "/auth/test/nope",
        ] {
            let reply = browser.get(uri).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(reply.json(), json!({ "error": "not found" }), "{uri}");
            assert!(!reply.headers.contains_key(header::CONTENT_SECURITY_POLICY));
        }
        let me = browser.get("/api/me").await;
        assert_eq!(me.status, StatusCode::UNAUTHORIZED);
        assert_eq!(me.json(), json!({ "error": "not signed in" }));
        assert_eq!(
            browser.get("/auth/providers").await.json(),
            json!([{ "id": PROVIDER }])
        );
    }

    #[tokio::test]
    async fn the_page_may_run_only_its_own_scripts_and_show_https_pictures() {
        let dir = built_ui();
        let app = corral(dir.path());
        let mut browser = browser(&app);
        for uri in ["/", "/sandboxes/main", "/_app/immutable/entry/start.Bx1.js"] {
            let reply = browser.get(uri).await;
            assert_eq!(
                header(&reply, header::CONTENT_SECURITY_POLICY),
                format!(
                    "default-src 'self'; script-src 'self' {SCRIPT_HASH}; \
                     style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; \
                     object-src 'none'; base-uri 'none'; form-action 'self'; \
                     frame-ancestors 'none'"
                ),
                "{uri}"
            );
            assert_eq!(header(&reply, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
            assert_eq!(header(&reply, header::REFERRER_POLICY), "no-referrer");
        }
    }

    #[test]
    fn a_configured_directory_without_a_ui_is_an_error() {
        let empty = tempfile::tempdir().unwrap();
        let error = Ui::find(Some(empty.path())).err().unwrap();
        assert!(format!("{error:#}").contains("index.html"), "{error:#}");
        assert!(Ui::find(Some(built_ui().path())).unwrap().is_some());
    }
}
