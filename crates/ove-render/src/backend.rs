//! Compositing backends (design D1). The renderer evaluates the timeline and orders layers; a backend holds the
//! images and runs the four pixel operations on them: draw a layer, apply a grade (the basic adjustments or a baked
//! lattice), apply an effect, and run a transition (mask and post effect).
//!
//! Images live inside the backend and are named by [`Tex`] handles, so a GPU backend keeps them as textures between
//! operations. [`Backend::cpu`] runs a CPU function on an image: on the CPU backend that is a plain call, and a GPU
//! backend reads the image back, runs the function and uploads the result. Every operation has a default that uses
//! it with the CPU code, so a backend only overrides what it does natively and every project stays renderable.
//!
//! [`CpuBackend`] is the reference (the "oracle"): its output is what the parity tests compare other backends with.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::canvas::{self, DrawParams, Place, Rgba};
use crate::color::{Lut, apply_lut};
use crate::eval::Grade;
use crate::transitions::TransState;

/// An image held by a backend. Valid until [`Backend::release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tex(pub u32);

/// The compositing operations the renderer needs. Pixel formats and rounding follow the CPU backend: straight-alpha
/// RGBA, 8 bits per channel between operations.
pub trait Backend: Send {
    /// `"cpu"` or `"gpu"`.
    fn name(&self) -> &'static str;

    /// A new `w`×`h` image filled with `fill`.
    fn create(&mut self, w: u32, h: u32, fill: [u8; 4]) -> Tex;
    /// An image with the pixels of `img` (the CPU backend shares the buffer until it is changed).
    fn upload(&mut self, img: &Arc<Rgba>) -> Tex;
    /// A copy of the pixels of `t`.
    fn read(&mut self, t: Tex) -> Rgba;
    /// Frees `t`.
    fn release(&mut self, t: Tex);
    /// The size of `t`.
    fn size(&self, t: Tex) -> (u32, u32);
    /// Runs `f` on the pixels of `t` and keeps its changes.
    fn cpu(&mut self, t: Tex, f: &mut dyn FnMut(&mut Rgba));

    /// False once the backend can no longer produce frames (a lost GPU device); the renderer then switches to the
    /// CPU backend.
    fn healthy(&self) -> bool {
        true
    }

    /// A copy of `t` as a new image.
    fn copy(&mut self, t: Tex) -> Tex {
        let img = Arc::new(self.read(t));
        self.upload(&img)
    }

    /// Draws `src` onto `dst` (affine placement, crop, corner radius, opacity, blend mode).
    fn draw(&mut self, dst: Tex, src: Tex, place: Place, p: &DrawParams) {
        let s = self.read(src);
        self.cpu(dst, &mut |d| canvas::draw(d, &s, place, p));
    }

    /// The basic adjustments (`colorGrading`).
    fn grade(&mut self, t: Tex, g: &Grade) {
        self.cpu(t, &mut |img| canvas::grade(img, g));
    }

    /// A baked grade or LUT (a lattice with tetrahedral lookup), mixed by `intensity`.
    fn lut(&mut self, t: Tex, lut: &Arc<Lut>, intensity: f32) {
        self.cpu(t, &mut |img| apply_lut(img, lut, intensity));
    }

    /// Effect `id` with parameter `values`, mixed by `amount`, at item-relative time `time` seconds. False for an
    /// unknown id (the image is left untouched).
    fn effect(&mut self, t: Tex, id: &str, values: &Map<String, Value>, amount: f32, time: f32) -> bool {
        let mut known = false;
        self.cpu(t, &mut |img| known = crate::effects::apply(img, id, values, amount, time));
        known
    }

    /// A transition's reveal mask on a layer's alpha.
    fn transition_mask(&mut self, t: Tex, st: &TransState) {
        if st.has_mask() {
            self.cpu(t, &mut |img| st.apply_mask(img));
        }
    }

    /// `dst = before + (dst − before)·k` (partial-strength adjustment layers).
    fn mix(&mut self, dst: Tex, before: Tex, k: f32) {
        let b = self.read(before);
        self.cpu(dst, &mut |d| mix_back(d, &b, k));
    }

    /// Makes every pixel opaque (alpha 255), keeping the colour.
    fn opaque(&mut self, t: Tex) {
        self.cpu(t, &mut |img| {
            for p in img.data.chunks_exact_mut(4) {
                p[3] = 255;
            }
        });
    }
}

/// `dst = before + (dst − before)·k`.
pub fn mix_back(dst: &mut Rgba, before: &Rgba, k: f32) {
    use rayon::prelude::*;
    dst.data.par_iter_mut().zip(before.data.par_iter()).for_each(|(d, b)| {
        *d = (*b as f32 + (*d as f32 - *b as f32) * k).round().clamp(0.0, 255.0) as u8;
    });
}

/// Images stored by handle, with free slots reused.
pub struct Slab<T> {
    items: Vec<Option<T>>,
    free: Vec<u32>,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Slab { items: Vec::new(), free: Vec::new() }
    }
}

impl<T> Slab<T> {
    pub fn insert(&mut self, v: T) -> Tex {
        match self.free.pop() {
            Some(i) => {
                self.items[i as usize] = Some(v);
                Tex(i)
            }
            None => {
                self.items.push(Some(v));
                Tex(self.items.len() as u32 - 1)
            }
        }
    }

    pub fn get(&self, t: Tex) -> Option<&T> {
        self.items.get(t.0 as usize).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, t: Tex) -> Option<&mut T> {
        self.items.get_mut(t.0 as usize).and_then(Option::as_mut)
    }

    pub fn remove(&mut self, t: Tex) -> Option<T> {
        let v = self.items.get_mut(t.0 as usize).and_then(Option::take);
        if v.is_some() {
            self.free.push(t.0);
        }
        v
    }

    /// Images currently held.
    pub fn len(&self) -> usize {
        self.items.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The reference backend: today's CPU compositor on RGBA8 buffers (rows in parallel).
#[derive(Default)]
pub struct CpuBackend {
    images: Slab<Arc<Rgba>>,
}

impl CpuBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Images currently held (tests check that the renderer releases what it creates).
    pub fn held(&self) -> usize {
        self.images.len()
    }

    fn img(&self, t: Tex) -> &Arc<Rgba> {
        static EMPTY: std::sync::OnceLock<Arc<Rgba>> = std::sync::OnceLock::new();
        self.images.get(t).unwrap_or_else(|| EMPTY.get_or_init(|| Arc::new(Rgba::new(0, 0))))
    }
}

impl Backend for CpuBackend {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn create(&mut self, w: u32, h: u32, fill: [u8; 4]) -> Tex {
        let img = if fill == [0; 4] { Rgba::new(w, h) } else { Rgba::filled(w, h, fill) };
        self.images.insert(Arc::new(img))
    }

    fn upload(&mut self, img: &Arc<Rgba>) -> Tex {
        self.images.insert(img.clone())
    }

    fn read(&mut self, t: Tex) -> Rgba {
        (**self.img(t)).clone()
    }

    fn release(&mut self, t: Tex) {
        self.images.remove(t);
    }

    fn size(&self, t: Tex) -> (u32, u32) {
        let i = self.img(t);
        (i.w, i.h)
    }

    fn cpu(&mut self, t: Tex, f: &mut dyn FnMut(&mut Rgba)) {
        if let Some(img) = self.images.get_mut(t) {
            f(Arc::make_mut(img));
        }
    }

    fn copy(&mut self, t: Tex) -> Tex {
        let img = self.img(t).clone();
        self.images.insert(img)
    }

    fn draw(&mut self, dst: Tex, src: Tex, place: Place, p: &DrawParams) {
        let Some(s) = self.images.get(src).cloned() else { return };
        self.cpu(dst, &mut |d| canvas::draw(d, &s, place, p));
    }

    fn mix(&mut self, dst: Tex, before: Tex, k: f32) {
        let Some(b) = self.images.get(before).cloned() else { return };
        self.cpu(dst, &mut |d| mix_back(d, &b, k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slab_reuses_slots() {
        let mut s = Slab::default();
        let a = s.insert(1);
        let b = s.insert(2);
        assert_eq!(s.remove(a), Some(1));
        assert_eq!(s.remove(a), None, "a second release is harmless");
        let c = s.insert(3);
        assert_eq!(c, a);
        assert_eq!((s.get(b), s.get(c), s.len()), (Some(&2), Some(&3), 2));
    }

    #[test]
    fn cpu_backend_shares_until_written() {
        let mut b = CpuBackend::new();
        let src = Arc::new(Rgba::filled(4, 4, [10, 20, 30, 255]));
        let t = b.upload(&src);
        b.cpu(t, &mut |img| img.data[0] = 99);
        assert_eq!(src.data[0], 10, "the uploaded frame is not changed in place");
        assert_eq!(b.read(t).data[0], 99);
        let c = b.copy(t);
        b.opaque(c);
        let canvas = b.create(4, 4, [0, 0, 0, 255]);
        b.draw(canvas, t, Place { cx: 2.0, cy: 2.0, sx: 1.0, sy: 1.0, rotation: 0.0 }, &DrawParams::default());
        assert_eq!(b.read(canvas).px(0, 0), [99, 20, 30, 255]);
        for x in [t, c, canvas] {
            b.release(x);
        }
        assert_eq!(b.held(), 0);
    }
}
