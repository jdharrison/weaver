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
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};
const MAX_INSTANCES: u32 = 200u;
@group(1) @binding(0)
var<uniform> instances: array<Instance, MAX_INSTANCES>;

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
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The Sun is at the origin; light radiates outward from (0, 0, 0).
    let to_light = normalize(-in.world_pos);
    let diff = max(dot(in.normal, to_light), 0.0);
    let ambient = 0.08;
    let lit = in.color.rgb * (ambient + diff);
    let glow = in.color.rgb * in.emissive;
    return vec4<f32>(lit + glow, in.color.a);
}
";

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
