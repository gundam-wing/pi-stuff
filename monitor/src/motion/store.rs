use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::Mutex};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MotionEvent {
    pub id: String,
    pub captured_at: u64,
    pub frames: usize,
    pub score: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Manifest {
    events: Vec<MotionEvent>,
}

pub(crate) struct EventStore {
    dir: PathBuf,
    max_events: usize,
    max_bytes: u64,
    inner: Mutex<Manifest>,
}

impl EventStore {
    pub(crate) async fn open(dir: PathBuf, max_events: usize, max_bytes: u64) -> Result<Self> {
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create motion directory {}", dir.display()))?;
        cleanup_tmp_dirs(&dir).await?;
        let mut manifest = load_manifest(&dir).await?;
        manifest.events = reconcile(&dir, manifest.events).await?;
        let store = Self {
            dir,
            max_events,
            max_bytes,
            inner: Mutex::new(manifest),
        };
        {
            let mut manifest = store.inner.lock().await;
            store.evict(&mut manifest).await?;
            store.persist(&manifest).await?;
        }
        Ok(store)
    }

    pub(crate) async fn list(&self) -> Vec<MotionEvent> {
        self.inner.lock().await.events.clone()
    }

    pub(crate) fn frame_path(&self, id: &str, frame: &str) -> Option<PathBuf> {
        if !valid_event_id(id) || !valid_frame_name(frame) {
            return None;
        }
        Some(self.dir.join(id).join(frame))
    }

    pub(crate) async fn insert(
        &self,
        jpegs: Vec<Vec<u8>>,
        score: f32,
        captured_at: u64,
    ) -> Result<MotionEvent> {
        if jpegs.is_empty() {
            bail!("motion event has no frames");
        }

        let mut manifest = self.inner.lock().await;
        let mut captured_at = captured_at;
        let mut id = event_id_from_millis(captured_at);
        while manifest.events.iter().any(|event| event.id == id) || self.dir.join(&id).exists() {
            captured_at += 1;
            id = event_id_from_millis(captured_at);
        }

        let tmp = self.dir.join(format!(".{id}.tmp"));
        let dest = self.dir.join(&id);
        if tmp.exists() {
            fs::remove_dir_all(&tmp).await.ok();
        }
        fs::create_dir(&tmp)
            .await
            .with_context(|| format!("create {}", tmp.display()))?;
        for (index, jpeg) in jpegs.iter().enumerate() {
            fs::write(tmp.join(format!("{index:02}.jpg")), jpeg)
                .await
                .with_context(|| format!("write motion frame {index}"))?;
        }
        fs::rename(&tmp, &dest)
            .await
            .with_context(|| format!("publish {}", dest.display()))?;

        let event = MotionEvent {
            id,
            captured_at,
            frames: jpegs.len(),
            score,
        };
        manifest.events.insert(0, event.clone());
        self.evict(&mut manifest).await?;
        self.persist(&manifest).await?;
        Ok(event)
    }

    async fn evict(&self, manifest: &mut Manifest) -> Result<()> {
        while manifest.events.len() > self.max_events
            || (manifest.events.len() > 1
                && self.total_bytes(&manifest.events).await? > self.max_bytes)
        {
            let Some(oldest) = manifest.events.pop() else {
                break;
            };
            let path = self.dir.join(&oldest.id);
            if let Err(error) = fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), %error, "could not remove old motion event");
            }
        }
        Ok(())
    }

    async fn total_bytes(&self, events: &[MotionEvent]) -> Result<u64> {
        let mut total = 0;
        for event in events {
            total += directory_size(&self.dir.join(&event.id)).await?;
        }
        Ok(total)
    }

    async fn persist(&self, manifest: &Manifest) -> Result<()> {
        let tmp = self.dir.join(".manifest.json.tmp");
        let dest = self.dir.join("manifest.json");
        let bytes = serde_json::to_vec_pretty(manifest).context("serialize motion manifest")?;
        fs::write(&tmp, bytes)
            .await
            .with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &dest)
            .await
            .with_context(|| format!("publish {}", dest.display()))?;
        Ok(())
    }
}

pub(crate) fn valid_event_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("event-") else {
        return false;
    };
    let bytes = rest.as_bytes();
    bytes.len() == 19
        && bytes[8] == b'-'
        && bytes[15] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 8 || index == 15 || byte.is_ascii_digit())
}

pub(crate) fn valid_frame_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jpg") else {
        return false;
    };
    stem.len() == 2 && stem.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn event_id_from_millis(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("event-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-{millis:03}")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month, day)
}

async fn load_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("manifest.json");
    match fs::read(&path).await {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

async fn reconcile(dir: &Path, events: Vec<MotionEvent>) -> Result<Vec<MotionEvent>> {
    let mut known = Vec::new();
    for mut event in events {
        if !valid_event_id(&event.id) {
            continue;
        }
        let event_dir = dir.join(&event.id);
        let frames = count_jpegs(&event_dir).await?;
        if frames == 0 {
            continue;
        }
        event.frames = frames;
        known.push(event);
    }

    let mut entries = fs::read_dir(dir)
        .await
        .with_context(|| format!("read {}", dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !valid_event_id(name) || known.iter().any(|event| event.id == name) {
            continue;
        }
        let frames = count_jpegs(&entry.path()).await?;
        if frames == 0 {
            continue;
        }
        known.push(MotionEvent {
            id: name.to_string(),
            captured_at: millis_from_event_id(name).unwrap_or(0),
            frames,
            score: 0.0,
        });
    }
    known.sort_by(|left, right| right.captured_at.cmp(&left.captured_at));
    Ok(known)
}

async fn count_jpegs(dir: &Path) -> Result<usize> {
    let mut count = 0;
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error).with_context(|| format!("read {}", dir.display())),
    };
    while let Some(entry) = entries.next_entry().await? {
        if entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(valid_frame_name)
        {
            count += 1;
        }
    }
    Ok(count)
}

async fn directory_size(dir: &Path) -> Result<u64> {
    let mut total = 0;
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error).with_context(|| format!("read {}", dir.display())),
    };
    while let Some(entry) = entries.next_entry().await? {
        total += entry
            .metadata()
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }
    Ok(total)
}

async fn cleanup_tmp_dirs(dir: &Path) -> Result<()> {
    let mut entries = fs::read_dir(dir)
        .await
        .with_context(|| format!("read {}", dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.')
            && (name.ends_with(".tmp")
                || name.starts_with(".burst-")
                || name.starts_with(".event-"))
        {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await.ok();
            } else {
                fs::remove_file(&path).await.ok();
            }
        }
    }
    Ok(())
}

fn millis_from_event_id(id: &str) -> Option<u64> {
    let rest = id.strip_prefix("event-")?;
    let (date, rest) = rest.split_once('-')?;
    let (time, millis) = rest.split_once('-')?;
    if date.len() != 8 || time.len() != 6 || millis.len() != 3 {
        return None;
    }
    let year: i32 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(4..6)?.parse().ok()?;
    let day: u32 = date.get(6..8)?.parse().ok()?;
    let hour: u32 = time.get(0..2)?.parse().ok()?;
    let minute: u32 = time.get(2..4)?.parse().ok()?;
    let second: u32 = time.get(4..6)?.parse().ok()?;
    let millis: u64 = millis.parse().ok()?;
    let days = days_from_civil(year, month, day)?;
    let tod = u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(second);
    days.checked_mul(86_400)?
        .checked_add(tod)?
        .checked_mul(1000)?
        .checked_add(millis)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let mut year = i64::from(year);
    let mut month = i64::from(month);
    if month <= 2 {
        year -= 1;
        month += 9;
    } else {
        month -= 3;
    }
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = (year - era * 400) as u32;
    let doy = (153 * month as u32 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + i64::from(doe) - 719_468;
    u64::try_from(days).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn formats_unix_epoch_event_ids() {
        assert_eq!(event_id_from_millis(0), "event-19700101-000000-000");
        assert_eq!(event_id_from_millis(1_000), "event-19700101-000001-000");
        assert_eq!(event_id_from_millis(1_001), "event-19700101-000001-001");
        assert_eq!(
            event_id_from_millis(86_400_000),
            "event-19700102-000000-000"
        );
        assert_eq!(
            event_id_from_millis(1_582_934_400_000),
            "event-20200229-000000-000"
        );
        assert!(valid_event_id("event-20200229-000000-000"));
        assert!(!valid_event_id("../secret"));
        assert!(!valid_event_id("event-20200229-000000"));
        assert!(valid_frame_name("00.jpg"));
        assert!(valid_frame_name("02.jpg"));
        assert!(!valid_frame_name("0.jpg"));
        assert!(!valid_frame_name("00.png"));
    }

    #[test]
    fn event_id_round_trips_through_millis() {
        let id = event_id_from_millis(1_582_934_400_123);
        assert_eq!(millis_from_event_id(&id), Some(1_582_934_400_123));
    }

    #[tokio::test]
    async fn evicts_oldest_event_when_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path().to_path_buf(), 2, 1_000_000)
            .await
            .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        store.insert(vec![vec![1, 2, 3]], 0.1, now).await.unwrap();
        store
            .insert(vec![vec![4, 5, 6]], 0.2, now + 1)
            .await
            .unwrap();
        store
            .insert(vec![vec![7, 8, 9]], 0.3, now + 2)
            .await
            .unwrap();
        let events = store.list().await;
        assert_eq!(events.len(), 2);
        assert!((events[0].score - 0.3).abs() < f32::EPSILON);
        assert!((events[1].score - 0.2).abs() < f32::EPSILON);
        assert!(dir.path().join(&events[0].id).join("00.jpg").exists());
        assert!(dir.path().join(&events[1].id).join("00.jpg").exists());
    }

    #[tokio::test]
    async fn evicts_when_over_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path().to_path_buf(), 48, 8)
            .await
            .unwrap();
        let now = 1_700_000_000_000;
        store.insert(vec![vec![0; 6]], 0.1, now).await.unwrap();
        store.insert(vec![vec![1; 6]], 0.2, now + 1).await.unwrap();
        let events = store.list().await;
        assert_eq!(events.len(), 1);
        assert!((events[0].score - 0.2).abs() < f32::EPSILON);
    }
}
