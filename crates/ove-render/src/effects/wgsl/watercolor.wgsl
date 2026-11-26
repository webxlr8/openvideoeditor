// stylize::watercolor
// Rust f32::round: halves away from zero (WGSL round goes to even).
fn rround(x: f32) -> f32 {
    let t = trunc(x);
    if (abs(x - t) >= 0.5) {
        return t + sign(x);
    }
    return t;
}

fn pool(x: f32) -> f32 {
    return (rround(x * 6.0) / 6.0) * 0.6 + x * 0.4;
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let r = p_brushSize() * 2.0;
    var acc = vec3<f32>(0.0);
    let n = 8u;
    let j = noise2(u * info.w / 7.0, v * info.h / 7.0, 11u) * TAU;
    for (var i = 0u; i < n; i = i + 1u) {
        let a = j + f32(i) * 2.399963;
        let rr = r * sqrt((f32(i) + 0.5) / f32(n));
        acc = acc + sample(u + cos(a) * rr / info.w, v + sin(a) * rr / info.h);
    }
    let m = acc * (1.0 / f32(n));
    let wash = vec3<f32>(pool(m.x), pool(m.y), pool(m.z));
    let e = 1.5 / info.h;
    let g = abs(luma(sample(u + e * info.h / info.w, v)) - luma(sample(u - e * info.h / info.w, v)))
        + abs(luma(sample(u, v + e)) - luma(sample(u, v - e)));
    let pooled = wash * (1.0 - min(g * p_bleed() * 1.2, 0.35));
    let grain = fbm(u * info.aspect * 120.0, v * 120.0, 3u, 5u);
    let paper = p_paper();
    let tex = 1.0 - paper * 0.25 * (grain - 0.5) * 2.0;
    let lit = sstep(0.75, 1.0, luma(pooled)) * paper;
    let out = lerp3(pooled * tex, p_paperColor(), lit);
    return lerp3(sample(u, v), out, p_strength());
}
