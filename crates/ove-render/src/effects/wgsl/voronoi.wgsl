// background::voronoi
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed();
    let c0 = centred(u, v);
    let s = p_scale();
    let gx = c0.x * s;
    let gy = c0.y * s;
    let ix = ti(floor(gx));
    let iy = ti(floor(gy));
    var d1 = 9.0;
    var d2 = 9.0;
    var idx = 0;
    var idy = 0;
    for (var oy = -1; oy <= 1; oy = oy + 1) {
        for (var ox = -1; ox <= 1; ox = ox + 1) {
            let cx = ix + ox;
            let cy = iy + oy;
            let h1 = hash21(cx, cy, 81u);
            let h2 = hash21(cx, cy, 82u);
            let qx = f32(cx) + 0.5 + 0.4 * sin(t + h1 * TAU);
            let qy = f32(cy) + 0.5 + 0.4 * cos(t * 0.8 + h2 * TAU);
            let dx = gx - qx;
            let dy = gy - qy;
            let d = sqrt(dx * dx + dy * dy);
            if (d < d1) {
                d2 = d1;
                d1 = d;
                idx = cx;
                idy = cy;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    let fill = lerp3(p_colorA(), p_colorB(), hash21(idx, idy, 83u));
    let edge = 1.0 - sstep(0.0, p_edgeSoftness() + 0.02, d2 - d1);
    return lerp3(fill, p_borderColor(), edge * p_edgeIntensity());
}
