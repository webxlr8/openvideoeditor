// background::studio_background
fn centred(u: f32, v: f32) -> vec2<f32> {
    return vec2<f32>((u - 0.5) * info.aspect, v - 0.5);
}

fn effect(u: f32, v: f32) -> vec3<f32> {
    let t = info.t * p_speed() * 0.05;
    let ctr0 = p_center();
    let ctr = vec2<f32>(ctr0.x + sin(t) * 0.05, ctr0.y + cos(t * 0.7) * 0.02);
    let q = rel(u, v, ctr);
    let r2 = q.x * q.x + q.y * q.y;
    let size = 0.15 + p_softness() * 0.6;
    let light = exp(-r2 / (size * size)) * p_intensity();
    let base = p_color();
    // A soft floor gradient: slightly darker towards the bottom.
    let fl = 1.0 - 0.18 * sstep(0.4, 1.0, v);
    let lit = lerp3(base * fl, p_lightColor(), min(light, 1.0));
    let vc = centred(u, v);
    let vig = 1.0 - p_vignette() * sstep(0.3, 0.9, sqrt(vc.x * vc.x / (info.aspect * info.aspect) + vc.y * vc.y) * 1.4);
    let grain = (hash21(ti(u * info.w), ti(v * info.h), 91u) - 0.5) * 0.01;
    return lit * vig + vec3<f32>(grain);
}
