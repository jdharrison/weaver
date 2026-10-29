//! Bounded embedded GLB skeletal animation: immutable geometry, CPU local-TRS
//! sampling, and GPU skinning. Coordinates remain glTF actor-local (Y-up).
//! LINEAR and STEP clips are supported; cubic splines and morphs are explicit errors.
//! Materials/textures use the static loader's opaque base-color, normal and
//! metallic/roughness subset and limits.

use crate::{
    AppAssets,
    glb::{
        GlbError, Loader, invalid, limit, node_matrix, validate_root_mode, validate_storage_mode,
    },
};
use glam::{Mat4, Quat, Vec3, Vec4};
use gltf::{
    accessor::{DataType, Dimensions},
    animation::{Interpolation, Property, util::ReadOutputs},
    mesh::Semantic,
};
use std::{collections::HashSet, sync::Arc};
use weaver_render::{MeshInstance, mesh::MAX_SKIN_JOINTS};
use weaver_render_wgpu::SkinVertex;

#[cfg(test)]
#[path = "animated_glb_tests.rs"]
mod tests;

#[path = "cooked_animation.rs"]
pub(crate) mod cooked_animation;

/// Loader and sampling failures share the static GLB error vocabulary.
pub type AnimatedGlbError = GlbError;

#[derive(Clone, Copy, Debug)]
struct Trs {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}
impl Trs {
    fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
    fn blend(self, other: Self, weight: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, weight),
            rotation: shortest(self.rotation, other.rotation, weight),
            scale: self.scale.lerp(other.scale, weight),
        }
    }
}

#[derive(Debug)]
struct Node {
    local: Trs,
    matrix: Option<Mat4>,
    parent: Option<usize>,
}
#[derive(Debug)]
struct Skin {
    joints: Vec<usize>,
    inverse_bind: Vec<Mat4>,
}
#[derive(Debug)]
struct Draw {
    instance: MeshInstance,
    node: usize,
    skin: Option<usize>,
}
#[derive(Debug)]
struct Channel {
    node: usize,
    property: Property,
    step: bool,
    times: Vec<f32>,
    values: Vec<Vec4>,
}

/// Immutable clip metadata and validated local-transform tracks.
#[derive(Debug)]
pub struct AnimationClip {
    /// Authored name, or an empty string for unnamed clips.
    pub name: String,
    /// Last key time in seconds (zero-length clips are valid).
    pub duration: f32,
    channels: Vec<Channel>,
}

/// Geometry/textures may be drained into app startup without invalidating sampling.
/// One model can be sampled independently for any number of actors. Samples own
/// palettes; changing another actor's time never modifies a previous sample.
#[derive(Debug)]
pub struct AnimatedModel {
    /// Upload once using the platform startup path.
    pub assets: AppAssets,
    /// Clips in document order; names are not interpreted as gameplay states.
    pub clips: Vec<AnimationClip>,
    /// Material/texture fallback diagnostics from the shared importer.
    pub warnings: Vec<String>,
    nodes: Vec<Node>,
    order: Vec<usize>,
    skins: Vec<Skin>,
    draws: Vec<Draw>,
}

/// A local clip/time selection for blending.
#[derive(Clone, Copy, Debug)]
pub struct AnimationSample {
    /// Clip index in `AnimatedModel::clips`.
    pub clip: usize,
    /// Finite seconds; negative looping time wraps, nonlooping time clamps to zero.
    pub seconds: f32,
    /// Wrap at duration instead of holding the final key.
    pub looping: bool,
}

fn shortest(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    a.slerp(b, t).normalize()
}
fn checked_matrix(matrix: Mat4) -> Result<Mat4, GlbError> {
    if !matrix.is_finite()
        || !matrix.inverse().is_finite()
        || matrix.x_axis.w != 0.0
        || matrix.y_axis.w != 0.0
        || matrix.z_axis.w != 0.0
        || matrix.w_axis.w != 1.0
        || matrix.to_cols_array().iter().any(|v| v.abs() > 1e12)
    {
        return Err(invalid(
            "transform must be bounded, finite, affine and invertible",
        ));
    }
    Ok(matrix)
}

impl AnimatedModel {
    /// Sample a clip into actor-local draws. Apply the same actor `RenderTransform`
    /// to each result. Nonlooping playback clamps exactly to the final key.
    pub fn sample(
        &self,
        clip: usize,
        seconds: f32,
        looping: bool,
    ) -> Result<Vec<MeshInstance>, GlbError> {
        self.instances(&self.local_sample(AnimationSample {
            clip,
            seconds,
            looping,
        })?)
    }

    /// Blend two samples in local TRS before hierarchy evaluation and inverse binds.
    /// `weight` must be finite in [0,1]; 0 selects `a`, 1 selects `b`.
    pub fn sample_blended(
        &self,
        a: AnimationSample,
        b: AnimationSample,
        weight: f32,
    ) -> Result<Vec<MeshInstance>, GlbError> {
        if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
            return Err(invalid("blend weight must be in [0,1]"));
        }
        let a = self.local_sample(a)?;
        let b = self.local_sample(b)?;
        self.instances(
            &a.into_iter()
                .zip(b)
                .map(|(a, b)| a.blend(b, weight))
                .collect::<Vec<_>>(),
        )
    }

    /// Sample authored default transforms without requiring any animation clip.
    pub fn sample_rest(&self) -> Result<Vec<MeshInstance>, GlbError> {
        self.instances(&self.nodes.iter().map(|n| n.local).collect::<Vec<_>>())
    }

    fn local_sample(&self, sample: AnimationSample) -> Result<Vec<Trs>, GlbError> {
        if !sample.seconds.is_finite() {
            return Err(invalid("sample time must be finite"));
        }
        let clip = self
            .clips
            .get(sample.clip)
            .ok_or_else(|| invalid("clip index out of range"))?;
        if !clip.duration.is_finite() || clip.duration < 0.0 {
            return Err(invalid("clip duration must be finite and nonnegative"));
        }
        let time = if sample.looping && clip.duration > 0.0 {
            sample.seconds.rem_euclid(clip.duration)
        } else {
            sample.seconds.clamp(0.0, clip.duration)
        };
        let mut locals: Vec<_> = self.nodes.iter().map(|node| node.local).collect();
        for channel in &clip.channels {
            let end = channel.times.partition_point(|key| *key <= time);
            let lo = end.saturating_sub(1);
            let hi = end.min(channel.times.len() - 1);
            let t = if channel.step || lo == hi || time <= channel.times[0] {
                0.0
            } else {
                (time - channel.times[lo]) / (channel.times[hi] - channel.times[lo])
            };
            let a = channel.values[lo];
            let b = channel.values[hi];
            let local = &mut locals[channel.node];
            match channel.property {
                Property::Translation => local.translation = a.truncate().lerp(b.truncate(), t),
                Property::Scale => local.scale = a.truncate().lerp(b.truncate(), t),
                Property::Rotation => {
                    local.rotation = shortest(
                        Quat::from_array(a.to_array()),
                        Quat::from_array(b.to_array()),
                        t,
                    );
                }
                Property::MorphTargetWeights => unreachable!(),
            }
        }
        Ok(locals)
    }

    fn instances(&self, locals: &[Trs]) -> Result<Vec<MeshInstance>, GlbError> {
        let mut world = vec![Mat4::IDENTITY; self.nodes.len()];
        for &index in &self.order {
            let node = &self.nodes[index];
            let local = node.matrix.unwrap_or_else(|| locals[index].matrix());
            world[index] =
                checked_matrix(node.parent.map_or(local, |parent| world[parent] * local))?;
        }
        let palettes: Vec<Arc<[Mat4]>> = self
            .skins
            .iter()
            .map(|skin| {
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(&joint, bind)| checked_matrix(world[joint] * *bind))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Arc::from)
            })
            .collect::<Result<_, _>>()?;
        self.draws
            .iter()
            .map(|draw| {
                let mut instance = draw.instance.clone();
                // glTF skinned positions use jointWorld * inverseBind. The mesh node's
                // world transform cancels against inverse(meshWorld); do not bake it.
                instance.skin = Some(draw.skin.map_or_else(
                    || Arc::from([world[draw.node]]),
                    |skin| palettes[skin].clone(),
                ));
                Ok(instance)
            })
            .collect()
    }
}

/// Load a self-contained standard GLB without filesystem, network, or GPU access.
/// Limits include 128 joints per skin, 4096 nodes, 256 clips, 1M total keys,
/// 128 MiB input, and the static importer's geometry/texture processing budgets.
/// Unsupported sparse, compressed, morph, cubic-spline and external data fail explicitly.
pub fn load_animated_glb(bytes: &[u8]) -> Result<AnimatedModel, GlbError> {
    limit(bytes.len(), 128 * 1024 * 1024, "input bytes")?;
    if bytes.len() < 12
        || !bytes.len().is_multiple_of(4)
        || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize != bytes.len()
    {
        return Err(invalid("invalid GLB header length"));
    }
    let glb = gltf::binary::Glb::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    limit(glb.json.len(), 8 * 1024 * 1024, "JSON bytes")?;
    if 20 + glb.json.len() + glb.bin.as_ref().map_or(0, |b| 8 + b.len()) != bytes.len()
        || !glb.json.len().is_multiple_of(4)
        || glb.bin.as_ref().is_some_and(|b| !b.len().is_multiple_of(4))
    {
        return Err(invalid("unaligned or extra GLB chunks"));
    }
    let root: gltf::json::Root =
        gltf::json::deserialize::from_slice(&glb.json).map_err(|e| invalid(e.to_string()))?;
    validate_root_mode(&root, true)?;
    limit(root.animations.len(), 256, "clips")?;
    limit(root.skins.len(), 256, "skins")?;
    limit(
        root.animations
            .iter()
            .map(|a| a.channels.len() + a.samplers.len())
            .sum(),
        16_384,
        "animation records",
    )?;
    // gltf-json 1.4.1 does not validate channel targets through Animation::validate;
    // the typed target accessors would otherwise unwrap these unchecked values.
    for animation in &root.animations {
        for channel in &animation.channels {
            if channel.target.node.value() >= root.nodes.len() {
                return Err(invalid("animation target node index out of range"));
            }
            if !matches!(
                channel.target.path,
                gltf::json::validation::Checked::Valid(_)
            ) {
                return Err(invalid("invalid animation target path"));
            }
        }
    }
    let document = gltf::Document::from_json(root).map_err(|e| invalid(e.to_string()))?;
    let data = validate_storage_mode(
        &document,
        glb.bin.as_deref().ok_or_else(|| invalid("missing BIN"))?,
        true,
    )?;
    let mut nodes = Vec::new();
    for node in document.nodes() {
        let matrix = checked_matrix(node_matrix(&node)?.as_mat4())?;
        let (translation, rotation, scale) = node.transform().decomposed();
        nodes.push(Node {
            local: Trs {
                translation: Vec3::from_array(translation),
                rotation: Quat::from_array(rotation),
                scale: Vec3::from_array(scale),
            },
            matrix: matches!(node.transform(), gltf::scene::Transform::Matrix { .. })
                .then_some(matrix),
            parent: None,
        });
    }
    for node in document.nodes() {
        for child in node.children() {
            if nodes[child.index()].parent.replace(node.index()).is_some() {
                return Err(invalid("multiple node parents"));
            }
        }
    }
    let mut order = Vec::new();
    let mut stack: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent.is_none())
        .map(|(i, _)| (i, 1))
        .collect();
    let source_nodes: Vec<_> = document.nodes().collect();
    while let Some((index, depth)) = stack.pop() {
        limit(depth, 128, "hierarchy depth")?;
        order.push(index);
        stack.extend(
            source_nodes[index]
                .children()
                .map(|n| (n.index(), depth + 1)),
        );
    }
    if order.len() != nodes.len() {
        return Err(invalid("cyclic hierarchy"));
    }
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or_else(|| invalid("no scene"))?;
    let mut selected = vec![false; nodes.len()];
    let mut stack: Vec<_> = scene.nodes().map(|n| n.index()).collect();
    for &root in &stack {
        if nodes[root].parent.is_some() {
            return Err(invalid("scene root has a parent"));
        }
    }
    while let Some(index) = stack.pop() {
        if std::mem::replace(&mut selected[index], true) {
            return Err(invalid("repeated scene node"));
        }
        stack.extend(source_nodes[index].children().map(|n| n.index()));
    }
    let mut skins = Vec::new();
    for skin in document.skins() {
        let joints: Vec<_> = skin.joints().map(|j| j.index()).collect();
        limit(joints.len(), MAX_SKIN_JOINTS, "skin joints")?;
        if joints.is_empty() || joints.iter().copied().collect::<HashSet<_>>().len() != joints.len()
        {
            return Err(invalid("empty or duplicate skin joints"));
        }
        let inverse_bind = if let Some(accessor) = skin.inverse_bind_matrices() {
            if accessor.dimensions() != Dimensions::Mat4
                || accessor.data_type() != DataType::F32
                || accessor.normalized()
                || accessor.count() != joints.len()
            {
                return Err(invalid("invalid inverse bind accessor"));
            }
            skin.reader(|_| Some(data))
                .read_inverse_bind_matrices()
                .ok_or_else(|| invalid("unreadable inverse binds"))?
                .map(|m| checked_matrix(Mat4::from_cols_array_2d(&m)))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![Mat4::IDENTITY; joints.len()]
        };
        skins.push(Skin {
            joints,
            inverse_bind,
        });
    }
    let mut clips = Vec::new();
    let mut keys = 0;
    for animation in document.animations() {
        let mut clip = AnimationClip {
            name: animation.name().unwrap_or_default().into(),
            duration: 0.0,
            channels: Vec::new(),
        };
        let mut targets = HashSet::new();
        for channel in animation.channels() {
            let target = channel.target();
            let node = target.node().index();
            let property = target.property();
            if property == Property::MorphTargetWeights || nodes[node].matrix.is_some() {
                return Err(GlbError::Unsupported(
                    "morph animation or animation targeting a matrix node".into(),
                ));
            }
            if !targets.insert((node, property as u8)) {
                return Err(invalid("duplicate animation target"));
            }
            let sampler = channel.sampler();
            if sampler.interpolation() == Interpolation::CubicSpline {
                return Err(GlbError::Unsupported(
                    "CUBICSPLINE interpolation; export LINEAR or STEP".into(),
                ));
            }
            let input = sampler.input();
            let output = sampler.output();
            keys += input.count();
            limit(keys, 1_000_000, "total animation keys")?;
            if input.data_type() != DataType::F32
                || input.dimensions() != Dimensions::Scalar
                || input.normalized()
                || output.data_type() != DataType::F32
                || output.normalized()
                || output.count() != input.count()
                || output.dimensions()
                    != if property == Property::Rotation {
                        Dimensions::Vec4
                    } else {
                        Dimensions::Vec3
                    }
            {
                return Err(invalid("invalid animation accessor type/count"));
            }
            let reader = channel.reader(|_| Some(data));
            let times: Vec<_> = reader
                .read_inputs()
                .ok_or_else(|| invalid("unreadable key times"))?
                .collect();
            if times.is_empty()
                || times.iter().any(|t| !t.is_finite() || *t < 0.0 || *t > 1e7)
                || times.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(invalid(
                    "key times must be finite, bounded, nonnegative and strictly increasing",
                ));
            }
            let values: Vec<Vec4> = match reader
                .read_outputs()
                .ok_or_else(|| invalid("unreadable key values"))?
            {
                ReadOutputs::Translations(v) | ReadOutputs::Scales(v) => {
                    v.map(|v| Vec3::from_array(v).extend(0.0)).collect()
                }
                ReadOutputs::Rotations(v) => v.into_f32().map(Vec4::from_array).collect(),
                _ => return Err(GlbError::Unsupported("morph animation".into())),
            };
            for value in &values {
                if !value.is_finite() || value.abs().max_element() > 1e12 {
                    return Err(invalid("nonfinite/unbounded animation value"));
                }
                if property == Property::Rotation && (value.length_squared() - 1.0).abs() > 0.001 {
                    return Err(invalid("animation quaternion is not unit length"));
                }
                if property == Property::Scale && value.truncate().abs().min_element() < 1e-8 {
                    return Err(invalid("zero animation scale"));
                }
            }
            let values = if property == Property::Rotation {
                values.into_iter().map(Vec4::normalize).collect()
            } else {
                values
            };
            clip.duration = clip.duration.max(*times.last().unwrap());
            clip.channels.push(Channel {
                node,
                property,
                step: sampler.interpolation() == Interpolation::Step,
                times,
                values,
            });
        }
        clips.push(clip);
    }
    let mut loader = Loader::animated(data, document.images().len());
    let mut draws = Vec::new();
    for node in document.nodes().filter(|node| selected[node.index()]) {
        if let Some(mesh) = node.mesh() {
            let skin = node.skin().map(|s| s.index());
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles
                    || primitive.material().alpha_mode() != gltf::material::AlphaMode::Opaque
                {
                    return Err(GlbError::Unsupported(
                        "non-triangle or non-opaque primitive".into(),
                    ));
                }
                let influences =
                    read_influences(&primitive, data, skin.map(|i| skins[i].joints.len()))?;
                let start = loader.model.instances.len();
                loader.animated_primitive(&primitive, Some(&influences))?;
                draws.extend(
                    loader.model.instances[start..]
                        .iter()
                        .cloned()
                        .map(|instance| Draw {
                            instance,
                            node: node.index(),
                            skin,
                        }),
                );
            }
        }
    }
    if draws.is_empty() {
        return Err(invalid("scene has no triangle meshes"));
    }
    let model = AnimatedModel {
        assets: loader.model.assets,
        warnings: loader.model.warnings,
        clips,
        nodes,
        order,
        skins,
        draws,
    };
    model.sample_rest()?;
    Ok(model)
}

fn read_influences(
    primitive: &gltf::Primitive<'_>,
    data: &[u8],
    joints: Option<usize>,
) -> Result<Vec<SkinVertex>, GlbError> {
    let (count, _) = crate::glb::validate_primitive(primitive)?;
    for position in primitive
        .reader(|_| Some(data))
        .read_positions()
        .ok_or_else(|| invalid("unreadable POSITION"))?
    {
        if position.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            return Err(invalid("nonfinite/unbounded bind-space position"));
        }
    }
    limit(count, 1_000_000, "skin vertices")?;
    let mut result = vec![
        SkinVertex {
            joints: [0; 8],
            weights: [0.0; 8]
        };
        count
    ];
    for (semantic, accessor) in primitive.attributes() {
        match semantic {
            Semantic::Joints(set) | Semantic::Weights(set) => {
                if set > 1 {
                    return Err(GlbError::Unsupported("more than eight influences".into()));
                }
                let valid_type = match semantic {
                    Semantic::Joints(_) => {
                        matches!(accessor.data_type(), DataType::U8 | DataType::U16)
                            && !accessor.normalized()
                    }
                    _ => {
                        (accessor.data_type() == DataType::F32 && !accessor.normalized())
                            || (matches!(accessor.data_type(), DataType::U8 | DataType::U16)
                                && accessor.normalized())
                    }
                };
                if joints.is_none()
                    || !valid_type
                    || accessor.dimensions() != Dimensions::Vec4
                    || accessor.count() != count
                {
                    return Err(invalid(
                        "invalid skin attribute type/count or missing node skin",
                    ));
                }
            }
            _ => {}
        }
    }
    let Some(joint_count) = joints else {
        for v in &mut result {
            v.weights[0] = 1.0;
        }
        return Ok(result);
    };
    let reader = primitive.reader(|_| Some(data));
    for set in 0..=1 {
        let js = reader.read_joints(set);
        let ws = reader.read_weights(set);
        match (js, ws) {
            (Some(js), Some(ws)) => {
                for (vertex, (js, ws)) in result.iter_mut().zip(js.into_u16().zip(ws.into_f32())) {
                    for i in 0..4 {
                        if usize::from(js[i]) >= joint_count
                            || !ws[i].is_finite()
                            || ws[i] < 0.0
                            || ws[i] > 1.0
                        {
                            return Err(invalid("invalid bone index or weight"));
                        }
                        vertex.joints[set as usize * 4 + i] = js[i];
                        vertex.weights[set as usize * 4 + i] = ws[i];
                    }
                }
            }
            (None, None) if set == 1 => {}
            _ => {
                return Err(invalid(
                    "paired JOINTS_0/WEIGHTS_0 required; set 1 must also be paired",
                ));
            }
        }
    }
    for vertex in &mut result {
        let sum: f32 = vertex.weights.iter().sum();
        if !sum.is_finite() || sum < 1e-8 {
            return Err(invalid("zero skin weight sum"));
        }
        for weight in &mut vertex.weights {
            *weight /= sum;
        }
    }
    Ok(result)
}
