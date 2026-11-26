// background::flowing_gradient
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

// util::ramp over four colours.
fn ramp4(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>, t: f32) -> vec3<f32> {
    var cols = array<vec3<f32>, 4>(a, b, c, d);
    let tt = clamp(t, 0.0, 1.0) * 3.0;
    let i = min(u32(tt), 2u);
    return lerp3(cols[i], cols[i + 1u], tt - f32(i));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.06;
    let seed = u32(max(p_seed(), 0.0));
    let c0 = centred(u, v);
    let x = c0.x;
    let y = c0.y;
    let d = p_distortion();
    let wx = fbm(x * 1.4 + t, y * 1.4, 4u, seed) - 0.5;
    let wy = fbm(x * 1.4, y * 1.4 - t, 4u, seed + 9u) - 0.5;
    let k = fbm(x * 1.1 + wx * d * 3.0, y * 1.1 + wy * d * 3.0 + t * 0.5, 4u, seed + 17u);
    let s = sstep(0.2, 0.8, k * 0.7 + (v + x * 0.2) * 0.3);
    return ramp4(p_colorA(), p_colorB(), p_colorC(), p_colorD(), s);
}
