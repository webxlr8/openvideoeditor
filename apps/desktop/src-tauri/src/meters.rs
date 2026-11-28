//! Level meters (design D5): the timeline mix passes through a meter tap on the producer side, and the engine sends
//! batches of readings to the UI at most 30 times per second. Each batch carries the heard media position at the
//! moment it was sent and the readings around it, so the UI shows the reading under what the person hears (device
//! latency included) rather than what was just mixed.

use std::time::{Duration, Instant};

use ove_playback::AudioPosition;
use ove_render::meter::{MeterQueue, MeterReading, MeterTap};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;

/// Shortest time between two meter events (30 per second).
pub const EMIT_EVERY: Duration = Duration::from_millis(33);
/// A batch holds readings from this long before the heard position…
const BEHIND_US: i64 = 40_000;
/// …to this long after it, so the UI can follow the heard position until the next batch.
const AHEAD_US: i64 = 90_000;

/// One reading as the UI uses it. Times are media seconds; levels are dB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeterReadingView {
    pub start: f64,
    pub end: f64,
    /// Sample peak, RMS and true peak per channel (left, right).
    pub peak: Vec<f64>,
    pub rms: Vec<f64>,
    pub true_peak: Vec<f64>,
    pub momentary: Option<f64>,
    pub short_term: Option<f64>,
}

impl From<&MeterReading> for MeterReadingView {
    fn from(r: &MeterReading) -> Self {
        MeterReadingView {
            start: r.start_us as f64 / 1e6,
            end: r.end_us as f64 / 1e6,
            peak: r.peak_db.to_vec(),
            rms: r.rms_db.to_vec(),
            true_peak: r.true_peak_db.to_vec(),
            momentary: r.momentary_lufs,
            short_term: r.short_term_lufs,
        }
    }
}

/// A batch of meter readings.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct AudioMeter {
    /// Media seconds being heard when the batch was sent.
    pub heard: f64,
    /// Whether the mix is playing; when false the meters fall back.
    pub playing: bool,
    pub readings: Vec<MeterReadingView>,
}

/// The engine's meter state.
#[derive(Default)]
pub struct Meters {
    tap: MeterTap,
    queue: MeterQueue,
    last_emit: Option<Instant>,
    was_playing: bool,
}

impl Meters {
    /// Meters one mixed block (48 kHz stereo) starting at media time `t0_us`.
    pub fn tap(&mut self, t0_us: i64, block: &[f32]) {
        let r = self.tap.process(t0_us, block);
        self.queue.push(r);
    }

    /// The play head jumped: readings ahead of it are stale.
    pub fn restart(&mut self) {
        self.tap.reset();
        self.queue.clear();
    }

    /// The batch to send now, if one is due: at most every [`EMIT_EVERY`] while playing, and once when playback stops.
    pub fn due(
        &mut self,
        now: Instant,
        wall_us: i64,
        position: Option<AudioPosition>,
        playing: bool,
    ) -> Option<AudioMeter> {
        let stopped = self.was_playing && !playing;
        self.was_playing = playing;
        if stopped {
            self.last_emit = Some(now);
            return Some(AudioMeter { heard: 0.0, playing: false, readings: vec![] });
        }
        if !playing || self.last_emit.is_some_and(|t| now.duration_since(t) < EMIT_EVERY) {
            return None;
        }
        let heard = position?.heard_at(wall_us);
        self.last_emit = Some(now);
        let readings = self.queue.batch(heard, BEHIND_US, AHEAD_US).iter().map(MeterReadingView::from).collect();
        Some(AudioMeter { heard: heard as f64 / 1e6, playing: true, readings })
    }

    /// Sends a due batch.
    pub fn emit(&mut self, app: &AppHandle, wall_us: i64, position: Option<AudioPosition>, playing: bool) {
        if let Some(m) = self.due(Instant::now(), wall_us, position, playing) {
            let _ = m.emit(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_follow_the_heard_position_at_most_30_times_a_second() {
        let mut m = Meters::default();
        // 1 s of a -6 dBFS tone, mixed ahead of the play head.
        let amp = 10f32.powf(-6.0 / 20.0);
        for b in 0..47i64 {
            let block: Vec<f32> = (0..1024)
                .flat_map(|i| {
                    let v = amp * (std::f32::consts::TAU * 750.0 * (b * 1024 + i) as f32 / 48_000.0).sin();
                    [v, v]
                })
                .collect();
            m.tap(b * 1024 * 1_000_000 / 48_000, &block);
        }
        let t0 = Instant::now();
        // Heard position: media 0.5 s at wall 0, with 30 ms of device latency.
        let pos = AudioPosition { media_us: 530_000, at_wall_us: 0, latency_us: 30_000 };
        let first = m.due(t0, 0, Some(pos), true).unwrap();
        assert!((first.heard - 0.5).abs() < 1e-9);
        assert!(first.readings.iter().all(|r| r.end >= 0.46 && r.start <= 0.59));
        assert!(first.readings.iter().any(|r| r.start <= 0.5 && 0.5 < r.end));
        for r in &first.readings {
            assert!((r.peak[0] + 6.0).abs() < 0.1 && (r.peak[1] + 6.0).abs() < 0.1, "{:?}", r.peak);
        }
        // Not again within 33 ms; again after.
        assert!(m.due(t0 + Duration::from_millis(10), 10_000, Some(pos), true).is_none());
        assert!(m.due(t0 + Duration::from_millis(34), 34_000, Some(pos), true).is_some());
        // Stopping sends one empty, not-playing batch.
        let stop = m.due(t0 + Duration::from_millis(40), 40_000, Some(pos), false).unwrap();
        assert!(!stop.playing && stop.readings.is_empty());
        assert!(m.due(t0 + Duration::from_millis(90), 90_000, Some(pos), false).is_none());
    }
}
