use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use tokio::{
    fs,
    process::{Child, Command},
    sync::{RwLock, watch},
    time::sleep,
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{error, info, warn};

const DEFAULT_CAPTURE_COMMAND: &str = "rpicam-vid --camera 0 --timeout 0 --width 1280 \
    --height 720 --framerate 15 --codec h264 --inline --intra 15 --bitrate 2500000 \
    --nopreview --output -";

#[derive(Clone, Debug)]
struct Config {
    bind: SocketAddr,
    hls_dir: PathBuf,
    web_dir: PathBuf,
    capture_command: Vec<String>,
    ffmpeg_bin: String,
    frame_rate: u32,
    revision: String,
}

impl Config {
    fn from_env() -> Result<Self> {
        let bind = env::var("MONITOR_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".into())
            .parse()
            .context("MONITOR_BIND must be an IP address and port")?;
        let hls_dir = env::var_os("MONITOR_HLS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/pi-camera-monitor/hls"));
        let web_dir = env::var_os("MONITOR_WEB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./web/dist"));
        let capture_command = parse_command(
            &env::var("MONITOR_CAPTURE_COMMAND").unwrap_or_else(|_| DEFAULT_CAPTURE_COMMAND.into()),
        )?;
        let ffmpeg_bin = env::var("MONITOR_FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into());
        let frame_rate = env::var("MONITOR_FRAME_RATE")
            .unwrap_or_else(|_| "15".into())
            .parse()
            .context("MONITOR_FRAME_RATE must be a positive integer")?;
        if frame_rate == 0 {
            bail!("MONITOR_FRAME_RATE must be greater than zero");
        }
        let revision = env::var("MONITOR_REVISION").unwrap_or_else(|_| "dev".into());

        Ok(Self {
            bind,
            hls_dir,
            web_dir,
            capture_command,
            ffmpeg_bin,
            frame_rate,
            revision,
        })
    }
}

#[derive(Clone)]
struct AppState {
    hls_dir: PathBuf,
    revision: String,
    stream: Arc<RwLock<StreamStatus>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    #[serde(flatten)]
    stream: StreamStatus,
    revision: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamStatus {
    phase: StreamPhase,
    message: Option<String>,
    started_at: Option<u64>,
    restarts: u64,
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

    let state = AppState {
        hls_dir: config.hls_dir.clone(),
        revision: config.revision.clone(),
        stream: Arc::new(RwLock::new(StreamStatus::default())),
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let supervisor = tokio::spawn(supervise_pipeline(
        config.clone(),
        state.clone(),
        shutdown_rx,
    ));

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/hls/stream.m3u8", get(playlist))
        .route("/hls/{segment}", get(segment))
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
    Ok(())
}

fn status_response(state: &AppState, stream: StreamStatus) -> StatusResponse {
    StatusResponse {
        revision: state.revision.clone(),
        stream,
    }
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    Json(status_response(&state, state.stream.read().await.clone()))
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let stream = state.stream.read().await.clone();
    let code = if stream.phase == StreamPhase::Live {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(status_response(&state, stream)))
}

async fn playlist(State(state): State<AppState>) -> Response {
    file_response(
        state.hls_dir.join("stream.m3u8"),
        "application/vnd.apple.mpegurl",
    )
    .await
}

async fn segment(State(state): State<AppState>, AxumPath(segment): AxumPath<String>) -> Response {
    if !valid_segment_name(&segment) {
        return StatusCode::NOT_FOUND.into_response();
    }
    file_response(state.hls_dir.join(segment), "video/mp2t").await
}

async fn file_response(path: PathBuf, content_type: &'static str) -> Response {
    match fs::read(path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => {
            error!(%error, "could not read HLS file");
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
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut restarts = 0;

    while !*shutdown.borrow() {
        clear_hls_dir(&config.hls_dir).await?;
        update_status(
            &state,
            StreamPhase::Starting,
            Some("Starting camera".into()),
            restarts,
        )
        .await;

        let (mut capture, mut ffmpeg) = match spawn_pipeline(&config) {
            Ok(children) => children,
            Err(error) => {
                error!(%error, "failed to start capture pipeline");
                update_status(
                    &state,
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
                    if state.stream.read().await.phase != StreamPhase::Live
                        && fs::metadata(config.hls_dir.join("stream.m3u8")).await.is_ok()
                    {
                        update_status(&state, StreamPhase::Live, None, restarts).await;
                        info!("camera stream is live");
                    }

                    let capture_exit = capture.try_wait().context("check camera process")?;
                    let ffmpeg_exit = ffmpeg.try_wait().context("check ffmpeg process")?;
                    if capture_exit.is_some() || ffmpeg_exit.is_some() {
                        let message = format!(
                            "Pipeline exited (camera: {capture_exit:?}, ffmpeg: {ffmpeg_exit:?})"
                        );
                        warn!("{message}");
                        stop_children(&mut capture, &mut ffmpeg).await;
                        update_status(
                            &state,
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

fn spawn_pipeline(config: &Config) -> Result<(Child, Child)> {
    let (program, args) = config
        .capture_command
        .split_first()
        .context("capture command is empty")?;
    let (camera_output, camera_input) = os_pipe::pipe().context("create camera-to-ffmpeg pipe")?;
    let capture = Command::new(program)
        .args(args)
        .stdout(Stdio::from(camera_input))
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("start camera command {program}"))?;
    let playlist = config.hls_dir.join("stream.m3u8");
    let segment_pattern = config.hls_dir.join("segment-%06d.ts");

    let ffmpeg = Command::new(&config.ffmpeg_bin)
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
        .stdin(Stdio::from(camera_output))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("start HLS muxer {}", config.ffmpeg_bin))?;

    Ok((capture, ffmpeg))
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
    let _ = capture.kill().await;
    let _ = ffmpeg.kill().await;
    let _ = capture.wait().await;
    let _ = ffmpeg.wait().await;
}

async fn update_status(
    state: &AppState,
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
        .or_else(|| {
            state
                .stream
                .try_read()
                .ok()
                .and_then(|status| status.started_at)
        });
    *state.stream.write().await = StreamStatus {
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

fn parse_command(value: &str) -> Result<Vec<String>> {
    let command = shell_words::split(value).context("parse MONITOR_CAPTURE_COMMAND")?;
    if command.is_empty() {
        bail!("MONITOR_CAPTURE_COMMAND cannot be empty");
    }
    Ok(command)
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
    fn parses_quoted_capture_command() {
        assert_eq!(
            parse_command("capture --label 'front camera'").unwrap(),
            ["capture", "--label", "front camera"]
        );
    }

    #[test]
    fn rejects_empty_capture_command() {
        assert!(parse_command("  ").is_err());
    }

    #[test]
    fn accepts_only_generated_transport_stream_names() {
        assert!(valid_segment_name("segment-000042.ts"));
        assert!(!valid_segment_name("../secret.ts"));
        assert!(!valid_segment_name("segment-current.m3u8"));
        assert!(!valid_segment_name("segment-.ts"));
    }

    #[test]
    fn status_payload_includes_revision() {
        let payload = StatusResponse {
            stream: StreamStatus::default(),
            revision: "abc1234-dirty".into(),
        };
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["revision"], "abc1234-dirty");
        assert_eq!(json["phase"], "starting");
        assert_eq!(json["restarts"], 0);
    }
}
