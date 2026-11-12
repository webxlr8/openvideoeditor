//! Video decoding: one FFmpeg decoder per open file, hardware first, exact seeking, frame conversion.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::ptr;

use ffmpeg_the_third::ffi;
use ffmpeg_the_third::frame::Video as AvFrame;
use ffmpeg_the_third::media::Type as MediaType;
use ffmpeg_the_third::{Error as AvError, Packet};

use crate::convert::Transfer;
use crate::error::{MediaError, Result};
use crate::frame::{DecodePath, OutputFormat, PixelFormat, Plane, VideoFrame};
use crate::util::{Micros, NOPTS, Rational, check_online, open_input, ts_to_us, us_to_ts};

/// A source of decoded video frames in presentation order, with exact seeking.
///
/// `seek(t)` positions the decoder so that the next `next_frame()` returns the frame whose presentation interval
/// contains `t` (the frame with the largest pts ≤ t; the first frame if `t` precedes it), followed by the frames
/// after it in order.
pub trait VideoDecoder: Send {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>>;
    fn seek(&mut self, t_us: Micros) -> Result<()>;
    fn decode_path(&self) -> DecodePath;
}

/// Hardware acceleration preference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HwAccel {
    /// The platform decoder: VideoToolbox on macOS, D3D11VA on Windows; none elsewhere.
    #[default]
    Auto,
    Off,
    /// A named FFmpeg device type (`vaapi`, `cuda`, …) — development only.
    Named(String),
}

impl HwAccel {
    /// `Auto`, overridden by `OVE_HWACCEL=off|<device>` for diagnostics.
    pub fn from_env() -> HwAccel {
        match std::env::var("OVE_HWACCEL").ok().as_deref() {
            None | Some("") | Some("auto") => HwAccel::Auto,
            Some("off") | Some("none") | Some("software") => HwAccel::Off,
            Some(name) => HwAccel::Named(name.to_string()),
        }
    }

    fn device_name(&self) -> Option<&str> {
        match self {
            HwAccel::Off => None,
            HwAccel::Named(n) => Some(n),
            HwAccel::Auto => {
                if cfg!(target_os = "macos") {
                    Some("videotoolbox")
                } else if cfg!(target_os = "windows") {
                    Some("d3d11va")
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DecoderOptions {
    pub hwaccel: HwAccel,
    pub output: OutputFormat,
    /// Fault injection for tests: on the hardware path, fail with a hardware error when this many frames have been
    /// returned. Lets the mid-stream fallback be exercised on real hardware.
    pub inject_hw_failure_after: Option<u64>,
}

struct HwDevice {
    device: *mut ffi::AVBufferRef,
    pix_fmt: ffi::AVPixelFormat,
    api: String,
}

// SAFETY: `device` is a counted reference to an AVHWDeviceContext, which FFmpeg allows to be used and released
// from any thread. `HwDevice` holds its own reference, is not `Clone`, and only reads the pointer (to hand it to
// `av_buffer_ref`) or releases it in `drop`, so moving it between threads cannot race.
unsafe impl Send for HwDevice {}

impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: `self.device` is the one reference `av_hwdevice_ctx_create` gave us; it is released exactly once,
        // here (`av_buffer_unref` nulls the pointer). Codec contexts that use the device hold their own references.
        unsafe { ffi::av_buffer_unref(&mut self.device) };
    }
}

struct RawFrame {
    frame: AvFrame,
    pts_us: Micros,
    dur_us: Micros,
}

/// FFmpeg-backed decoder for the best video stream of a file.
pub struct FfmpegVideoDecoder {
    path: PathBuf,
    input: ffmpeg_the_third::format::context::Input,
    stream_index: usize,
    tb: Rational,
    start_ts: i64,
    dec: ffmpeg_the_third::decoder::Video,
    hw: Option<HwDevice>,
    hw_active: bool,
    hw_fallback_reason: Option<String>,
    opts: DecoderOptions,
    sws: Sws,
    pending: VecDeque<RawFrame>,
    demux_eof: bool,
    eof_sent: bool,
    last_pts: Option<Micros>,
    frames_out: u64,
    reads_since_check: u32,
}

impl FfmpegVideoDecoder {
    pub fn open(path: &Path, opts: DecoderOptions) -> Result<Self> {
        let input = open_input(path)?;
        let st = input.streams().best(MediaType::Video).ok_or(MediaError::NoStream("video"))?;
        let stream_index = st.index();
        let tb: Rational = st.time_base().into();
        let start = st.start_time();
        let start_ts = if start == NOPTS { 0 } else { start };
        let mut ctx = ffmpeg_the_third::codec::context::Context::from_parameters(st.parameters())
            .map_err(|e| MediaError::from_open(path, e))?;

        // SAFETY: `ctx` owns a valid AVCodecContext (from_parameters succeeded) that has not been opened yet, so
        // the pointer is non-null and exclusively ours; these are plain-data fields read when it is opened.
        unsafe {
            let c = ctx.as_mut_ptr();
            (*c).thread_count = 0;
            (*c).thread_type = ffi::FF_THREAD_FRAME | ffi::FF_THREAD_SLICE;
            (*c).pkt_timebase = ffi::AVRational { num: tb.num, den: tb.den };
        }
        let codec_id = ctx.id();
        let mut hw_fallback_reason = None;
        let hw = match opts.hwaccel.device_name() {
            None => None,
            Some(name) => match create_hw_device(name, codec_id.into()) {
                Ok(dev) => {
                    // SAFETY: `ctx` is a valid, unopened codec context we own and `dev.device` a live device
                    // reference; the context gets a new reference of its own (freed with the context) and the
                    // field was null before, so nothing leaks.
                    unsafe { (*ctx.as_mut_ptr()).hw_device_ctx = ffi::av_buffer_ref(dev.device) };
                    Some(dev)
                }
                Err(reason) => {
                    hw_fallback_reason = Some(reason);
                    None
                }
            },
        };

        let dec = ctx.decoder().video().map_err(|e| match e {
            AvError::DecoderNotFound => MediaError::Unsupported {
                path: path.to_owned(),
                reason: format!("no decoder for {}", crate::util::codec_name(codec_id.into())),
            },
            e => MediaError::from_open(path, e),
        })?;

        Ok(FfmpegVideoDecoder {
            path: path.to_owned(),
            input,
            stream_index,
            tb,
            start_ts,
            dec,
            hw_active: hw.is_some(),
            hw,
            hw_fallback_reason,
            opts,
            sws: Sws::default(),
            pending: VecDeque::new(),
            demux_eof: false,
            eof_sent: false,
            last_pts: None,
            frames_out: 0,
            reads_since_check: 0,
        })
    }

    /// Why the hardware path is not in use, if it is not.
    pub fn hw_fallback_reason(&self) -> Option<&str> {
        self.hw_fallback_reason.as_deref()
    }

    fn hw_or_decode(&self, e: AvError) -> MediaError {
        if self.hw_active { MediaError::Hardware(e.to_string()) } else { MediaError::Decode(e.to_string()) }
    }

    /// Feed one packet of our stream to the decoder (or EOF). Returns false when nothing more can be fed.
    fn feed(&mut self) -> Result<bool> {
        if self.demux_eof {
            if !self.eof_sent {
                self.eof_sent = true;
                let _ = self.dec.send_eof();
                return Ok(true);
            }
            return Ok(false);
        }
        let mut packet = Packet::empty();
        loop {
            self.reads_since_check += 1;
            if self.reads_since_check >= 32 {
                self.reads_since_check = 0;
                check_online(&self.path)?;
            }
            match packet.read(&mut self.input) {
                Ok(()) => {
                    if packet.stream() != self.stream_index {
                        continue;
                    }
                    match self.dec.send_packet(&packet) {
                        Ok(()) => return Ok(true),
                        Err(AvError::InvalidData) if !self.hw_active => continue, // damaged packet: skip it
                        Err(AvError::Other { errno }) if errno == libc::EAGAIN => {
                            // Decoder full: caller drains frames first. Keep the packet by re-sending after drain.
                            return Err(MediaError::Decode("decoder refused input while full".into()));
                        }
                        Err(e) => return Err(self.hw_or_decode(e)),
                    }
                }
                Err(AvError::Eof) => {
                    self.demux_eof = true;
                    return self.feed();
                }
                Err(e) => {
                    // Read errors: the file went away (offline) or it is truncated/damaged (treat as end).
                    check_online(&self.path)?;
                    if matches!(e, AvError::Other { .. }) && !self.path.exists() {
                        return Err(MediaError::Offline { path: self.path.clone(), reason: e.to_string() });
                    }
                    self.demux_eof = true;
                    return self.feed();
                }
            }
        }
    }

    fn decode_raw(&mut self) -> Result<Option<RawFrame>> {
        loop {
            let mut f = AvFrame::empty();
            match self.dec.receive_frame(&mut f) {
                Ok(()) => {
                    // SAFETY: `receive_frame` succeeded, so `f` owns a valid AVFrame; these are plain-data reads.
                    let (best, dur, fmt) = unsafe {
                        let p = f.as_ptr();
                        ((*p).best_effort_timestamp, (*p).duration, (*p).format)
                    };
                    let ts = if best != NOPTS { Some(best) } else { f.pts() };
                    let pts_us = match ts {
                        Some(ts) => ts_to_us(ts - self.start_ts, self.tb),
                        None => self.last_pts.map(|p| p + 1).unwrap_or(0),
                    };
                    if let Some(hw) = &self.hw
                        && self.hw_active
                        && fmt != hw.pix_fmt.0
                    {
                        // The platform decoder declined this stream (e.g. unsupported profile).
                        self.hw_active = false;
                        self.hw_fallback_reason = Some("stream not supported by the hardware decoder".into());
                    }
                    let dur_us = if dur > 0 { ts_to_us(dur, self.tb) } else { 0 };
                    return Ok(Some(RawFrame { frame: f, pts_us, dur_us }));
                }
                Err(AvError::Other { errno }) if errno == libc::EAGAIN => {
                    if !self.feed()? {
                        return Ok(None);
                    }
                }
                Err(AvError::Eof) => return Ok(None),
                Err(AvError::InvalidData) if !self.hw_active => continue,
                Err(e) => return Err(self.hw_or_decode(e)),
            }
        }
    }

    fn convert(&mut self, raw: RawFrame) -> Result<VideoFrame> {
        let mut copies = 1;
        let mut frame = raw.frame;
        // SAFETY: `frame` owns a valid decoded AVFrame; `format` is a plain-data field.
        let fmt = unsafe { (*frame.as_ptr()).format };
        let is_hw = self.hw.as_ref().is_some_and(|h| fmt == h.pix_fmt.0);
        if is_hw {
            let mut sw = AvFrame::empty();
            // SAFETY: `frame` is a valid hardware frame (its format is the device's) and `sw` a valid empty frame
            // we own; `av_hwframe_transfer_data` allocates `sw`'s buffers, and props are copied only on success.
            let r = unsafe {
                let r = ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), frame.as_ptr(), 0);
                if r >= 0 {
                    ffi::av_frame_copy_props(sw.as_mut_ptr(), frame.as_ptr());
                }
                r
            };
            if r < 0 {
                return Err(MediaError::Hardware(AvError::from(r).to_string()));
            }
            frame = sw;
            copies += 1;
        }
        let path = if is_hw {
            DecodePath::Hardware(self.hw.as_ref().map(|h| h.api.clone()).unwrap_or_default())
        } else {
            DecodePath::Software
        };
        let (w, h) = (frame.width(), frame.height());
        // SAFETY: `frame` owns a valid AVFrame; `format` is a plain-data field.
        let src_fmt = ffi::AVPixelFormat(unsafe { (*frame.as_ptr()).format });
        // SAFETY: `frame` owns a valid AVFrame for the duration of the call (the contract of `color_info`).
        let mut color = unsafe { crate::convert::color_info(frame.as_ptr()) };
        let hdr = color.transfer != Transfer::Sdr;
        let (format, planes) = match self.opts.output {
            OutputFormat::Native => {
                let name = crate::util::pix_fmt_info(src_fmt).map(|i| i.name).unwrap_or_default();
                (PixelFormat::Native(name), pack_planes(&frame, src_fmt)?)
            }
            OutputFormat::Display if hdr => {
                copies += 1;
                self.rgba(&frame, src_fmt, w, h, &mut color)?
            }
            OutputFormat::Display | OutputFormat::DisplayHdr => {
                if src_fmt == ffi::AVPixelFormat::NV12 {
                    (PixelFormat::Nv12, pack_planes(&frame, src_fmt)?)
                } else if src_fmt == ffi::AVPixelFormat::P010LE {
                    (PixelFormat::P010, pack_planes(&frame, src_fmt)?)
                // SAFETY: `frame` is a valid software frame in CPU memory (hardware frames were transferred above)
                // and outlives the call (the contract of `to_semiplanar`).
                } else if let Some(out) = unsafe { crate::convert::to_semiplanar(frame.as_ptr()) } {
                    out
                } else {
                    copies += 1;
                    let target = ffi::AVPixelFormat::RGBA;
                    let out = self.sws.convert(&frame, src_fmt, target, w, h)?;
                    (PixelFormat::Rgba8, pack_planes(&out, target)?)
                }
            }
            OutputFormat::Rgba { max_width, max_height } => {
                let fit = |limit: u32, v: u32| if limit == 0 { 1.0 } else { (limit as f64 / v as f64).min(1.0) };
                let s = fit(max_width, w).min(fit(max_height, h));
                let (ow, oh) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
                copies += 1;
                self.rgba(&frame, src_fmt, ow, oh, &mut color)?
            }
        };
        let (w, h) = match format {
            PixelFormat::Rgba8 => (planes[0].width, planes[0].height),
            _ => (w, h),
        };
        Ok(VideoFrame {
            color,
            pts_us: raw.pts_us,
            duration_us: raw.dur_us,
            width: w,
            height: h,
            format,
            planes,
            decode_path: path,
            copies,
        })
    }

    /// RGBA8 at `ow`×`oh`: HDR (PQ/HLG) is tone-mapped to SDR BT.709, everything else goes through swscale.
    fn rgba(
        &mut self,
        frame: &AvFrame,
        src_fmt: ffi::AVPixelFormat,
        ow: u32,
        oh: u32,
        color: &mut crate::convert::ColorInfo,
    ) -> Result<(PixelFormat, Vec<Plane>)> {
        if color.transfer != Transfer::Sdr {
            let scaled;
            let (w, h) = (frame.width(), frame.height());
            // SAFETY: `frame` (a software frame) and `scaled` (allocated by swscale) are valid CPU frames, and the
            // returned view borrows one of them; both outlive `src`, which is used only within this function
            // (the contract of `yuv_source`).
            let src = unsafe {
                match crate::convert::yuv_source(frame.as_ptr()) {
                    Some(src) if (ow, oh) == (w, h) => Some(src),
                    _ => {
                        // Scale (and widen other layouts) in Y'CbCr first, so only output pixels are tone-mapped.
                        let target = ffi::AVPixelFormat::YUV444P16LE;
                        scaled = self.sws.convert_yuv(frame, src_fmt, target, ow, oh)?;
                        crate::convert::yuv_source(scaled.as_ptr())
                    }
                }
            };
            if let Some(src) = src {
                let (kr, kb) = color.matrix.coefficients();
                let params = crate::tonemap::YuvParams { kr, kb, full_range: color.full_range };
                let mut data = vec![0u8; src.width * src.height * 4];
                crate::tonemap::tone_map_yuv(&src, params, color.transfer, color.bt2020_primaries, &mut data);
                color.transfer = Transfer::Sdr;
                let plane = Plane { data, stride: src.width * 4, width: src.width as u32, height: src.height as u32 };
                return Ok((PixelFormat::Rgba8, vec![plane]));
            }
        }
        let target = ffi::AVPixelFormat::RGBA;
        let out = self.sws.convert(frame, src_fmt, target, ow, oh)?;
        Ok((PixelFormat::Rgba8, pack_planes(&out, target)?))
    }

    fn next_raw(&mut self) -> Result<Option<RawFrame>> {
        if let Some(f) = self.pending.pop_front() {
            return Ok(Some(f));
        }
        self.decode_raw()
    }

    fn reset_decoder(&mut self) {
        self.dec.flush();
        self.pending.clear();
        self.demux_eof = false;
        self.eof_sent = false;
    }

    fn seek_container(&mut self, ts: i64) -> Result<()> {
        // SAFETY: `self.input` owns an open AVFormatContext for as long as `self` lives and `stream_index` is one
        // of its streams; the call only takes plain values besides the context.
        let r =
            unsafe { ffi::avformat_seek_file(self.input.as_mut_ptr(), self.stream_index as i32, i64::MIN, ts, ts, 0) };
        if r < 0 {
            // Some demuxers only support backward seeking by timestamp.
            // SAFETY: same context and stream as the `avformat_seek_file` call above.
            let r2 = unsafe {
                ffi::av_seek_frame(self.input.as_mut_ptr(), self.stream_index as i32, ts, ffi::AVSEEK_FLAG_BACKWARD)
            };
            if r2 < 0 {
                check_online(&self.path)?;
                return Err(MediaError::Decode(format!("seek failed: {}", AvError::from(r2))));
            }
        }
        self.reset_decoder();
        Ok(())
    }
}

impl VideoDecoder for FfmpegVideoDecoder {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        if let Some(k) = self.opts.inject_hw_failure_after
            && self.hw_active
            && self.frames_out >= k
        {
            return Err(MediaError::Hardware(format!("injected failure after {k} frames")));
        }
        let Some(raw) = self.next_raw()? else {
            return Ok(None);
        };
        self.last_pts = Some(raw.pts_us);
        self.frames_out += 1;
        self.convert(raw).map(Some)
    }

    fn seek(&mut self, t_us: Micros) -> Result<()> {
        let target = self.start_ts + us_to_ts(t_us, self.tb);
        let mut back_us: Micros = 0;
        for _ in 0..12 {
            let seek_ts = target - us_to_ts(back_us, self.tb);
            let at_start = seek_ts <= self.start_ts;
            self.seek_container(if at_start { self.start_ts.min(seek_ts) } else { seek_ts })?;
            let mut before: Option<RawFrame> = None;
            let mut after: Option<RawFrame> = None;
            while let Some(f) = self.decode_raw()? {
                if f.pts_us <= t_us {
                    before = Some(f);
                } else {
                    after = Some(f);
                    break;
                }
            }
            match (before, after) {
                (Some(b), a) => {
                    self.pending.push_back(b);
                    self.pending.extend(a);
                    return Ok(());
                }
                (None, Some(a)) if at_start => {
                    // Target precedes the first frame: show the first frame.
                    self.pending.push_back(a);
                    return Ok(());
                }
                (None, None) if at_start => return Ok(()), // empty stream
                _ => {
                    // Landed after the target (imprecise index): go further back.
                    back_us = if back_us == 0 { 1_000_000 } else { back_us * 2 };
                }
            }
        }
        Err(MediaError::Decode(format!("could not seek to {t_us} µs")))
    }

    fn decode_path(&self) -> DecodePath {
        match (&self.hw, self.hw_active) {
            (Some(h), true) => DecodePath::Hardware(h.api.clone()),
            _ => DecodePath::Software,
        }
    }
}

fn create_hw_device(name: &str, codec: ffi::AVCodecID) -> std::result::Result<HwDevice, String> {
    let cname = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    // SAFETY: `cname` is a NUL-terminated string that outlives the lookup. `avcodec_find_decoder` and
    // `avcodec_get_hw_config` return null or pointers into static tables (null ends the loop before any
    // dereference). `device` starts null and is either left null on failure or set to a context we then own
    // (released by `HwDevice::drop`).
    unsafe {
        let ty = ffi::av_hwdevice_find_type_by_name(cname.as_ptr());
        if ty == ffi::AVHWDeviceType::NONE {
            return Err(format!("{name} is not available in this FFmpeg build"));
        }
        let codec_ptr = ffi::avcodec_find_decoder(codec);
        if codec_ptr.is_null() {
            return Err("no decoder".into());
        }
        let mut pix_fmt = None;
        let mut i = 0;
        loop {
            let cfg = ffi::avcodec_get_hw_config(codec_ptr, i);
            if cfg.is_null() {
                break;
            }
            if (*cfg).device_type == ty && ((*cfg).methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX.0 as i32) != 0
            {
                pix_fmt = Some((*cfg).pix_fmt);
                break;
            }
            i += 1;
        }
        let Some(pix_fmt) = pix_fmt else {
            return Err(format!("{name} cannot decode this codec"));
        };
        let mut device = ptr::null_mut();
        let r = ffi::av_hwdevice_ctx_create(&mut device, ty, ptr::null(), ptr::null_mut(), 0);
        if r < 0 {
            return Err(format!("{name} device unavailable: {}", AvError::from(r)));
        }
        Ok(HwDevice { device, pix_fmt, api: name.to_string() })
    }
}

/// Copy a frame's planes into tightly packed buffers, exactly like `av_image_copy_to_buffer(align = 1)`.
fn pack_planes(frame: &AvFrame, fmt: ffi::AVPixelFormat) -> Result<Vec<Plane>> {
    let (w, h) = (frame.width() as i32, frame.height() as i32);
    // SAFETY: the descriptor is a non-null pointer into FFmpeg's static table (checked). `frame` is a valid CPU
    // frame whose format is `fmt` (all callers pass its own format, or the format it was allocated with), so plane
    // `i` holds at least `ph` rows of `linesize` bytes, each at least `av_image_get_linesize` bytes long: every
    // slice read stays inside the plane.
    unsafe {
        let desc = ffi::av_pix_fmt_desc_get(fmt);
        if desc.is_null() {
            return Err(MediaError::Decode("unknown pixel format".into()));
        }
        let n = ffi::av_pix_fmt_count_planes(fmt);
        let p = frame.as_ptr();
        let mut planes = Vec::with_capacity(n as usize);
        for i in 0..n {
            let row = ffi::av_image_get_linesize(fmt, w, i);
            if row < 0 {
                return Err(MediaError::Decode("bad plane layout".into()));
            }
            let chroma = (i == 1 || i == 2) && ((*desc).flags & ffi::AV_PIX_FMT_FLAG_RGB as u64) == 0;
            let ph = if chroma { (h + (1 << (*desc).log2_chroma_h) - 1) >> (*desc).log2_chroma_h } else { h };
            let pw = if chroma { (w + (1 << (*desc).log2_chroma_w) - 1) >> (*desc).log2_chroma_w } else { w };
            let src = (*p).data[i as usize];
            let ls = (*p).linesize[i as usize] as isize;
            let mut data = Vec::with_capacity((row * ph) as usize);
            for y in 0..ph as isize {
                let line = std::slice::from_raw_parts(src.offset(y * ls), row as usize);
                data.extend_from_slice(line);
            }
            planes.push(Plane { data, stride: row as usize, width: pw as u32, height: ph as u32 });
        }
        Ok(planes)
    }
}

// swscale constants as plain values: their binding types differ between FFmpeg major versions.
/// `SWS_BILINEAR | SWS_ACCURATE_RND`.
const SWS_FLAGS: i32 = 0x2 | 0x40000;
const SWS_CS_ITU709: i32 = 1;
const SWS_CS_ITU601: i32 = 5;

/// Cached swscale context for format conversion (and scaling).
#[derive(Default)]
struct Sws {
    ctx: Option<*mut ffi::SwsContext>,
}

// SAFETY: an SwsContext has no thread affinity. `Sws` owns its context exclusively (not `Clone`, never shared) and
// only uses it through `&mut self`, so moving it to another thread cannot cause concurrent access.
unsafe impl Send for Sws {}

impl Sws {
    fn convert(
        &mut self,
        src: &AvFrame,
        from: ffi::AVPixelFormat,
        to: ffi::AVPixelFormat,
        out_w: u32,
        out_h: u32,
    ) -> Result<AvFrame> {
        self.convert_with(src, from, to, out_w, out_h, false)
    }

    /// Y'CbCr → Y'CbCr (scaling, re-layout) keeping the source's range and matrix.
    fn convert_yuv(
        &mut self,
        src: &AvFrame,
        from: ffi::AVPixelFormat,
        to: ffi::AVPixelFormat,
        out_w: u32,
        out_h: u32,
    ) -> Result<AvFrame> {
        self.convert_with(src, from, to, out_w, out_h, true)
    }

    fn convert_with(
        &mut self,
        src: &AvFrame,
        from: ffi::AVPixelFormat,
        to: ffi::AVPixelFormat,
        out_w: u32,
        out_h: u32,
        keep_yuv: bool,
    ) -> Result<AvFrame> {
        let (w, h) = (src.width() as i32, src.height() as i32);
        let (ow, oh) = (out_w as i32, out_h as i32);
        // SAFETY: `sws_getCachedContext` takes ownership of the previous context (reusing or freeing it) and
        // returns null or a context we own, which replaces it in `self.ctx`. `src` is a valid CPU frame of
        // `w`×`h` in format `from`, and `out` is allocated by `AvFrame::new` as `ow`×`oh` in `to`, so `sws_scale`
        // reads and writes only within the planes of the two frames. The coefficient tables are static.
        unsafe {
            let ctx = ffi::sws_getCachedContext(
                self.ctx.unwrap_or(ptr::null_mut()),
                w,
                h,
                from,
                ow,
                oh,
                to,
                SWS_FLAGS,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if ctx.is_null() {
                self.ctx = None;
                return Err(MediaError::Decode("no pixel format converter".into()));
            }
            self.ctx = Some(ctx);
            let sp = src.as_ptr();
            // Colour matrix and range from the frame; BT.709 for HD and up when unspecified.
            let space = match (*sp).colorspace {
                ffi::AVColorSpace::UNSPECIFIED | ffi::AVColorSpace::RGB => {
                    if h >= 720 {
                        SWS_CS_ITU709
                    } else {
                        SWS_CS_ITU601
                    }
                }
                s => s.0 as i32,
            };
            let full = ((*sp).color_range == ffi::AVColorRange::JPEG) as i32;
            let coeffs = ffi::sws_getCoefficients(space);
            let (dst_coeffs, dst_full) =
                if keep_yuv { (coeffs, full) } else { (ffi::sws_getCoefficients(SWS_CS_ITU601), 1) };
            ffi::sws_setColorspaceDetails(ctx, coeffs, full, dst_coeffs, dst_full, 0, 1 << 16, 1 << 16);
            let mut out = AvFrame::new(to.into(), ow as u32, oh as u32);
            ffi::sws_scale(
                ctx,
                (*sp).data.as_ptr() as *const *const u8,
                (*sp).linesize.as_ptr(),
                0,
                h,
                (*out.as_mut_ptr()).data.as_ptr(),
                (*out.as_mut_ptr()).linesize.as_ptr(),
            );
            Ok(out)
        }
    }
}

impl Drop for Sws {
    fn drop(&mut self) {
        if let Some(c) = self.ctx.take() {
            // SAFETY: `c` came from `sws_getCachedContext`, is owned only by `self` and was taken out of `self.ctx`,
            // so it is freed exactly once.
            unsafe { ffi::sws_freeContext(c) };
        }
    }
}
