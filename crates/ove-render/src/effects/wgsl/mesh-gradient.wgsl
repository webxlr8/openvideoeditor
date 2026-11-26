// background::mesh_gradient
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.15;
    let c0 = centred(u, v);
    let r = sqrt(c0.x * c0.x + c0.y * c0.y);
    let q = rot(c0.x, c0.y, p_swirl() * max(1.0 - r * 1.5, 0.0) * 180.0);
    let x = q.x;
    let y = q.y;
    var cols = array<vec3<f32>, 5>(p_color1(), p_color2(), p_color3(), p_color4(), p_color5());
    let pw = 1.0 + p_smoothness() * 0.8;
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < 5u; i = i + 1u) {
        let fi = f32(i);
        let qx = sin(t * (0.7 + fi * 0.13) + fi * 1.7) * 0.45 * info.aspect;
        let qy = cos(t * (0.5 + fi * 0.11) + fi * 2.9) * 0.38;
        let dx = x - qx;
        let dy = y - qy;
        let d = max(sqrt(dx * dx + dy * dy), 1e-3);
        let w = 1.0 / pow(d, pw * 1.6);
        acc = acc + cols[i] * w;
        wsum = wsum + w;
    }
    return acc * (1.0 / wsum);
}
