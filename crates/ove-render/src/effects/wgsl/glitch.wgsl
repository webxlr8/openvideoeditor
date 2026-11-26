// stylize::glitch
fn effect(u: f32, v: f32) -> vec3<f32> {
    let i = p_intensity();
    let seed = u32(max(p_seed() * 1000.0, 0.0));
    let tick = ti(floor(info.t * p_speed() * 12.0));
    let dens = p_blockDensity();
    let bx = ti(floor(u * dens * 0.5));
    let by = ti(floor(v * dens));
    var uu = u;
    let row = hash21(by, tick, seed);
    if (row < i * 0.45) {
        uu = uu + (hash21(bx, by + tick * 7, seed + 1u) - 0.5) * 0.25 * i;
    }
    if (hash21(bx, by + tick * 3, seed + 2u) < p_mirrorAmount() * i * 0.3) {
        uu = 1.0 - uu;
    }
    let shift = p_rgbShift() / info.w * (0.4 + i) * select(1.0, 2.0, row < i);
    var o = vec3<f32>(sample(uu + shift, v).x, sample(uu, v).y, sample(uu - shift, v).z);
    let bar = hash21(0, ti(floor(v * 40.0)) + tick * 13, seed + 3u);
    if (bar < p_colorBarIntensity() * i * 0.25) {
        let tint = vec3<f32>(hash21(1, tick, seed), hash21(2, tick, seed), hash21(3, tick, seed));
        o = lerp3(o, tint, 0.6);
    }
    var line = 1.0;
    if (ti(v * info.h) % 2 == 0) {
        line = 1.0 - p_scanlineIntensity() * 0.5;
    }
    return o * line;
}
