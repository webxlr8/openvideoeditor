// stylize::pixelate
fn effect(u: f32, v: f32) -> vec3<f32> {
    let cell = info.w / max(p_cells(), 1.0); // square cells in pixels
    let gx = u * info.w / cell;
    let gy = v * info.h / cell;
    let cx = floor(gx) + 0.5;
    let cy = floor(gy) + 0.5;
    let col = sample(cx * cell / info.w, cy * cell / info.h);
    let gap = p_gap();
    let half = 0.5 * (1.0 - gap);
    if (gap <= 0.0 && p_roundness() <= 0.0) {
        return col;
    }
    let rad = p_roundness() * half;
    let dx = max(abs(gx - cx) - half + rad, 0.0);
    let dy = max(abs(gy - cy) - half + rad, 0.0);
    let outside = sqrt(dx * dx + dy * dy) - rad + min(max(abs(gx - cx) - half, abs(gy - cy) - half), 0.0);
    let aa = 1.0 / cell;
    return col * (1.0 - sstep(-aa, aa, outside));
}
