// stylize::vhs
fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed();
    let wob = (noise2(v * 40.0, t * 3.0, 21u) - 0.5) * p_wobble() * 0.006
        + sin(v * 3.0 + t * 1.7) * p_wobble() * 0.0015;
    // A tracking band that rolls down the frame.
    let band_y = fract1(t * 0.13);
    let band = sstep(0.035, 0.0, abs(v - band_y)) * p_scanlineNoise();
    let uu = u + wob + band * (noise2(v * 200.0, t * 30.0, 3u) - 0.5) * 0.04;
    let y = luma(sample(uu, v));
    // Chroma is band-limited on tape: smear it horizontally.
    let sm = p_smear() * 0.006;
    let ch = sample(uu - sm, v);
    let cy = luma(ch);
    let o = vec3<f32>(y + (ch.x - cy) * 0.85, y + (ch.y - cy) * 0.85, y + (ch.z - cy) * 0.85);
    let grain = (hash21(ti(u * info.w), ti(v * info.h), u32(max(t * 60.0, 0.0))) - 0.5) * 0.12 * p_scanlineNoise();
    let s = sin(v * info.h * PI * 0.5);
    let lines = 1.0 - 0.08 * (s * s) * p_scanlineNoise();
    return (o + vec3<f32>(grain + band * 0.25)) * lines;
}
