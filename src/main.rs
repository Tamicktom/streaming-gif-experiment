//! HTTP entrypoint: thin Axum handlers over `GifStream`.

//* Libraries imports
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;

//* Local imports
mod frames;
mod gif_stream;
mod video;

use gif_stream::{GifStream, TrailerPolicy};
use video::FfmpegFrameSource;

const BIND_ADDR: &str = "127.0.0.1:3000";
const INDEX_HTML: &str = include_str!("../static/index.html");
const EXPERIMENT_1_HTML: &str = include_str!("../static/experiment-1.html");
const EXPERIMENT_2_HTML: &str = include_str!("../static/experiment-2.html");
const STYLES_CSS: &str = include_str!("../static/styles.css");
const DEFAULT_VIDEO_PATH: &str = "static/example.mp4";
/// Match ffmpeg `fps=10` unless `GIF_INTERVAL_MS` overrides.
const DEFAULT_VIDEO_INTERVAL_MS: u64 = 16;

/// Runtime knobs for GIF streams (shared via Axum state).
#[derive(Clone)]
pub struct StreamConfig {
    pub interval: Duration,
    pub policy: TrailerPolicy,
    /// Interval/policy for `/video.gif` (defaults: 100 ms, play-once / OnDrop).
    pub video_interval: Duration,
    pub video_policy: TrailerPolicy,
    pub video_path: PathBuf,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            policy: TrailerPolicy::AfterN(10),
            video_interval: Duration::from_millis(DEFAULT_VIDEO_INTERVAL_MS),
            video_policy: TrailerPolicy::OnDrop,
            video_path: PathBuf::from(DEFAULT_VIDEO_PATH),
        }
    }
}

#[tokio::main]
async fn main() {
    let config = stream_config_from_env();
    let policy_label = format_trailer_policy(config.policy);
    let video_policy_label = format_trailer_policy(config.video_policy);
    let interval_ms = config.interval.as_millis();
    let video_interval_ms = config.video_interval.as_millis();
    let video_path = config.video_path.display().to_string();
    let app = app(config);
    let addr: SocketAddr = BIND_ADDR.parse().expect("valid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {BIND_ADDR}: {err}"));
    eprintln!(
        "listening on http://{BIND_ADDR}/ live trailer={policy_label} interval={interval_ms}ms; video trailer={video_policy_label} interval={video_interval_ms}ms path={video_path}"
    );
    axum::serve(listener, app).await.expect("server error");
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
        .route("/experiment-1", get(experiment_1))
        .route("/experiment-2", get(experiment_2))
        .route("/styles.css", get(styles))
        .route("/live.gif", get(live_gif))
        .route("/video.gif", get(video_gif))
        .with_state(config)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn experiment_1() -> Html<&'static str> {
    Html(EXPERIMENT_1_HTML)
}

async fn experiment_2() -> Html<&'static str> {
    Html(EXPERIMENT_2_HTML)
}

async fn styles() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    (StatusCode::OK, headers, STYLES_CSS).into_response()
}

async fn live_gif(State(config): State<StreamConfig>) -> Response {
    let stream = GifStream::new(config.interval, config.policy);
    let body = Body::from_stream(stream.into_byte_stream());

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/gif"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    (StatusCode::OK, headers, body).into_response()
}

async fn video_gif(State(config): State<StreamConfig>) -> Response {
    let source = match FfmpegFrameSource::open(&config.video_path).await {
        Ok(source) => source,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "video file not found: {} (place an MP4 there or set GIF_VIDEO_PATH)",
                    config.video_path.display()
                ),
            )
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to open video source: {err}"),
            )
                .into_response();
        }
    };

    let stream =
        GifStream::from_source(config.video_interval, config.video_policy, Box::new(source));
    let body = Body::from_stream(stream.into_byte_stream());

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/gif"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    (StatusCode::OK, headers, body).into_response()
}

fn stream_config_from_env() -> StreamConfig {
    let mut config = StreamConfig::default();
    config.policy = trailer_policy_from_env().unwrap_or(config.policy);
    config.video_policy = trailer_policy_from_env().unwrap_or(config.video_policy);

    if let Ok(ms) = env::var("GIF_INTERVAL_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            let interval = Duration::from_millis(ms);
            config.interval = interval;
            config.video_interval = interval;
        }
    }

    if let Ok(path) = env::var("GIF_VIDEO_PATH") {
        config.video_path = PathBuf::from(path);
    }

    config
}

/// Parse `GIF_TRAILER` when set; `None` means keep the caller's default.
fn trailer_policy_from_env() -> Option<TrailerPolicy> {
    match env::var("GIF_TRAILER").ok().as_deref() {
        Some("never") => Some(TrailerPolicy::Never),
        Some("ondrop") => Some(TrailerPolicy::OnDrop),
        Some(value) if value.starts_with("after:") => {
            let n = value.trim_start_matches("after:").parse().unwrap_or(10);
            Some(TrailerPolicy::AfterN(n))
        }
        Some(_) | None => None,
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
        use std::process::Command as StdCommand;
        use tempfile::tempdir;

        fn test_config() -> StreamConfig {
            StreamConfig {
                interval: Duration::ZERO,
                policy: TrailerPolicy::AfterN(10),
                video_interval: Duration::ZERO,
                video_policy: TrailerPolicy::AfterN(2),
                video_path: PathBuf::from("static/example.mp4"),
            }
        }

        fn write_tiny_mp4(path: &std::path::Path, frames: u32) {
            let status = StdCommand::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=c=blue:s=32x32:d=2"),
                    "-r",
                    "1",
                    "-frames:v",
                    &frames.to_string(),
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(path)
                .status()
                .expect("spawn ffmpeg");
            assert!(status.success());
        }

        #[tokio::test]
        async fn index_explains_the_project_and_links_to_the_experiments() {
            let html = get_html("/").await;
            assert!(html.contains("Streaming GIF"));
            assert!(html.contains("href=\"/experiment-1\""));
            assert!(html.contains("href=\"/experiment-2\""));
            assert!(
                !html.contains("id=\"live-gif\""),
                "home must not start an open GIF stream"
            );
        }

        #[tokio::test]
        async fn experiment_1_page_loads_the_synthetic_stream_after_document_load() {
            let html = get_html("/experiment-1").await;
            assert!(html.contains("id=\"experiment-1-gif\""));
            assert!(html.contains("/live.gif"));
        }

        #[tokio::test]
        async fn experiment_2_page_loads_the_video_stream_after_document_load() {
            let html = get_html("/experiment-2").await;
            assert!(html.contains("id=\"experiment-2-gif\""));
            assert!(html.contains("/video.gif"));
        }

        #[tokio::test]
        async fn unknown_html_route_is_not_found() {
            let response = app(test_config())
                .oneshot(
                    Request::builder()
                        .uri("/experiment-99")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn styles_css_is_served_as_stylesheet() {
            let response = app(test_config())
                .oneshot(
                    Request::builder()
                        .uri("/styles.css")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                content_type.starts_with("text/css"),
                "expected text/css, got {content_type}"
            );
        }

        async fn get_html(uri: &str) -> String {
            let response = app(test_config())
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            String::from_utf8(body.to_vec()).unwrap()
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
                headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
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
                video_interval: Duration::ZERO,
                video_policy: TrailerPolicy::OnDrop,
                video_path: PathBuf::from("static/example.mp4"),
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
                headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
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

        #[tokio::test]
        async fn video_gif_returns_not_found_when_file_is_missing() {
            let mut config = test_config();
            config.video_path = PathBuf::from("/tmp/missing-streaming-gif-video.mp4");
            let response = app(config)
                .oneshot(
                    Request::builder()
                        .uri("/video.gif")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn video_gif_sets_image_gif_and_no_store_and_starts_with_gif89a() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("tiny.mp4");
            write_tiny_mp4(&path, 2);

            let mut config = test_config();
            config.video_path = path;
            config.video_policy = TrailerPolicy::AfterN(2);
            config.video_interval = Duration::ZERO;

            let response = app(config)
                .oneshot(
                    Request::builder()
                        .uri("/video.gif")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let headers = response.headers();
            assert_eq!(
                headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
                Some("image/gif")
            );
            assert_eq!(
                headers
                    .get(header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            assert!(headers.get(header::CONTENT_LENGTH).is_none());

            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(body.starts_with(b"GIF89a"), "body must be a GIF");
            assert_eq!(*body.last().unwrap(), 0x3B);
        }
    }
}
