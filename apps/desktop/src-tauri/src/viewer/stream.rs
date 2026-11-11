//! Candidate 3: frames rendered offscreen and fetched by the UI through the `ove-frame` URI
//! scheme, then painted on a canvas.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Largest streamed frame; larger viewer rectangles are scaled by the canvas.
pub const MAX_STREAM: (u32, u32) = (1920, 1080);

/// Scales (w, h) down to fit [`MAX_STREAM`], keeping the aspect ratio.
pub fn cap_size(width: u32, height: u32) -> (u32, u32) {
    let (mw, mh) = MAX_STREAM;
    if width <= mw && height <= mh {
        return (width.max(1), height.max(1));
    }
    let scale = (f64::from(mw) / f64::from(width)).min(f64::from(mh) / f64::from(height));
    (((f64::from(width) * scale).round() as u32).max(1), ((f64::from(height) * scale).round() as u32).max(1))
}

#[derive(Debug, Clone)]
pub struct StreamFrame {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Latest rendered frame, shared between the viewer thread and the URI scheme handler.
#[derive(Debug, Default)]
pub struct StreamSink {
    seq: AtomicU64,
    latest: Mutex<Option<StreamFrame>>,
}

impl StreamSink {
    pub fn publish(&self, width: u32, height: u32, rgba: Vec<u8>) -> u64 {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut l) = self.latest.lock() {
            *l = Some(StreamFrame { seq, width, height, rgba });
        }
        seq
    }

    pub fn latest(&self) -> Option<StreamFrame> {
        self.latest.lock().ok().and_then(|l| l.clone())
    }

    /// HTTP response for `ove-frame://localhost/latest`: raw RGBA with size headers.
    pub fn response(&self) -> tauri::http::Response<Vec<u8>> {
        let builder = tauri::http::Response::builder()
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Expose-Headers", "X-Frame-Width, X-Frame-Height, X-Frame-Seq")
            .header("Cache-Control", "no-store");
        let response = match self.latest() {
            Some(f) => builder
                .status(200)
                .header("Content-Type", "application/octet-stream")
                .header("X-Frame-Width", f.width.to_string())
                .header("X-Frame-Height", f.height.to_string())
                .header("X-Frame-Seq", f.seq.to_string())
                .body(f.rgba),
            None => builder.status(204).body(Vec::new()),
        };
        response.unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_large_rects() {
        assert_eq!(cap_size(640, 360), (640, 360));
        assert_eq!(cap_size(3840, 2160), (1920, 1080));
        assert_eq!(cap_size(4000, 1000), (1920, 480));
        assert_eq!(cap_size(0, 0), (1, 1));
    }

    #[test]
    fn response_carries_size_headers() {
        let sink = StreamSink::default();
        assert_eq!(sink.response().status(), 204);
        let seq = sink.publish(2, 1, vec![1; 8]);
        let r = sink.response();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["X-Frame-Width"], "2");
        assert_eq!(r.headers()["X-Frame-Seq"], seq.to_string().as_str());
        assert_eq!(r.body().len(), 8);
    }
}
