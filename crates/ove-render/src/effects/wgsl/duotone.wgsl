// stylize::duotone
fn effect(u: f32, v: f32) -> vec3<f32> {
    let l = luma(sample(u, v));
    let t = sstep(0.0, 1.0, clamp(l + 0.5 - p_balance(), 0.0, 1.0));
    return lerp3(p_shadows(), p_highlights(), t);
}
