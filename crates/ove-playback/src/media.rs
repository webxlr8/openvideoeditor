//! Adapter from `ove_media::DecodeWorker` to [`FrameSource`] (feature `media`).

use ove_media::{DecodePath, DecodeWorker, VideoFrame, WorkerEvent};

use crate::clock::Micros;
use crate::player::{FrameSource, PresentableFrame, SourcePoll};

impl PresentableFrame for VideoFrame {
    fn pts_us(&self) -> Micros {
        self.pts_us
    }
    fn duration_us(&self) -> Micros {
        self.duration_us
    }
}

/// A decode worker as a non-blocking frame source.
pub struct WorkerSource {
    worker: DecodeWorker,
}

impl WorkerSource {
    pub fn new(worker: DecodeWorker) -> Self {
        WorkerSource { worker }
    }
    pub fn decode_path(&self) -> DecodePath {
        self.worker.decode_path()
    }
}

impl FrameSource for WorkerSource {
    type Frame = VideoFrame;

    fn seek(&mut self, t_us: Micros) {
        self.worker.seek(t_us);
    }

    fn poll(&mut self) -> SourcePoll<VideoFrame> {
        match self.worker.try_recv() {
            Some(WorkerEvent::Frame(f)) => SourcePoll::Frame(f),
            Some(WorkerEvent::Eof) => SourcePoll::Eof,
            Some(WorkerEvent::Error(e)) => SourcePoll::Error(e.to_string()),
            None => SourcePoll::Pending,
        }
    }
}
