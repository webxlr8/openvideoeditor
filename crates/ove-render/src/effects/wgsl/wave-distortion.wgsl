// distort::wave_distortion
fn effect(u: f32, v: f32) -> vec3<f32> {
    let a = rot((u - 0.5) * info.aspect, v - 0.5, -p_angle());
    let x = a.x;
    let y = a.y;
    let off = sin(y * p_frequency() * TAU * 3.0 + info.t * p_speed() * TAU) * p_strength() * 0.06;
    let b = rot(x + off, y, p_angle());
    return sample(b.x / info.aspect + 0.5, b.y + 0.5);
}
