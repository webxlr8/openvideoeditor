// background::marble
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

// util::ramp over three colours.
fn ramp3(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    var cols = array<vec3<f32>, 3>(a, b, c);
    let tt = clamp(t, 0.0, 1.0) * 2.0;
    let i = min(u32(tt), 1u);
    return lerp3(cols[i], cols[i + 1u], tt - f32(i));
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed();
    let seed = u32(max(p_seed(), 0.0));
    let c0 = centred(u, v);
    let x = c0.x;
    let y = c0.y;
    let s = p_scale() * 4.0;
    let turb = fbm(x * s + t, y * s, 5u, seed) * p_turbulence() * 0.25;
    let vein = sin((x + y * 0.6) * s * 2.0 + turb) * 0.5 + 0.5;
    var k = 0.0;
    if (vein > 0.0) {
        k = pow(vein, 0.6);
    }
    return ramp3(p_colorC(), p_colorB(), p_colorA(), k);
}
