use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};

use crate::motion::Roi;

const DEFAULT_CAPTURE_COMMAND: &str = "rpicam-vid --camera 0 --timeout 0 --width 1280 \
    --height 720 --framerate 15 --codec h264 --inline --intra 15 --bitrate 2500000 \
    --nopreview --output -";

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub bind: SocketAddr,
    pub hls_dir: PathBuf,
    pub web_dir: PathBuf,
    pub capture_command: Vec<String>,
    pub ffmpeg_bin: String,
    pub frame_rate: u32,
    pub revision: String,
    pub motion: MotionConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct MotionConfig {
    pub dir: PathBuf,
    pub max_events: usize,
    pub max_bytes: u64,
    pub threshold: f32,
    pub pixel_floor: u8,
    pub cooldown: Duration,
    pub roi: Option<Roi>,
    pub settle: Duration,
    pub analysis_width: u32,
    pub analysis_height: u32,
    pub analysis_fps: u32,
    pub burst_width: u32,
    pub burst_height: u32,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self> {
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
        let frame_rate = parse_env("MONITOR_FRAME_RATE", "15")?;
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
            motion: MotionConfig::from_env()?,
        })
    }
}

impl MotionConfig {
    fn from_env() -> Result<Self> {
        let dir = env::var_os("MONITOR_MOTION_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/pi-camera-monitor/motion"));
        let max_events = parse_env("MONITOR_MOTION_MAX_EVENTS", "48")?;
        if max_events == 0 {
            bail!("MONITOR_MOTION_MAX_EVENTS must be greater than zero");
        }
        let max_bytes = parse_env("MONITOR_MOTION_MAX_BYTES", "16777216")?;
        if max_bytes == 0 {
            bail!("MONITOR_MOTION_MAX_BYTES must be greater than zero");
        }
        let threshold = parse_env("MONITOR_MOTION_THRESHOLD", "0.02")?;
        if !(0.0..=1.0).contains(&threshold) {
            bail!("MONITOR_MOTION_THRESHOLD must be between 0 and 1");
        }
        let pixel_floor = parse_env("MONITOR_MOTION_PIXEL_FLOOR", "25")?;
        let cooldown_ms: u64 = parse_env("MONITOR_MOTION_COOLDOWN_MS", "3000")?;
        let settle_secs: u64 = parse_env("MONITOR_MOTION_SETTLE_SECS", "5")?;
        let roi = match env::var("MONITOR_MOTION_ROI") {
            Ok(value) if !value.trim().is_empty() => Some(Roi::parse(&value)?),
            _ => None,
        };

        Ok(Self {
            dir,
            max_events,
            max_bytes,
            threshold,
            pixel_floor,
            cooldown: Duration::from_millis(cooldown_ms),
            roi,
            settle: Duration::from_secs(settle_secs),
            analysis_width: 320,
            analysis_height: 180,
            analysis_fps: 2,
            burst_width: 640,
            burst_height: 360,
        })
    }

    pub(crate) fn frame_bytes(&self) -> usize {
        self.analysis_width as usize * self.analysis_height as usize
    }
}

pub(crate) fn parse_command(value: &str) -> Result<Vec<String>> {
    let command = shell_words::split(value).context("parse MONITOR_CAPTURE_COMMAND")?;
    if command.is_empty() {
        bail!("MONITOR_CAPTURE_COMMAND cannot be empty");
    }
    Ok(command)
}

fn parse_env<T>(key: &str, default: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env::var(key)
        .unwrap_or_else(|_| default.into())
        .parse()
        .with_context(|| format!("{key} is invalid"))
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
}
