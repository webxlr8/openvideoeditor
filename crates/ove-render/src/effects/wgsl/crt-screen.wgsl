// stylize::crt_screen
fn crt_at(gx: f32, gy: f32, cell: f32, o: f32) -> vec3<f32> {
    return sample(clamp((floor(gx) + 0.5) * cell / info.w + o, 0.0, 1.0), (floor(gy) + 0.5) * cell / info.h);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    // Slight barrel curvature of the tube.
    let x0 = u * 2.0 - 1.0;
    let y0 = v * 2.0 - 1.0;
    let k = 0.06;
    let x = x0 * (1.0 + k * y0 * y0);
    let y = y0 * (1.0 + k * x0 * x0);
    let cu = x * 0.5 + 0.5;
    let cv = y * 0.5 + 0.5;
    if (!(cu >= 0.0 && cu <= 1.0) || !(cv >= 0.0 && cv <= 1.0)) {
        return vec3<f32>(0.0);
    }
    let rows = p_pixelSize();
    let cell = info.h / rows;
    let gx = cu * info.w / cell;
    let gy = cv * info.h / cell;
    let sh = p_colorShift() / info.w;
    let col = vec3<f32>(crt_at(gx, gy, cell, -sh).x, crt_at(gx, gy, cell, 0.0).y, crt_at(gx, gy, cell, sh).z);
    // Phosphor triads: each cell is three vertical stripes, faded out where they would only alias.
    let sub = min(u32(max(floor(fract1(gx) * 3.0), 0.0)), 2u);
    let lo = 1.0 - 0.65 * sstep(2.5, 6.0, cell);
    var mask = vec3<f32>(lo);
    mask[sub] = 1.0;
    let ss = sin(cv * p_scanlineFrequency() * PI);
    let scan = 1.0 - p_scanlineIntensity() * (ss * ss);
    let ex = cu - 0.5;
    let ey = cv - 0.5;
    let r = sqrt(ex * ex + ey * ey);
    let vr = max(p_vignetteRadius(), 0.01);
    let vig = 1.0 - p_vignetteIntensity() * sstep(vr * 0.6, vr * 0.6 + 0.45, r);
    let gain = p_brightness() * scan * vig * 1.35;
    return vec3<f32>(col.x * mask.x * gain, col.y * mask.y * gain, col.z * mask.z * gain);
}
