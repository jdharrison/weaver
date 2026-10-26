//! Shared shader code and pipeline helpers.

/// Common WGSL header used by all pipelines.
pub const COMMON_HEADER: &str = r"
struct CameraUniform {
    view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct Instance {
    model_0: vec4<f32>,
    model_1: vec4<f32>,
    model_2: vec4<f32>,
    model_3: vec4<f32>,
    color: vec4<f32>,
};
";

/// WGSL mesh shader.
pub const MESH_SHADER: &str = r"
struct CameraUniform {
    view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct Instance {
    model_0: vec4<f32>,
    model_1: vec4<f32>,
    model_2: vec4<f32>,
    model_3: vec4<f32>,
    color: vec4<f32>,
    emissive: f32,
    ambient: f32,
    intensity: f32,
    lighting_enabled: f32,
    light_position: vec4<f32>,
    texture_fit: vec4<f32>,
};
const MAX_INSTANCES: u32 = 128u;
@group(1) @binding(0)
var<uniform> instances: array<Instance, MAX_INSTANCES>;

@group(2) @binding(0)
var mesh_texture: texture_2d<f32>;
@group(2) @binding(1)
var mesh_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) emissive: f32,
    @location(5) light_position: vec3<f32>,
    @location(6) lighting: vec3<f32>,
    @location(7) @interpolate(flat) texture_fit: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let inst = instances[instance_index];
    let model = mat4x4<f32>(inst.model_0, inst.model_1, inst.model_2, inst.model_3);
    let world_pos = model * vec4<f32>(input.position, 1.0);
    var out: VertexOutput;
    out.clip_position = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    out.color = input.color * inst.color;
    out.uv = input.uv;
    out.emissive = inst.emissive;
    out.light_position = inst.light_position.xyz;
    out.lighting = vec3<f32>(inst.ambient, inst.intensity, inst.lighting_enabled);
    out.texture_fit = inst.texture_fit.xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Preserve origin lighting for instances without an independent point light.
    var light_direction = -in.world_pos;
    var ambient = 0.08;
    var intensity = 1.0;
    if (in.lighting.z > 0.5) {
        light_direction = in.light_position - in.world_pos;
        ambient = in.lighting.x;
        intensity = in.lighting.y;
    }
    var to_light = vec3<f32>(0.0);
    if (dot(light_direction, light_direction) > 0.0) {
        to_light = normalize(light_direction);
    }
    let diff = max(dot(normalize(in.normal), to_light), 0.0);
    var image_uv = in.uv;
    if (in.texture_fit.z > 0.5) {
        image_uv = (in.uv - vec2<f32>(0.5)) / in.texture_fit.xy + vec2<f32>(0.5);
        // This draw binds ClampToEdge for mip and anisotropic filtering footprints.
    }
    // Sample before the per-fragment bar branch to keep implicit derivatives valid.
    let color = textureSample(mesh_texture, mesh_sampler, image_uv) * in.color;
    if (in.texture_fit.z > 0.5 &&
        (any(image_uv < vec2<f32>(0.0)) || any(image_uv > vec2<f32>(1.0)))) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let lit = color.rgb * (ambient + diff * intensity);
    let glow = color.rgb * in.emissive;
    return vec4<f32>(lit + glow, color.a);
}
";

/// WGSL stylized water shader.
pub const WATER_SHADER: &str = r"
struct CameraUniform {
    view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct Instance {
    model_0: vec4<f32>,
    model_1: vec4<f32>,
    model_2: vec4<f32>,
    model_3: vec4<f32>,
    color: vec4<f32>,
    emissive: f32,
    ambient: f32,
    intensity: f32,
    lighting_enabled: f32,
    light_position: vec4<f32>,
    texture_fit: vec4<f32>,
};
const MAX_INSTANCES: u32 = 128u;
@group(1) @binding(0)
var<uniform> instances: array<Instance, MAX_INSTANCES>;

@group(2) @binding(0)
var mesh_texture: texture_2d<f32>;
@group(2) @binding(1)
var mesh_sampler: sampler;

struct WaterUniform {
    deep_color: vec4<f32>,
    shallow_color: vec4<f32>,
    horizon: vec4<f32>,
    zenith: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    // time, specular strength, wave scale, wave height
    params: vec4<f32>,
    // foam strength, detail strength, normal strength, reserved
    params2: vec4<f32>,
};
@group(3) @binding(0)
var<uniform> water: WaterUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) wave: f32,
    @location(3) mask: f32,
};

fn wave_gradient(position: vec2<f32>, time: f32, scale: f32) -> vec2<f32> {
    var gradient = vec2<f32>(0.0);
    gradient += cos(dot(position, vec2<f32>(0.11, 0.07)) * scale + time * 1.05)
        * 0.50 * vec2<f32>(0.11, 0.07) * scale;
    gradient += cos(dot(position, vec2<f32>(-0.05, 0.13)) * scale + time * 1.45 + 1.7)
        * 0.27 * vec2<f32>(-0.05, 0.13) * scale;
    gradient += cos(dot(position, vec2<f32>(0.19, -0.11)) * scale + time * 1.85 + 3.1)
        * 0.14 * vec2<f32>(0.19, -0.11) * scale;
    gradient += cos(dot(position, vec2<f32>(0.31, 0.23)) * scale + time * 2.35 + 5.2)
        * 0.06 * vec2<f32>(0.31, 0.23) * scale;
    return gradient;
}

fn wave_height(position: vec2<f32>, time: f32, scale: f32) -> f32 {
    var height = 0.0;
    height += sin(dot(position, vec2<f32>(0.11, 0.07)) * scale + time * 1.05) * 0.50;
    height += sin(dot(position, vec2<f32>(-0.05, 0.13)) * scale + time * 1.45 + 1.7) * 0.27;
    height += sin(dot(position, vec2<f32>(0.19, -0.11)) * scale + time * 1.85 + 3.1) * 0.14;
    height += sin(dot(position, vec2<f32>(0.31, 0.23)) * scale + time * 2.35 + 5.2) * 0.06;
    return height;
}

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let inst = instances[instance_index];
    let model = mat4x4<f32>(inst.model_0, inst.model_1, inst.model_2, inst.model_3);
    var world_pos = model * vec4<f32>(input.position, 1.0);
    let mask = clamp(input.color.r, 0.0, 1.0);
    let wave = wave_height(world_pos.xz, water.params.x, water.params.z) * mask;
    world_pos.y += wave * water.params.w;
    var out: VertexOutput;
    out.clip_position = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.uv = input.uv;
    out.wave = wave;
    out.mask = mask;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let gradient = wave_gradient(in.world_pos.xz, water.params.x, water.params.z);
    let amplitude = water.params.w * in.mask * water.params2.z;
    let normal = normalize(vec3<f32>(-gradient.x * amplitude, 1.0, -gradient.y * amplitude));

    let scroll_a = vec2<f32>(water.params.x * 0.010, water.params.x * 0.014);
    let scroll_b = vec2<f32>(water.params.x * -0.013, water.params.x * 0.008);
    let detail_a = textureSample(mesh_texture, mesh_sampler, in.uv + scroll_a).rgb;
    let detail_b = textureSample(mesh_texture, mesh_sampler, in.uv * 1.7 + scroll_b).rgb;
    let detail = max(detail_a, detail_b);
    let detail_luma = clamp(dot(detail, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);

    let view_dir = normalize(camera.eye - in.world_pos);
    let fresnel = pow(clamp(1.0 - dot(normal, view_dir), 0.0, 1.0), 5.0);
    let reflected = reflect(-view_dir, normal);
    let sky = mix(water.horizon.rgb, water.zenith.rgb, pow(clamp(reflected.y, 0.0, 1.0), 0.5));

    let depth_mix = clamp(0.42 + in.wave * 0.30 + detail_luma * 0.12, 0.0, 1.0);
    var color = mix(water.deep_color.rgb, water.shallow_color.rgb, depth_mix);
    color = mix(color, sky, clamp(fresnel * 0.90 + 0.05, 0.0, 1.0));

    let half_dir = normalize(water.sun_direction.xyz + view_dir);
    let ndoth = clamp(dot(normal, half_dir), 0.0, 1.0);
    let glint = pow(ndoth, 220.0) * water.params.y * (0.45 + 1.1 * detail_luma);
    let sheen = pow(ndoth, 24.0) * 0.06 * water.params.y;
    color += water.sun_color.rgb * (glint + sheen);

    let foam = smoothstep(0.42, 0.88, in.wave) * water.params2.x * (0.25 + 0.75 * detail_luma);
    color = mix(color, vec3<f32>(0.86, 0.94, 1.0), clamp(foam, 0.0, 1.0));
    color += detail_luma * water.params2.y * 0.10;
    return vec4<f32>(color, 1.0);
}
";

/// Skinned variant shares fragment/material behavior with rigid meshes.
/// Inverse-transpose of the blended linear transform supports nonuniform scale.
pub fn skinned_mesh_shader() -> String {
    MESH_SHADER.replace(
        "struct VertexInput {",
        "@group(3) @binding(0) var<uniform> joints: array<mat4x4<f32>, 128>;\nstruct VertexInput {\n    @location(4) joints0: vec4<u32>,\n    @location(5) joints1: vec4<u32>,\n    @location(6) weights0: vec4<f32>,\n    @location(7) weights1: vec4<f32>,"
    ).replace(
        "let model = mat4x4<f32>(inst.model_0, inst.model_1, inst.model_2, inst.model_3);",
        "var skin = joints[input.joints0.x] * input.weights0.x;\n    for (var i = 1u; i < 4u; i += 1u) { skin += joints[input.joints0[i]] * input.weights0[i]; }\n    for (var i = 0u; i < 4u; i += 1u) { skin += joints[input.joints1[i]] * input.weights1[i]; }\n    let model = mat4x4<f32>(inst.model_0, inst.model_1, inst.model_2, inst.model_3) * skin;"
    ).replace(
        "out.normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);",
        "let a = model[0].xyz; let b = model[1].xyz; let c = model[2].xyz;\n    let determinant = dot(a, cross(b, c));\n    let cofactor = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));\n    var normal = cofactor * input.normal;\n    if (determinant < 0.0) { normal = -normal; }\n    if (dot(normal, normal) < 1e-20) { normal = input.normal; }\n    out.normal = normalize(normal);"
    )
}

/// WGSL sprite shader.
pub const SPRITE_SHADER: &str = r"
struct CameraUniform {
    view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct Instance {
    model_0: vec4<f32>,
    model_1: vec4<f32>,
    model_2: vec4<f32>,
    model_3: vec4<f32>,
    uv_rect: vec4<f32>,
    tint: vec4<f32>,
};
const MAX_INSTANCES: u32 = 160u;
@group(1) @binding(0)
var<uniform> instances: array<Instance, MAX_INSTANCES>;

@group(2) @binding(0)
var sprite_texture: texture_2d<f32>;
@group(2) @binding(1)
var sprite_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

const QUAD: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(-0.5, -0.5),
    vec2<f32>( 0.5, -0.5),
    vec2<f32>(-0.5,  0.5),
    vec2<f32>( 0.5,  0.5),
);

const UVS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
);

@vertex
fn vs_main(@builtin(instance_index) instance_index: u32, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let inst = instances[instance_index];
    let model = mat4x4<f32>(inst.model_0, inst.model_1, inst.model_2, inst.model_3);
    let pos = vec4<f32>(QUAD[vertex_index], 0.0, 1.0);
    let world = model * pos;
    var out: VertexOutput;
    out.clip_position = camera.view_proj * world;
    let raw_uv = UVS[vertex_index];
    out.uv = vec2<f32>(
        inst.uv_rect.x + raw_uv.x * inst.uv_rect.z,
        inst.uv_rect.y + raw_uv.y * inst.uv_rect.w
    );
    out.tint = inst.tint;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(sprite_texture, sprite_sampler, in.uv) * in.tint;
}
";

/// WGSL particle shader.
pub const PARTICLE_SHADER: &str = r"
struct CameraUniform {
    view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct Particle {
    position: vec3<f32>,
    size: f32,
    color: vec4<f32>,
};
const MAX_PARTICLES: u32 = 500u;
@group(1) @binding(0)
var<uniform> particles: array<Particle, MAX_PARTICLES>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

const QUAD: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(-0.5, -0.5),
    vec2<f32>( 0.5, -0.5),
    vec2<f32>(-0.5,  0.5),
    vec2<f32>( 0.5,  0.5),
);

const UVS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
);

@vertex
fn vs_main(@builtin(instance_index) instance_index: u32, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let p = particles[instance_index];
    let offset = QUAD[vertex_index] * p.size;
    // Billboard: align to screen, approximately by using view-aligned basis.
    let view_right = vec3<f32>(camera.view_proj[0][0], camera.view_proj[1][0], camera.view_proj[2][0]);
    let view_up = vec3<f32>(camera.view_proj[0][1], camera.view_proj[1][1], camera.view_proj[2][1]);
    let world = p.position + view_right * offset.x + view_up * offset.y;
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = p.color;
    out.uv = UVS[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = length(in.uv - vec2<f32>(0.5, 0.5));
    let alpha = smoothstep(0.5, 0.0, d);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
";

/// WGSL UI shader.
pub const UI_SHADER: &str = r"
struct UiUniform {
    screen_size: vec2<f32>,
    // Match the 16-byte Rust buffer layout required by downlevel WebGL2 devices.
    _pad: vec2<f32>,
};
@group(0) @binding(0)
var<uniform> ui_uniform: UiUniform;

struct Rect {
    position: vec2<f32>,
    size: vec2<f32>,
    background: vec4<f32>,
    border_color: vec4<f32>,
    border_width: f32,
    corner_radius: f32,
    layer: f32,
};
const MAX_RECTS: u32 = 250u;
@group(1) @binding(0)
var<uniform> rects: array<Rect, MAX_RECTS>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) border: vec4<f32>,
    @location(2) local: vec2<f32>,
    @location(3) size: vec2<f32>,
    @location(4) border_width: f32,
    @location(5) corner_radius: f32,
};

const QUAD: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(@builtin(instance_index) instance_index: u32, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let r = rects[instance_index];
    let q = QUAD[vertex_index];
    let pixel = r.position + q * r.size;
    let ndc = vec2<f32>(pixel.x / ui_uniform.screen_size.x, 1.0 - pixel.y / ui_uniform.screen_size.y) * 2.0 - 1.0;
    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = r.background;
    out.border = r.border_color;
    out.local = q * r.size;
    out.size = r.size;
    out.border_width = r.border_width;
    out.corner_radius = r.corner_radius;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let center = in.size * 0.5;
    let p = in.local - center;
    let b = in.size * 0.5 - vec2<f32>(in.border_width, in.border_width);
    let d_box = sd_rounded_box(p, b, in.corner_radius);
    let d_outer = sd_rounded_box(p, in.size * 0.5, in.corner_radius);
    if (d_outer > 0.0) {
        discard;
    }
    if (d_box < 0.0) {
        return in.color;
    }
    return in.border;
}
";

/// Build a default pipeline layout with a camera uniform at group 0.
pub fn camera_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("camera-bind-group-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Bind group layout for a uniform buffer of instances.
pub fn instance_uniform_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("instance-uniform-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Build a shader module from a WGSL source string.
pub fn create_shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}
