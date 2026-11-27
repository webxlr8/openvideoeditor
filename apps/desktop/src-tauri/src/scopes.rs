//! Live scopes for the colour panel (design D7): the frame the viewer presents is binned on a worker thread, at
//! most 15 times a second, into the scope the panel shows; the panel fetches the latest u8 density grid (256 × 256,
//! base64) by sequence number. Scopes never render the timeline again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use ove_render::Rgba;
use ove_render::scopes::{self, Scope};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::AppResult;
use crate::worker::relock;

/// The shortest time between two binned frames (15 Hz).
const INTERVAL: Duration = Duration::from_millis(66);

/// The latest scope for the panel.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScopeFrame {
    /// Increases with every new grid.
    pub seq: f64,
    pub scope: String,
    /// Timeline time of the frame, microseconds.
    pub pts_us: f64,
    pub width: u32,
    pub height: u32,
    /// `width × height` density bytes (0 empty … 255 fullest), row-major, base64.
    pub data: String,
}

struct State {
    wanted: Option<Scope>,
    last: Option<Instant>,
    latest: Option<ScopeFrame>,
    worker: Option<SyncSender<(Arc<Rgba>, i64, Scope)>>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { wanted: None, last: None, latest: None, worker: None }))
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Bins one frame into `scope` (also used by tests).
pub fn frame_of(image: &Rgba, pts_us: i64, scope: Scope) -> ScopeFrame {
    let density = scopes::density(&scopes::bin(image, scope));
    ScopeFrame {
        seq: SEQ.fetch_add(1, Ordering::SeqCst) as f64 + 1.0,
        scope: scopes::NAMES[scope as usize].into(),
        pts_us: pts_us as f64,
        width: scopes::SIZE as u32,
        height: scopes::SIZE as u32,
        data: base64::engine::general_purpose::STANDARD.encode(density),
    }
}

/// Offers a presented frame: binned on the worker when the panel shows a scope and 66 ms have passed; dropped
/// when the worker is still busy with the previous one.
pub fn offer(image: &Arc<Rgba>, pts_us: i64) {
    let mut s = relock(state());
    let Some(scope) = s.wanted else { return };
    if s.last.is_some_and(|t| t.elapsed() < INTERVAL) {
        return;
    }
    s.last = Some(Instant::now());
    if s.worker.is_none() {
        let (tx, rx) = sync_channel::<(Arc<Rgba>, i64, Scope)>(1);
        let spawned = crate::worker::spawn_contained("scopes", move || {
            while let Ok((img, pts, scope)) = rx.recv() {
                let f = frame_of(&img, pts, scope);
                relock(state()).latest = Some(f);
            }
        });
        if spawned.is_ok() {
            s.worker = Some(tx);
        }
    }
    if let Some(tx) = &s.worker {
        let _ = tx.try_send((image.clone(), pts_us, scope));
    }
}

/// Which scope the colour panel shows (`waveform`, `parade`, `vectorscope`), or none to stop binning.
#[tauri::command]
#[specta::specta]
pub fn set_scope(scope: Option<String>) -> AppResult<()> {
    let mut s = relock(state());
    s.wanted = scope.as_deref().and_then(Scope::parse);
    s.last = None;
    Ok(())
}

/// The latest scope grid, when newer than `since` (its `seq`).
#[tauri::command]
#[specta::specta]
pub fn scope_frame(since: Option<f64>) -> AppResult<Option<ScopeFrame>> {
    let s = relock(state());
    Ok(s.latest.clone().filter(|f| since.is_none_or(|n| f.seq > n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_binned_at_most_15_times_a_second_for_the_shown_scope() {
        set_scope(None).unwrap();
        let img = Arc::new(ove_render::parity::mosaic(192, 108));
        offer(&img, 0);
        assert!(relock(state()).worker.is_none(), "nothing is binned while no scope is shown");
        set_scope(Some("vectorscope".into())).unwrap();
        offer(&img, 1_000);
        let deadline = Instant::now() + Duration::from_secs(5);
        let first = loop {
            if let Some(f) = scope_frame(None).unwrap() {
                break f;
            }
            assert!(Instant::now() < deadline, "the worker bins the frame");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!((first.scope.as_str(), first.width, first.height), ("vectorscope", 256, 256));
        let bytes = base64::engine::general_purpose::STANDARD.decode(&first.data).unwrap();
        assert_eq!(bytes.len(), 256 * 256);
        assert!(bytes.contains(&255));
        // A second offer within 66 ms is skipped; nothing newer than the first.
        offer(&img, 2_000);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(scope_frame(Some(first.seq)).unwrap(), None);
        set_scope(None).unwrap();
    }
}
