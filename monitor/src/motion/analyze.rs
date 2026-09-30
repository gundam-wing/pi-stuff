use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use tokio::{
    fs,
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex, RwLock, watch},
    time::timeout,
};
use tracing::{info, warn};

use super::EventStore;
use super::motion_score;
use crate::config::Config;
use crate::{MotionStatus, StreamPhase, StreamStatus, valid_segment_name, wait_or_shutdown};

struct Detector {
    previous: Option<Vec<u8>>,
    in_motion: bool,
    cooldown_until: Instant,
    last_segment: Option<PathBuf>,
}

impl Detector {
    fn new() -> Self {
        Self {
            previous: None,
            in_motion: false,
            cooldown_until: Instant::now(),
            last_segment: None,
        }
    }

    fn reset(&mut self) {
        self.previous = None;
        self.in_motion = false;
        self.last_segment = None;
    }
}

pub(crate) async fn run_analyzer(
    config: Config,
    stream: Arc<RwLock<StreamStatus>>,
    motion: Arc<RwLock<MotionStatus>>,
    store: Arc<EventStore>,
    burst_lock: Arc<Mutex<()>>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut detector = Detector::new();
    let sample_period = Duration::from_millis(1000 / u64::from(config.motion.analysis_fps.max(1)));
    let mut announced = false;

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }

        if !should_detect(&config, &stream).await {
            detector.reset();
            announced = false;
            update_motion(&motion, 0.0, false).await;
            if wait_or_shutdown(&mut shutdown, Duration::from_millis(500)).await {
                return Ok(());
            }
            continue;
        }

        if !announced {
            info!("motion analyzer started");
            announced = true;
            update_motion(&motion, 0.0, true).await;
        }

        match sample_analysis_frame(&config, &mut detector).await {
            Ok(None) => {
                // No new complete segment yet; keep the last published score.
            }
            Ok(Some(frame)) => {
                if let Err(error) = on_frame(
                    &config,
                    &mut detector,
                    &frame,
                    &motion,
                    &store,
                    &burst_lock,
                )
                .await
                {
                    warn!(%error, "motion capture failed");
                }
            }
            Err(error) => {
                warn!(%error, "motion sample failed");
            }
        }

        if wait_or_shutdown(&mut shutdown, sample_period).await {
            return Ok(());
        }
    }
}

async fn on_frame(
    config: &Config,
    detector: &mut Detector,
    frame: &[u8],
    motion: &Arc<RwLock<MotionStatus>>,
    store: &Arc<EventStore>,
    burst_lock: &Arc<Mutex<()>>,
) -> Result<()> {
    let Some(previous) = detector.previous.as_deref() else {
        detector.previous = Some(frame.to_vec());
        update_motion(motion, 0.0, true).await;
        return Ok(());
    };

    let score = motion_score(
        previous,
        frame,
        config.motion.analysis_width as usize,
        config.motion.analysis_height as usize,
        config.motion.pixel_floor,
        config.motion.roi,
    );
    update_motion(motion, score, true).await;

    let triggered = score >= config.motion.threshold;
    let rising_edge = triggered && !detector.in_motion;
    detector.in_motion = triggered;
    detector.previous = Some(frame.to_vec());

    if !rising_edge || Instant::now() < detector.cooldown_until {
        return Ok(());
    }

    let _guard = burst_lock.lock().await;
    let captured = capture_burst(config).await?;
    if captured.is_empty() {
        bail!("burst produced no JPEG frames");
    }
    let captured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let event = store.insert(captured, score, captured_at).await?;
    detector.cooldown_until = Instant::now() + config.motion.cooldown;
    info!(
        id = %event.id,
        score,
        frames = event.frames,
        "captured motion event"
    );
    Ok(())
}

async fn should_detect(config: &Config, stream: &Arc<RwLock<StreamStatus>>) -> bool {
    let status = stream.read().await;
    if status.phase != StreamPhase::Live {
        return false;
    }
    let Some(started_at) = status.started_at else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(started_at) < config.motion.settle.as_secs() {
        return false;
    }
    drop(status);
    fs::metadata(config.hls_dir.join("stream.m3u8"))
        .await
        .is_ok()
}

async fn update_motion(motion: &Arc<RwLock<MotionStatus>>, score: f32, detecting: bool) {
    let mut status = motion.write().await;
    status.score = score;
    status.detecting = detecting;
}

/// Pull one gray analysis frame from the newest complete HLS segment.
///
/// The previous long-lived `ffmpeg -i stream.m3u8` reader regularly stalled on
/// live playlists that use `delete_segments`, leaving motion score stuck at 0.
/// Sampling a finished `.ts` file avoids that hang.
async fn sample_analysis_frame(
    config: &Config,
    detector: &mut Detector,
) -> Result<Option<Vec<u8>>> {
    let Some(segment) = newest_complete_segment(&config.hls_dir).await? else {
        return Ok(None);
    };
    if detector.last_segment.as_ref() == Some(&segment) {
        return Ok(None);
    }

    let tmp = config
        .motion
        .dir
        .join(format!(".analyze-{}", std::process::id()));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).await.ok();
    }
    fs::create_dir_all(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;
    let segment_copy = tmp.join("source.ts");
    let copy_result = fs::copy(&segment, &segment_copy).await;
    if let Err(error) = copy_result {
        let _ = fs::remove_dir_all(&tmp).await;
        return Err(error).with_context(|| {
            format!(
                "copy HLS segment {} for motion analysis",
                segment.display()
            )
        });
    }

    let filter = format!(
        "scale={}:{}:flags=fast_bilinear,format=gray",
        config.motion.analysis_width, config.motion.analysis_height
    );
    let mut child = Command::new(&config.ffmpeg_bin)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&segment_copy)
        .args(["-an", "-vf"])
        .arg(&filter)
        .args(["-frames:v", "1", "-pix_fmt", "gray", "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start motion sample ffmpeg")?;
    let mut stdout = child
        .stdout
        .take()
        .context("motion sample stdout is missing")?;
    let mut frame = vec![0; config.motion.frame_bytes()];
    let read = timeout(Duration::from_secs(3), stdout.read_exact(&mut frame)).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = fs::remove_dir_all(&tmp).await;

    match read {
        Ok(Ok(_)) => {
            detector.last_segment = Some(segment);
            Ok(Some(frame))
        }
        Ok(Err(error)) => Err(error).context("read motion analysis frame"),
        Err(_) => bail!(
            "timed out reading motion analysis frame from {}",
            segment.display()
        ),
    }
}

async fn capture_burst(config: &Config) -> Result<Vec<Vec<u8>>> {
    let Some(segment) = newest_complete_segment(&config.hls_dir).await? else {
        bail!("no complete HLS segment available for a still burst");
    };
    let tmp = config
        .motion
        .dir
        .join(format!(".burst-{}", std::process::id()));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).await.ok();
    }
    fs::create_dir_all(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;

    // Copy out of the live HLS directory first: ffmpeg may delete the segment
    // via delete_segments between playlist parse and still extract.
    let segment_copy = tmp.join("source.ts");
    fs::copy(&segment, &segment_copy)
        .await
        .with_context(|| {
            format!(
                "copy HLS segment {} for still burst",
                segment.display()
            )
        })?;

    let filter = format!(
        "select='eq(n,0)+eq(n,3)+eq(n,6)',scale={}:{}",
        config.motion.burst_width, config.motion.burst_height
    );
    let output = tmp.join("%02d.jpg");
    let status = Command::new(&config.ffmpeg_bin)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&segment_copy)
        .args(["-an", "-vf"])
        .arg(&filter)
        .args([
            "-vsync",
            "0",
            "-frames:v",
            "3",
            "-q:v",
            "5",
            "-start_number",
            "0",
        ])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("extract motion JPEG burst")?;

    let frames = collect_jpegs(&tmp).await;
    let _ = fs::remove_dir_all(&tmp).await;
    let frames = frames?;
    if frames.is_empty() {
        bail!(
            "ffmpeg burst extract produced no frames (status {status}) from {}",
            segment.display()
        );
    }
    if !status.success() {
        warn!(
            %status,
            segment = %segment.display(),
            frames = frames.len(),
            "burst ffmpeg exited non-zero"
        );
    }
    Ok(frames)
}

async fn newest_complete_segment(hls_dir: &Path) -> Result<Option<PathBuf>> {
    let playlist = match fs::read_to_string(hls_dir.join("stream.m3u8")).await {
        Ok(playlist) => playlist,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).context("read HLS playlist for motion stills");
        }
    };
    let mut segments = Vec::new();
    for line in playlist.lines() {
        let line = line.trim();
        if valid_segment_name(line) {
            segments.push(hls_dir.join(line));
        }
    }
    if segments.len() >= 2 {
        Ok(Some(segments[segments.len() - 2].clone()))
    } else {
        Ok(segments.pop())
    }
}

async fn collect_jpegs(dir: &Path) -> Result<Vec<Vec<u8>>> {
    let mut paths = Vec::new();
    let mut entries = fs::read_dir(dir)
        .await
        .with_context(|| format!("read {}", dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "jpg") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut frames = Vec::new();
    for path in paths.into_iter().take(3) {
        let bytes = fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        if !bytes.is_empty() {
            frames.push(bytes);
        }
    }
    Ok(frames)
}
