use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use walkdir::WalkDir;

use crate::paths::codex_home;

const WORKING_SECS: u64 = 120;
const SLEEPING_SECS: u64 = 30 * 60;
const WARNING_PCT: f64 = 80.0;
const SUCCESS_HOLD_SECS: i64 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MascotState {
    Idle,
    Working,
    Success,
    Warning,
    Sleeping,
}

impl MascotState {
    /// Frame range inside the bundled GIF to play for this state.
    fn frame_range(self, total: usize) -> (usize, usize) {
        let total = total.max(1);
        match self {
            // ponytail: one GIF — play whole loop; states keep detection for later per-range use
            Self::Idle | Self::Working | Self::Success | Self::Warning | Self::Sleeping => {
                (0, total)
            }
        }
    }
}

struct MascotRuntime {
    state: MascotState,
    success_until: Option<i64>,
    used_percent: f64,
    frame_count: usize,
}

static RUNTIME: OnceLock<Mutex<MascotRuntime>> = OnceLock::new();
static ICON_TICK: AtomicUsize = AtomicUsize::new(0);

fn runtime() -> &'static Mutex<MascotRuntime> {
    RUNTIME.get_or_init(|| {
        let frames = ensure_extracted();
        Mutex::new(MascotRuntime {
            state: MascotState::Idle,
            success_until: None,
            used_percent: 0.0,
            frame_count: frames,
        })
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn frames_dir() -> std::path::PathBuf {
    crate::paths::icon_dir().join("mascot-gif")
}

fn ensure_extracted() -> usize {
    let dir = frames_dir();
    let _ = std::fs::create_dir_all(&dir);
    let count = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    if count > 0 {
        return count;
    }
    let gif = include_bytes!("../assets/mascot/mascot.gif");
    let tmp = dir.join("src.gif");
    let _ = std::fs::write(&tmp, gif);
    let out = dir.join("f%04d.png");
    // ponytail: ImageMagick chokes on this GIF's LZW; ffmpeg extracts fine
    let _ = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&tmp)
        .args(["-vsync", "0"])
        .arg(&out)
        .output();
    let count = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    if count == 0 {
        let _ = Command::new("convert")
            .arg(&tmp)
            .arg("-coalesce")
            .arg(&out)
            .output();
    }
    std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
        .max(1)
}

pub fn seconds_since_last_session_write() -> Option<u64> {
    let root = codex_home().join("sessions");
    if !root.exists() {
        return None;
    }
    let mut newest: Option<SystemTime> = None;
    for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if entry.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        newest = Some(match newest {
            Some(n) => n.max(mtime),
            None => mtime,
        });
    }
    newest.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs())
}

fn resolve_state(used_percent: f64, success_until: Option<i64>) -> MascotState {
    let now = now_secs();
    if success_until.is_some_and(|until| now < until) {
        return MascotState::Success;
    }
    let age = seconds_since_last_session_write();
    if age.is_some_and(|s| s <= WORKING_SECS) {
        return MascotState::Working;
    }
    if used_percent >= WARNING_PCT {
        return MascotState::Warning;
    }
    if age.is_none_or(|s| s >= SLEEPING_SECS) {
        return MascotState::Sleeping;
    }
    MascotState::Idle
}

pub fn notify_success() {
    let mut rt = runtime().lock().unwrap();
    rt.success_until = Some(now_secs() + SUCCESS_HOLD_SECS);
    if rt.state != MascotState::Success {
        rt.state = MascotState::Success;
    }
}

pub fn set_used_percent(pct: f64) {
    let mut rt = runtime().lock().unwrap();
    rt.used_percent = pct;
    if rt.success_until.is_some_and(|until| now_secs() >= until) {
        rt.success_until = None;
    }
    let next = resolve_state(rt.used_percent, rt.success_until);
    if rt.state != next {
        rt.state = next;
    }
}

/// Advance to next animation frame. Returns true if a new frame should be drawn.
pub fn advance_frame() -> bool {
    let mut rt = runtime().lock().unwrap();
    if rt.success_until.is_some_and(|until| now_secs() >= until) {
        rt.success_until = None;
        let next = resolve_state(rt.used_percent, None);
        if rt.state != next {
            rt.state = next;
        }
    }
    rt.frame_count > 1
}

pub fn current_frame_path() -> std::path::PathBuf {
    let rt = runtime().lock().unwrap();
    let (start, end) = rt.state.frame_range(rt.frame_count);
    let tick = ICON_TICK.fetch_add(1, Ordering::Relaxed) as usize;
    let span = end.saturating_sub(start).max(1);
    let idx = start + (tick % span);
    // ponytail: ffmpeg writes 1-based (f0001.png), so +1
    frames_dir().join(format!("f{:04}.png", idx + 1))
}

pub fn icon_suffix() -> String {
    let rt = runtime().lock().unwrap();
    let tick = ICON_TICK.load(Ordering::Relaxed);
    format!("{:?}-{}", rt.state, tick % 64)
}

pub fn tick(used_percent: f64) {
    set_used_percent(used_percent);
}
