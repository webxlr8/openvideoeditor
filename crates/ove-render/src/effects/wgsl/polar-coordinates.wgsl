// distort::polar_coordinates
fn effect(u: f32, v: f32) -> vec3<f32> {
    let ctr = p_center();
    // The x of the output is the angle and y the radius: the source is unrolled around the centre.
    let a = (u - 0.5) * TAU * p_wrap();
    let r = v * p_radius() * 0.5;
    let pp = unrel(r * sin(a), -r * cos(a), ctr);
    let k = p_intensity();
    return sample(u + (pp.x - u) * k, v + (pp.y - v) * k);
}
