//! Composited timeline frames, rendered by the export compositor.

use std::path::Path;

use ove_core::model::Document;
use ove_render::Renderer;
use ove_render::eval::reference_size;

use crate::image::Image;
use crate::{Error, Result};

/// Output height for a canvas `width` pixels wide in the project's aspect ratio.
pub fn height_for_width(doc: &Document, width: u32) -> u32 {
    let (rw, rh) = reference_size(&doc.project.aspect_ratio);
    ((width as f64 * rh as f64 / rw as f64).round() as u32).max(2)
}

/// Renders the timeline at each of `times` (seconds) at about `width` pixels wide (the compositor keeps sizes even).
pub fn look(doc: &Document, bundle: Option<&Path>, times: &[f64], width: u32) -> Result<Vec<Image>> {
    if width < 2 {
        return Err(Error::Invalid("width must be at least 2 pixels".into()));
    }
    ove_media::init();
    let mut r = Renderer::new(bundle, height_for_width(doc, width));
    Ok(times.iter().map(|t| Image::from(r.render(doc, (t.max(0.0) * 1_000_000.0).round() as i64))).collect())
}

/// `count` evenly spaced times over the timeline (centres of equal slices), or one every `every` seconds.
pub fn timeline_times(doc: &Document, count: u32, every: Option<f64>) -> Vec<f64> {
    let dur = doc.project.items.values().map(|i| i.end).max().unwrap_or(0) as f64 / 1_000_000.0;
    let opts = crate::video::FilmstripOpts { count, every, ..Default::default() };
    crate::video::filmstrip_times(&opts, dur)
}
