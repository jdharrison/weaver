struct Params {
    inverse_current: mat4x4<f32>,
    previous: mat4x4<f32>,
    // width, height, exposure/dt, maximum blur length in pixels
    settings: vec4<f32>,
};
@group(0) @binding(0) var color: texture_2d<f32>;
@group(0) @binding(1) var depth: DEPTH_TYPE;
@group(0) @binding(2) var<uniform> params: Params;

fn depth_at(p: vec2<i32>) -> f32 {
    DEPTH_LOAD
}
// NDC depth is affine across a projected plane, including perspective views.
// Each one-sided derivative must agree with its next neighbor. A step or
// isolated thin surface is not a slope merely because its depth delta is large.
fn one_sided_slope(p: vec2<i32>, axis: vec2<i32>, d: f32) -> vec2<f32> {
    let end = p + 2 * axis;
    if any(end < vec2<i32>(0)) || any(end >= vec2<i32>(params.settings.xy)) {
        return vec2<f32>(0.0);
    }
    let near_depth = depth_at(p + axis);
    let far_depth = depth_at(end);
    if near_depth >= 1.0 || far_depth >= 1.0 { return vec2<f32>(0.0); }
    let first = near_depth - d;
    let second = far_depth - near_depth;
    let tolerance = max(0.00001, min(abs(first), abs(second)) * 0.05);
    if abs(first - second) > tolerance { return vec2<f32>(0.0); }
    return vec2<f32>(first, 1.0);
}
fn depth_slope(p: vec2<i32>, axis: vec2<i32>, d: f32) -> f32 {
    let forward = one_sided_slope(p, axis, d);
    let backward = one_sided_slope(p, -axis, d);
    if forward.y == 0.0 { return -backward.x; }
    if backward.y == 0.0 { return forward.x; }
    // Prefer the conservative side at creases, not the side spanning an edge.
    return select(-backward.x, forward.x, abs(forward.x) < abs(backward.x));
}
fn world_at(uv: vec2<f32>, d: f32) -> vec4<f32> {
    return params.inverse_current * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
}
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let center = textureLoad(color, p, 0);
    if params.settings.z <= 0.0 || params.settings.w <= 0.0 { return center; }
    let size = params.settings.xy;
    let uv = position.xy / size;
    let d = depth_at(p);
    // Clear depth has no reconstructed surface. No sky/transparent velocity is invented.
    if d >= 1.0 { return center; }
    let world = world_at(uv, d);
    let old = params.previous * world;
    if abs(world.w) < 0.000001 || old.w * world.w <= 0.000001 { return center; }
    let old_uv = vec2<f32>(old.x / old.w * 0.5 + 0.5, 0.5 - old.y / old.w * 0.5);
    var velocity = (old_uv - uv) * size * params.settings.z;
    let distance = length(velocity);
    // Also catches NaNs; bound extreme reprojections before normalization.
    if !(distance > 0.5 && distance < 1000000.0) { return center; }
    velocity *= min(distance, params.settings.w) / distance;
    let taps = i32(clamp(ceil(length(velocity)), 2.0, 16.0));
    var sum = center;
    var weight = 1.0;
    let center_world = world.xyz / world.w;
    // World-space pixel footprint gives a projection-independent edge threshold.
    let neighbor = world_at(uv + vec2<f32>(1.0 / size.x, 0.0), d);
    let footprint = length(neighbor.xyz / neighbor.w - center_world);
    let slope = vec2<f32>(depth_slope(p, vec2<i32>(1, 0), d), depth_slope(p, vec2<i32>(0, 1), d));
    for (var i = 1; i < taps; i += 1) {
        let q = vec2<i32>(floor(position.xy + velocity * (f32(i) / f32(taps - 1))));
        if any(q < vec2<i32>(0)) || any(q >= vec2<i32>(size)) { continue; }
        let sample_depth = depth_at(q);
        if sample_depth >= 1.0 { continue; }
        let sample_uv = (vec2<f32>(q) + 0.5) / size;
        let actual = world_at(sample_uv, sample_depth);
        let expected_depth = d + dot(slope, vec2<f32>(q - p));
        if expected_depth < 0.0 || expected_depth >= 1.0 { continue; }
        let expected = world_at(sample_uv, expected_depth);
        if abs(actual.w) < 0.000001 || abs(expected.w) < 0.000001 { continue; }
        if length(actual.xyz / actual.w - expected.xyz / expected.w) > max(footprint * 2.0, 0.0001) { continue; }
        // textureLoad decodes sRGB; the render target encodes exactly once.
        sum += textureLoad(color, q, 0);
        weight += 1.0;
    }
    return sum / weight;
}
