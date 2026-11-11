//! Task 3.1: scheduler, A/V sync and every transport transition on a simulated clock.

use ove_playback::{
    AudioRing, ClockSource, Command, FrameSource, Micros, PlayState, Player, PlayerConfig, PresentableFrame, SimTime,
    SourcePoll, Speed, TimeSource,
};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    pts: Micros,
    dur: Micros,
}

impl PresentableFrame for Frame {
    fn pts_us(&self) -> Micros {
        self.pts
    }
    fn duration_us(&self) -> Micros {
        self.dur
    }
}

/// Frames at given timestamps; decoding one frame takes `decode_us` of simulated time; a seek costs `seek_us`.
struct FakeSource {
    frames: Vec<Frame>,
    idx: usize,
    time: SimTime,
    ready_at: Micros,
    decode_us: Micros,
    seek_us: Micros,
    seeks: Vec<Micros>,
    fail_at: Option<usize>,
}

impl FakeSource {
    fn cfr(n: usize, interval: Micros, time: &SimTime) -> Self {
        let frames = (0..n).map(|i| Frame { pts: i as Micros * interval, dur: interval }).collect();
        Self::with(frames, time)
    }
    fn with(frames: Vec<Frame>, time: &SimTime) -> Self {
        FakeSource {
            frames,
            idx: 0,
            time: time.clone(),
            ready_at: 0,
            decode_us: 0,
            seek_us: 0,
            seeks: vec![],
            fail_at: None,
        }
    }
}

impl FrameSource for FakeSource {
    type Frame = Frame;
    fn seek(&mut self, t: Micros) {
        self.seeks.push(t);
        // Frame whose interval contains t (first frame if before it).
        self.idx = self.frames.iter().rposition(|f| f.pts <= t).unwrap_or(0);
        self.ready_at = self.time.now_us() + self.seek_us;
    }
    fn poll(&mut self) -> SourcePoll<Frame> {
        if Some(self.idx) == self.fail_at {
            return SourcePoll::Error("media offline".into());
        }
        if self.idx >= self.frames.len() {
            return SourcePoll::Eof;
        }
        let now = self.time.now_us();
        if now < self.ready_at {
            return SourcePoll::Pending;
        }
        let f = self.frames[self.idx];
        self.idx += 1;
        self.ready_at = now.max(self.ready_at) + self.decode_us;
        SourcePoll::Frame(f)
    }
}

const FRAME: Micros = 33_333; // 30 fps
const TICK: Micros = 1_000;

fn config(src: &FakeSource) -> PlayerConfig {
    PlayerConfig { frame_interval_us: FRAME, last_frame_us: src.frames.last().unwrap().pts }
}

/// Run ticks for `dur`, returning presented (wall, pts).
fn run(p: &mut Player<FakeSource>, time: &SimTime, dur: Micros) -> Vec<(Micros, Micros)> {
    let mut shown = vec![];
    let end = time.now_us() + dur;
    while time.now_us() < end {
        time.advance(TICK);
        let out = p.tick(time.now_us());
        if let Some(f) = out.present {
            shown.push((time.now_us(), f.pts));
        }
    }
    shown
}

fn new_player(src: FakeSource, time: &SimTime) -> Player<FakeSource> {
    let cfg = config(&src);
    let mut p = Player::new(src, cfg, None, time.now_us());
    run(&mut p, time, 5_000); // first frame shown
    p
}

#[test]
fn shows_first_frame_while_paused() {
    let time = SimTime::new();
    let mut p = Player::new(FakeSource::cfr(30, FRAME, &time), config(&FakeSource::cfr(30, FRAME, &time)), None, 0);
    let shown = run(&mut p, &time, 5_000);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].1, 0);
    assert_eq!(p.transport().state, PlayState::Paused);
    assert_eq!(p.diagnostics().seek_latency.count, 0, "initial load is not a user seek");
}

#[test]
fn plays_every_frame_on_time_at_1x() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(90, FRAME, &time), &time);
    p.command(Command::Play, time.now_us());
    let shown = run(&mut p, &time, 4_000_000);
    let pts: Vec<Micros> = shown.iter().map(|s| s.1).collect();
    assert_eq!(pts, (1..90).map(|i| i * FRAME).collect::<Vec<_>>(), "every frame after the first, in order");
    let d = p.diagnostics();
    assert_eq!(d.dropped_frames, 0);
    assert!(d.av_offset.max_us <= TICK, "presented within one tick of its pts: {:?}", d.av_offset);
    // Without loop, playback stops on the last frame.
    assert_eq!(p.transport().state, PlayState::Paused);
    assert_eq!(p.transport().position, 89 * FRAME);
}

#[test]
fn late_frames_are_dropped_and_counted() {
    let time = SimTime::new();
    let mut src = FakeSource::cfr(120, FRAME, &time);
    src.decode_us = 50_000; // slower than real time
    let mut p = new_player(src, &time);
    p.command(Command::Play, time.now_us());
    run(&mut p, &time, 2_000_000);
    let d = p.diagnostics();
    assert!(d.dropped_frames > 10, "{d:?}");
    assert!(d.presented_frames + d.dropped_frames > 20);
}

#[test]
fn variable_frame_rate_presents_each_frame_at_its_own_time() {
    let time = SimTime::new();
    // Irregular intervals: 20, 47, 20, 47 ms ...
    let mut pts = vec![0];
    for i in 0..40 {
        pts.push(pts.last().unwrap() + if i % 2 == 0 { 20_000 } else { 47_000 });
    }
    let frames: Vec<Frame> = pts.windows(2).map(|w| Frame { pts: w[0], dur: w[1] - w[0] }).collect();
    let mut p = new_player(FakeSource::with(frames.clone(), &time), &time);
    let t0 = time.now_us();
    p.command(Command::Play, t0);
    let shown = run(&mut p, &time, 3_000_000);
    assert_eq!(shown.len(), frames.len() - 1);
    for (wall, pts) in shown {
        let media = wall - t0;
        assert!(media >= pts && media - pts <= TICK, "frame {pts} shown at media time {media}");
    }
    assert_eq!(p.diagnostics().dropped_frames, 0);
}

#[test]
fn seek_shows_containing_frame_and_measures_latency() {
    let time = SimTime::new();
    let mut src = FakeSource::cfr(300, FRAME, &time);
    src.seek_us = 40_000;
    let mut p = new_player(src, &time);
    p.command(Command::Seek(5 * FRAME + 10_000), time.now_us());
    let shown = run(&mut p, &time, 100_000);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].1, 5 * FRAME);
    let lat = p.diagnostics().seek_latency;
    assert_eq!(lat.count, 1);
    assert!((40_000..=41_000).contains(&lat.max_us), "{lat:?}");
    assert_eq!(p.transport().position, 5 * FRAME);
    // Seeking while playing keeps playing from the target.
    p.command(Command::Play, time.now_us());
    p.command(Command::Seek(100 * FRAME), time.now_us());
    let shown = run(&mut p, &time, 200_000);
    assert_eq!(shown[0].1, 100 * FRAME);
    assert!(shown.len() > 3, "continues playing after the seek");
    assert!(p.transport().is_playing());
}

#[test]
fn ten_steps_forward_advance_exactly_ten_frames() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(60, FRAME, &time), &time);
    p.command(Command::Seek(17 * FRAME), time.now_us());
    run(&mut p, &time, 5_000);
    for _ in 0..10 {
        p.command(Command::StepForward, time.now_us());
        run(&mut p, &time, 2_000);
    }
    assert_eq!(p.transport().position, 27 * FRAME);
    for _ in 0..3 {
        p.command(Command::StepBackward, time.now_us());
        run(&mut p, &time, 2_000);
    }
    assert_eq!(p.transport().position, 24 * FRAME);
    // Stepping is paused-only: a step while playing pauses first.
    p.command(Command::Play, time.now_us());
    run(&mut p, &time, 100_000);
    p.command(Command::StepForward, time.now_us());
    let pos_before = p.transport().position;
    run(&mut p, &time, 5_000);
    assert_eq!(p.transport().state, PlayState::Paused);
    assert_eq!(p.transport().position, pos_before + FRAME);
    let pos = p.transport().position;
    run(&mut p, &time, 200_000);
    assert_eq!(p.transport().position, pos, "stays paused");
}

#[test]
fn step_back_at_first_frame_and_forward_at_last_frame_are_no_ops() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(5, FRAME, &time), &time);
    p.command(Command::StepBackward, time.now_us());
    run(&mut p, &time, 5_000);
    assert_eq!(p.transport().position, 0);
    p.command(Command::GoToEnd, time.now_us());
    run(&mut p, &time, 5_000);
    assert_eq!(p.transport().position, 4 * FRAME);
    p.command(Command::StepForward, time.now_us());
    run(&mut p, &time, 5_000);
    assert_eq!(p.transport().position, 4 * FRAME);
}

#[test]
fn home_end_and_play_from_end_restarts() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(30, FRAME, &time), &time);
    p.command(Command::GoToEnd, time.now_us());
    run(&mut p, &time, 5_000);
    assert_eq!(p.transport().position, 29 * FRAME);
    p.command(Command::Play, time.now_us());
    let shown = run(&mut p, &time, 100_000);
    assert_eq!(shown[0].1, 0, "play at the end starts over");
    p.command(Command::GoToStart, time.now_us());
    run(&mut p, &time, 5_000);
    assert!(p.transport().position < 3 * FRAME);
}

#[test]
fn loop_wraps_to_start() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(30, FRAME, &time), &time);
    p.command(Command::SetLoop(true), time.now_us());
    p.command(Command::Play, time.now_us());
    let shown = run(&mut p, &time, 2_500_000); // 2.5 loops of 1 s
    let wraps = shown.windows(2).filter(|w| w[1].1 < w[0].1).count();
    assert_eq!(wraps, 2, "wrapped twice");
    assert!(p.transport().is_playing());
    assert_eq!(p.diagnostics().dropped_frames, 0);
}

#[test]
fn double_speed_skips_without_counting_drops_and_is_video_only() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(300, FRAME, &time), &time);
    p.command(Command::ShuttleForward, time.now_us());
    p.command(Command::ShuttleForward, time.now_us());
    assert_eq!(p.transport().speed, Speed::Fwd2);
    assert!(!p.transport().audio_audible());
    let t0 = p.transport().position;
    // 30 Hz display refresh: at 2× only every other 30 fps frame can be shown.
    let mut shown = vec![];
    for _ in 0..30 {
        time.advance(33_334);
        if let Some(f) = p.tick(time.now_us()).present {
            shown.push(f.pts);
        }
    }
    let advanced = p.transport().position - t0;
    assert!((1_900_000..=2_100_000).contains(&advanced), "advanced {advanced} µs in 1 s at 2×");
    let d = p.diagnostics();
    assert_eq!(d.dropped_frames, 0);
    assert!(d.skipped_frames >= 25, "{d:?}");
    assert!(shown.len() >= 25);
}

#[test]
fn reverse_shuttle_plays_backwards_and_stops_at_start() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(90, FRAME, &time), &time);
    p.command(Command::Seek(60 * FRAME), time.now_us());
    run(&mut p, &time, 5_000);
    p.command(Command::ShuttleReverse, time.now_us());
    assert_eq!(p.transport().speed, Speed::Rev1);
    let shown = run(&mut p, &time, 500_000);
    assert!(shown.len() >= 10);
    assert!(shown.windows(2).all(|w| w[1].1 < w[0].1), "positions decrease: {shown:?}");
    p.command(Command::ShuttleReverse, time.now_us());
    assert_eq!(p.transport().speed, Speed::Rev2);
    run(&mut p, &time, 2_000_000);
    assert_eq!(p.transport().state, PlayState::Paused, "stops at the start");
    assert_eq!(p.transport().position, 0);
    p.command(Command::ShuttleForward, time.now_us());
    assert_eq!((p.transport().state, p.transport().speed), (PlayState::Playing, Speed::Fwd1));
}

#[test]
fn shuttle_stop_pauses_and_resume_continues_from_the_shown_frame() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(300, FRAME, &time), &time);
    p.command(Command::ShuttleForward, time.now_us());
    run(&mut p, &time, 500_000);
    p.command(Command::ShuttleStop, time.now_us());
    let pos = p.transport().position;
    run(&mut p, &time, 300_000);
    assert_eq!(p.transport().position, pos);
    p.command(Command::TogglePlay, time.now_us());
    let shown = run(&mut p, &time, 100_000);
    assert_eq!(shown[0].1, pos + FRAME, "continues with the next frame");
}

#[test]
fn source_error_stops_playback_with_message() {
    let time = SimTime::new();
    let mut src = FakeSource::cfr(100, FRAME, &time);
    src.fail_at = Some(40);
    let mut p = new_player(src, &time);
    p.command(Command::Play, time.now_us());
    let mut err = None;
    for _ in 0..3_000 {
        time.advance(TICK);
        if let Some(e) = p.tick(time.now_us()).error {
            err = Some(e);
            break;
        }
    }
    assert_eq!(err.as_deref(), Some("media offline"));
    assert_eq!(p.transport().state, PlayState::Paused);
}

/// Simulated audio device: a callback every 10 ms writing 480 samples, 25 ms output latency.
fn audio_callback(ring: &AudioRing, time: &SimTime, buf: &mut [f32]) {
    ring.fill(buf, time.now_us(), 25_000);
}

#[test]
fn audio_is_master_clock_at_1x_and_video_follows_it() {
    let time = SimTime::new();
    let ring = AudioRing::new(48_000, 1);
    let src = FakeSource::cfr(120, FRAME, &time);
    let cfg = config(&src);
    let mut p = Player::new(src, cfg, Some(ring.clone()), 0);
    run(&mut p, &time, 5_000);
    p.command(Command::Play, time.now_us());
    let mut buf = vec![0f32; 480];
    let mut fed_to = None;
    let mut offsets = vec![];
    for step in 0..2_000 {
        time.advance(TICK);
        let out = p.tick(time.now_us());
        if let Some(t) = out.audio_restart {
            fed_to = Some(t);
        }
        // Producer keeps 200 ms queued.
        if let Some(t) = fed_to.as_mut()
            && ring.queued_us() < 200_000
        {
            ring.push(*t, &[0.25; 4_800]);
            *t += 100_000;
        }
        if step % 10 == 0 {
            audio_callback(&ring, &time, &mut buf);
        }
        if let (Some(f), Some(pos)) = (out.present, ring.position()) {
            offsets.push((pos.heard_at(time.now_us()) - f.pts).abs());
        }
    }
    assert_eq!(p.clock_source(), ClockSource::Audio);
    let max = offsets.iter().max().copied().unwrap();
    assert!(max <= 2 * FRAME, "A/V offset {max} µs");
    let d = p.diagnostics();
    assert!(d.av_offset.mean_us <= 2 * FRAME, "{d:?}");
    assert!(d.presented_frames > 45);
}

#[test]
fn audio_stall_holds_video() {
    let time = SimTime::new();
    let ring = AudioRing::new(1_000, 1);
    let src = FakeSource::cfr(120, FRAME, &time);
    let cfg = config(&src);
    let mut p = Player::new(src, cfg, Some(ring.clone()), 0);
    run(&mut p, &time, 5_000);
    p.command(Command::Play, time.now_us());
    let restart = p.tick(time.now_us()).audio_restart.unwrap();
    ring.push(restart, &[0.0; 300]); // only 300 ms of audio
    let mut buf = vec![0f32; 10];
    for _ in 0..1_000 {
        time.advance(TICK);
        p.tick(time.now_us());
        if time.now_us() % 10_000 == 0 {
            ring.fill(&mut buf, time.now_us(), 0);
        }
    }
    // After 1 s of wall time only ~300 ms of media has played: video waits for audio.
    let pos = p.transport().position;
    assert!((250_000..=360_000).contains(&pos), "video position {pos}");
}

#[test]
fn no_audio_device_uses_system_clock() {
    let time = SimTime::new();
    let mut p = new_player(FakeSource::cfr(60, FRAME, &time), &time);
    p.command(Command::Play, time.now_us());
    run(&mut p, &time, 500_000);
    assert_eq!(p.clock_source(), ClockSource::System);
    assert!(p.transport().position >= 14 * FRAME);
    // Attaching a device later switches to the audio clock on the next restart.
    let ring = AudioRing::new(48_000, 2);
    p.set_audio(Some(ring.clone()), time.now_us());
    let out = p.tick(time.now_us());
    assert!(out.audio_restart.is_some());
}

#[test]
fn pause_and_non_1x_mute_the_audio_ring() {
    let time = SimTime::new();
    let ring = AudioRing::new(1_000, 1);
    let src = FakeSource::cfr(300, FRAME, &time);
    let cfg = config(&src);
    let mut p = Player::new(src, cfg, Some(ring.clone()), 0);
    run(&mut p, &time, 5_000);
    let mut buf = [1.0f32; 4];
    p.command(Command::Play, time.now_us());
    let t = p.tick(time.now_us()).audio_restart.unwrap();
    ring.push(t, &[0.5; 100]);
    ring.fill(&mut buf, time.now_us(), 0);
    assert_eq!(buf, [0.5; 4], "audible at 1×");
    p.command(Command::ShuttleForward, time.now_us()); // 2×
    ring.fill(&mut buf, time.now_us(), 0);
    assert_eq!(buf, [0.0; 4], "muted at 2×");
    p.command(Command::Pause, time.now_us());
    ring.fill(&mut buf, time.now_us(), 0);
    assert_eq!(buf, [0.0; 4], "silent when paused");
    p.set_volume(0.5);
    p.set_muted(true);
}

#[test]
fn diagnostics_snapshot_serializes() {
    let time = SimTime::new();
    let p = new_player(FakeSource::cfr(3, FRAME, &time), &time);
    let json = serde_json::to_value(p.diagnostics()).unwrap();
    for key in ["presentedFrames", "droppedFrames", "skippedFrames", "avOffset", "seekLatency", "clock"] {
        assert!(json.get(key).is_some(), "{key} in {json}");
    }
    let _ = time.now_us();
}
