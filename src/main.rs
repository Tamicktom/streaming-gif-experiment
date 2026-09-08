//! HTTP entrypoint: thin Axum handlers over `GifStream`.

//* Libraries imports
use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;

//* Local imports
mod frames;
mod gif_stream;

use gif_stream::{GifStream, TrailerPolicy};

const BIND_ADDR: &str = "127.0.0.1:3000";
const INDEX_HTML: &str = include_str!("../static/index.html");

/// Runtime knobs for `/live.gif` (shared via Axum state).
#[derive(Clone)]
pub struct StreamConfig {
    pub interval: Duration,
    pub policy: TrailerPolicy,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            policy: TrailerPolicy::AfterN(10),
        }
    }
}

#[tokio::main]
async fn main() {
    let config = stream_config_from_env();
    let policy_label = format_trailer_policy(config.policy);
    let interval_ms = config.interval.as_millis();
    let app = app(config);
    let addr: SocketAddr = BIND_ADDR.parse().expect("valid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {BIND_ADDR}: {err}"));
    eprintln!(
        "listening on http://{BIND_ADDR}/ trailer={policy_label} interval={interval_ms}ms"
    );
    axum::serve(listener, app)
        .await
        .expect("server error");
}

fn format_trailer_policy(policy: TrailerPolicy) -> String {
    match policy {
        TrailerPolicy::Never => "never".to_string(),
        TrailerPolicy::OnDrop => "ondrop".to_string(),
        TrailerPolicy::AfterN(n) => format!("after:{n}"),
    }
}

/// Application router (also used by tests).
pub fn app(config: StreamConfig) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/live.gif", get(live_gif))
        .with_state(config)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn live_gif(State(config): State<StreamConfig>) -> Response {
    let stream = GifStream::new(config.interval, config.policy);
    let body = Body::from_stream(stream.into_byte_stream());

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/gif"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    (StatusCode::OK, headers, body).into_response()
}

fn stream_config_from_env() -> StreamConfig {
    let mut config = StreamConfig::default();
    config.policy = trailer_policy_from_env();
    if let Ok(ms) = env::var("GIF_INTERVAL_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            config.interval = Duration::from_millis(ms);
        }
    }
    config
}

fn trailer_policy_from_env() -> TrailerPolicy {
    match env::var("GIF_TRAILER").ok().as_deref() {
        Some("never") => TrailerPolicy::Never,
        Some("ondrop") => TrailerPolicy::OnDrop,
        Some(value) if value.starts_with("after:") => {
            let n = value.trim_start_matches("after:").parse().unwrap_or(10);
            TrailerPolicy::AfterN(n)
        }
        _ => TrailerPolicy::AfterN(10),
    }
}

#[cfg(test)]
mod tests {
    //* Libraries imports
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    //* Local imports
    use super::*;

    mod app_routes {
        use super::*;

        fn test_config() -> StreamConfig {
            StreamConfig {
                interval: Duration::ZERO,
                policy: TrailerPolicy::AfterN(10),
            }
        }

        #[tokio::test]
        async fn index_returns_html_that_references_live_gif() {
            let response = app(test_config())
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert!(html.contains("/live.gif"));
        }

        #[tokio::test]
        async fn live_gif_sets_image_gif_and_no_store_without_content_length() {
            let response = app(test_config())
                .oneshot(
                    Request::builder()
                        .uri("/live.gif")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let headers = response.headers();
            assert_eq!(
                headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
                Some("image/gif")
            );
            assert_eq!(
                headers
                    .get(header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            assert!(
                headers.get(header::CONTENT_LENGTH).is_none(),
                "open/streaming responses must not set Content-Length"
            );

            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(body.starts_with(b"GIF89a"), "body must be a GIF");
            assert_eq!(
                *body.last().unwrap(),
                0x3B,
                "default AfterN(10) must end with trailer"
            );
        }

        #[tokio::test]
        async fn live_gif_never_policy_streams_partial_body_without_trailer() {
            use futures_util::StreamExt;
            use http_body_util::BodyDataStream;

            let config = StreamConfig {
                interval: Duration::ZERO,
                policy: TrailerPolicy::Never,
            };
            let response = app(config)
                .oneshot(
                    Request::builder()
                        .uri("/live.gif")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let headers = response.headers();
            assert_eq!(
                headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
                Some("image/gif")
            );
            assert_eq!(
                headers
                    .get(header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            assert!(
                headers.get(header::CONTENT_LENGTH).is_none(),
                "open/streaming responses must not set Content-Length"
            );

            // Never never ends — take a partial body (header + frames), not collect().
            const CHUNK_COUNT: usize = 13;
            let mut data = BodyDataStream::new(response.into_body());
            let mut bytes = Vec::new();
            for _ in 0..CHUNK_COUNT {
                let frame = data
                    .next()
                    .await
                    .expect("Never response must keep streaming")
                    .expect("frame ok");
                bytes.extend_from_slice(&frame);
            }

            assert!(bytes.starts_with(b"GIF89a"), "body must be a GIF");
            assert_ne!(
                *bytes.last().unwrap(),
                0x3B,
                "Never must not emit trailer in partial body"
            );
        }
    }
}
