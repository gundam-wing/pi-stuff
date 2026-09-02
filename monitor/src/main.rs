mod config;
mod motion;
mod webrtc_hub;

use std::{
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tokio::{
    fs,
    sync::{Mutex, RwLock, watch},
    time::sleep,
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{error, info, warn};

use bytes::Bytes;
use config::Config;
use motion::{EventStore, MotionEvent, run_analyzer, valid_event_id, valid_frame_name};
use webrtc::media::io::h264_reader::H264Reader;
use webrtc_hub::WebRtcHub;

#[derive(Clone)]
pub(crate) struct AppState {
    hls_dir: PathBuf,
    revision: String,
    stream: Arc<RwLock<StreamStatus>>,
    motion: Arc<RwLock<MotionStatus>>,
    store: Arc<EventStore>,
    webrtc: WebRtcHub,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    #[serde(flatten)]
    stream: StreamStatus,
    revision: String,
    motion: MotionStatus,
    events: Vec<MotionEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventsResponse {
    events: Vec<MotionEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamStatus {
    phase: StreamPhase,
    message: Option<String>,
    started_at: Option<u64>,
    restarts: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MotionStatus {
    score: f32,
    threshold: f32,
    detecting: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum StreamPhase {
    Starting,
    Live,
    Offline,
}

impl Default for StreamStatus {
    fn default() -> Self {
        Self {
            phase: StreamPhase::Starting,
            message: None,
            started_at: None,
            restarts: 0,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let config = Config::from_env()?;
    fs::create_dir_all(&config.hls_dir)
        .await
        .with_context(|| format!("create HLS directory {}", config.hls_dir.display()))?;
    let store = Arc::new(
        EventStore::open(
            config.motion.dir.clone(),
            config.motion.max_events,
            config.motion.max_bytes,
        )
        .await?,
    );

    let stream = Arc::new(RwLock::new(StreamStatus::default()));
    let motion = Arc::new(RwLock::new(MotionStatus {
        score: 0.0,
        threshold: config.motion.threshold,
        detecting: false,
    }));
    let webrtc_hub = WebRtcHub::new(config.frame_rate)?;
    let state = AppState {
        hls_dir: config.hls_dir.clone(),
        revision: config.revision.clone(),
        stream: stream.clone(),
        motion: motion.clone(),
        store: store.clone(),
        webrtc: webrtc_hub.clone(),
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let supervisor = tokio::spawn(supervise_pipeline(
        config.clone(),
        stream.clone(),
        webrtc_hub.clone(),
        shutdown_rx.clone(),
    ));
    let analyzer = tokio::spawn(run_analyzer(
        config.clone(),
        stream,
        motion,
        store,
        Arc::new(Mutex::new(())),
        shutdown_tx.subscribe(),
    ));

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/events", get(events))
        .route("/api/webrtc/status", get(webrtc_hub::webrtc_status))
        .route("/api/webrtc/offer", post(webrtc_hub::webrtc_offer))
        .route("/hls/stream.m3u8", get(playlist))
        .route("/hls/{segment}", get(segment))
        .route("/events/{id}/{frame}", get(event_frame))
        .fallback_service(ServeDir::new(&config.web_dir).append_index_html_on_directories(true))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("bind monitor server to {}", config.bind))?;
    info!(
        "monitor listening on http://{} (rev {})",
        config.bind, config.revision
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve monitor")?;

    let _ = shutdown_tx.send(true);
    supervisor.await.context("join pipeline supervisor")??;
    analyzer.await.context("join motion analyzer")??;
    Ok(())
}

fn status_response(
    state: &AppState,
    stream: StreamStatus,
    motion: MotionStatus,
    events: Vec<MotionEvent>,
) -> StatusResponse {
    StatusResponse {
        revision: state.revision.clone(),
        stream,
        motion,
        events,
    }
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    Json(status_payload(&state).await)
}

async fn events(State(state): State<AppState>) -> Json<EventsResponse> {
    Json(EventsResponse {
        events: state.store.list().await,
    })
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let payload = status_payload(&state).await;
    let code = if payload.stream.phase == StreamPhase::Live {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(payload))
}

async fn status_payload(state: &AppState) -> StatusResponse {
    status_response(
        state,
        state.stream.read().await.clone(),
        state.motion.read().await.clone(),
        state.store.list().await,
    )
}

async fn playlist(State(state): State<AppState>) -> Response {
    file_response(
        state.hls_dir.join("stream.m3u8"),
        "application/vnd.apple.mpegurl",
        "no-store",
    )
    .await
}

async fn segment(State(state): State<AppState>, AxumPath(segment): AxumPath<String>) -> Response {
    if !valid_segment_name(&segment) {
        return StatusCode::NOT_FOUND.into_response();
    }
    file_response(state.hls_dir.join(segment), "video/mp2t", "no-store").await
}

async fn event_frame(
    State(state): State<AppState>,
    AxumPath((id, frame)): AxumPath<(String, String)>,
) -> Response {
    if !valid_event_id(&id) || !valid_frame_name(&frame) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(path) = state.store.frame_path(&id, &frame) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    file_response(path, "image/jpeg", "private, max-age=86400, immutable").await
}

async fn file_response(
    path: PathBuf,
    content_type: &'static str,
    cache_control: &'static str,
) -> Response {
    match fs::read(path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(cache_control),
            );
            response
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => {
            error!(%error, "could not read requested file");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn valid_segment_name(name: &str) -> bool {
    name.starts_with("segment-")
        && name.ends_with(".ts")
        && name
            .strip_prefix("segment-")
            .and_then(|value| value.strip_suffix(".ts"))
            .is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            })
}

async fn supervise_pipeline(
    config: Config,
    stream: Arc<RwLock<StreamStatus>>,
    webrtc_hub: WebRtcHub,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut restarts = 0;

    while !*shutdown.borrow() {
        clear_hls_dir(&config.hls_dir).await?;
        update_status(
            &stream,
            StreamPhase::Starting,
            Some("Starting camera".into()),
            restarts,
        )
        .await;

        let (mut capture, mut ffmpeg, fanout) = match spawn_pipeline(&config, webrtc_hub.clone()).await
        {
            Ok(children) => children,
            Err(error) => {
                error!(%error, "failed to start capture pipeline");
                update_status(
                    &stream,
                    StreamPhase::Offline,
                    Some(error.to_string()),
                    restarts,
                )
                .await;
                restarts += 1;
                if wait_or_shutdown(&mut shutdown, Duration::from_secs(2)).await {
                    break;
                }
                continue;
            }
        };

        loop {
            tokio::select! {
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        stop_children(&mut capture, &mut ffmpeg).await;
                        return Ok(());
                    }
                }
                _ = sleep(Duration::from_millis(500)) => {
                    if stream.read().await.phase != StreamPhase::Live
                        && fs::metadata(config.hls_dir.join("stream.m3u8")).await.is_ok()
                    {
                        update_status(&stream, StreamPhase::Live, None, restarts).await;
                        info!("camera stream is live");
                    }

                    let capture_exit = capture.try_wait().context("check camera process")?;
                    let ffmpeg_exit = ffmpeg.try_wait().context("check ffmpeg process")?;
                    if capture_exit.is_some() || ffmpeg_exit.is_some() || fanout.is_finished() {
                        if fanout.is_finished() {
                            let _ = fanout.await;
                        } else {
                            fanout.abort();
                        }
                        let message = format!(
                            "Pipeline exited (camera: {capture_exit:?}, ffmpeg: {ffmpeg_exit:?})"
                        );
                        warn!("{message}");
                        stop_children(&mut capture, &mut ffmpeg).await;
                        update_status(
                            &stream,
                            StreamPhase::Offline,
                            Some(message),
                            restarts,
                        )
                        .await;
                        restarts += 1;
                        break;
                    }
                }
            }
        }

        if wait_or_shutdown(&mut shutdown, Duration::from_secs(2)).await {
            break;
        }
    }

    Ok(())
}

async fn spawn_pipeline(
    config: &Config,
    webrtc_hub: WebRtcHub,
) -> Result<(Child, Child, tokio::task::JoinHandle<Result<()>>)> {
    let (program, args) = config
        .capture_command
        .split_first()
        .context("capture command is empty")?;
    let mut capture = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start camera command {program}"))?;
    let camera_stdout = capture
        .stdout
        .take()
        .context("capture stdout pipe missing")?;
    let playlist = config.hls_dir.join("stream.m3u8");
    let segment_pattern = config.hls_dir.join("segment-%06d.ts");

    let mut ffmpeg = Command::new(&config.ffmpeg_bin)
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-fflags",
            "+genpts+nobuffer",
            "-use_wallclock_as_timestamps",
            "1",
            "-f",
            "h264",
            "-r",
        ])
        .arg(config.frame_rate.to_string())
        .args([
            "-i",
            "pipe:0",
            "-c:v",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_list_size",
            "6",
            "-hls_flags",
            "delete_segments+omit_endlist+independent_segments+program_date_time",
            "-hls_segment_filename",
        ])
        .arg(segment_pattern)
        .arg(playlist)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start HLS muxer {}", config.ffmpeg_bin))?;
    let ffmpeg_stdin = ffmpeg
        .stdin
        .take()
        .context("ffmpeg stdin pipe missing")?;

    let fanout = tokio::task::spawn_blocking(move || fanout_h264(camera_stdout, ffmpeg_stdin, webrtc_hub));

    Ok((capture, ffmpeg, fanout))
}

fn fanout_h264(
    camera_stdout: impl std::io::Read,
    mut ffmpeg_stdin: impl Write,
    webrtc_hub: WebRtcHub,
) -> Result<()> {
    let mut reader = BufReader::new(camera_stdout);
    let mut h264 = H264Reader::new(&mut reader, 1_048_576);

    loop {
        match h264.next_nal() {
            Ok(nal) => {
                ffmpeg_stdin.write_all(&nal.data)?;
                webrtc_hub.publish_nal(Bytes::copy_from_slice(&nal.data));
            }
            Err(error) => {
                if error.to_string().contains("EOF") {
                    break;
                }
                return Err(error.into());
            }
        }
    }

    Ok(())
}

async fn clear_hls_dir(directory: &Path) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .await
        .with_context(|| format!("read HLS directory {}", directory.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "ts")
            || path
                .extension()
                .is_some_and(|extension| extension == "m3u8")
        {
            fs::remove_file(path).await?;
        }
    }
    Ok(())
}

async fn stop_children(capture: &mut Child, ffmpeg: &mut Child) {
    let _ = capture.kill();
    let _ = ffmpeg.kill();
    let _ = capture.wait();
    let _ = ffmpeg.wait();
}

async fn update_status(
    stream: &Arc<RwLock<StreamStatus>>,
    phase: StreamPhase,
    message: Option<String>,
    restarts: u64,
) {
    let started_at = (phase == StreamPhase::Live)
        .then(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .or_else(|| stream.try_read().ok().and_then(|status| status.started_at));
    *stream.write().await = StreamStatus {
        phase,
        message,
        started_at,
        restarts,
    };
}

async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    tokio::select! {
        _ = sleep(duration) => false,
        result = shutdown.changed() => result.is_err() || *shutdown.borrow(),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_generated_transport_stream_names() {
        assert!(valid_segment_name("segment-000042.ts"));
        assert!(!valid_segment_name("../secret.ts"));
        assert!(!valid_segment_name("segment-current.m3u8"));
        assert!(!valid_segment_name("segment-.ts"));
    }

    #[test]
    fn status_payload_includes_revision_and_motion() {
        let payload = StatusResponse {
            stream: StreamStatus::default(),
            revision: "abc1234-dirty".into(),
            motion: MotionStatus {
                score: 0.04,
                threshold: 0.02,
                detecting: true,
            },
            events: Vec::new(),
        };
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["revision"], "abc1234-dirty");
        assert_eq!(json["phase"], "starting");
        assert_eq!(json["restarts"], 0);
        assert!((json["motion"]["threshold"].as_f64().unwrap() - 0.02).abs() < 1e-6);
        assert!(json["motion"]["detecting"].as_bool().unwrap());
        assert_eq!(json["events"], serde_json::json!([]));
    }
}
