//! Cooked animation schema; a child of `animated_glb` to keep graph internals private.
use super::{
    AnimatedModel, AnimationClip, Arc, Channel, Draw, MAX_SKIN_JOINTS, Mat4, MeshInstance, Node,
    Property, Quat, Skin, Trs, Vec3, Vec4, checked_matrix,
};
use crate::cooked::{
    RECORDS, Reader, References, Writer, read_assets, read_instance, read_strings, require,
    write_assets, write_strings,
};
use std::mem::size_of;

type Result<T> = std::result::Result<T, String>;

pub(crate) fn encode(w: &mut Writer, model: &AnimatedModel) -> Result<()> {
    write_assets(w, &model.assets)?;
    write_strings(w, &model.warnings);
    w.count(model.nodes.len());
    for node in &model.nodes {
        w.array(&node.local.translation.to_array());
        w.array(&node.local.rotation.to_array());
        w.array(&node.local.scale.to_array());
        w.boolean(node.matrix.is_some());
        if let Some(m) = node.matrix {
            w.matrix(m);
        }
        w.optional_index(node.parent);
    }
    w.count(model.order.len());
    for &i in &model.order {
        w.count(i);
    }
    w.count(model.skins.len());
    for skin in &model.skins {
        require(
            skin.joints.len() == skin.inverse_bind.len(),
            "inverse bind count",
        )?;
        w.count(skin.joints.len());
        for (&joint, &bind) in skin.joints.iter().zip(&skin.inverse_bind) {
            w.count(joint);
            w.matrix(bind);
        }
    }
    w.count(model.clips.len());
    for clip in &model.clips {
        w.string(&clip.name);
        w.f32(clip.duration);
        w.count(clip.channels.len());
        for channel in &clip.channels {
            w.count(channel.node);
            w.u32(match channel.property {
                Property::Translation => 0,
                Property::Rotation => 1,
                Property::Scale => 2,
                Property::MorphTargetWeights => return Err("cannot cook morph animation".into()),
            });
            w.boolean(channel.step);
            require(
                channel.times.len() == channel.values.len(),
                "key/value count",
            )?;
            w.count(channel.times.len());
            for (&time, value) in channel.times.iter().zip(&channel.values) {
                w.f32(time);
                w.array(&value.to_array());
            }
        }
    }
    let refs = References::new(&model.assets)?;
    w.count(model.draws.len());
    for draw in &model.draws {
        refs.instance(w, &draw.instance)?;
        w.count(draw.node);
        w.optional_index(draw.skin);
    }
    Ok(())
}

pub(crate) fn decode(r: &mut Reader<'_>) -> Result<AnimatedModel> {
    let assets = read_assets(r)?;
    let warnings = read_strings(r)?;
    let nodes = r.vec(4096, 48, |r| {
        let local = Trs {
            translation: Vec3::from_array(r.array()?),
            rotation: Quat::from_array(r.array()?),
            scale: Vec3::from_array(r.array()?),
        };
        let matrix = if r.boolean()? {
            Some(r.matrix()?)
        } else {
            None
        };
        if matrix.is_none() {
            require(local.scale.abs().min_element() >= 1e-8, "zero rest scale")?;
            require(
                (local.rotation.length_squared() - 1.0).abs() <= 0.001,
                "rest quaternion",
            )?;
        }
        // Matrix-node decomposed TRS is retained but never evaluated by sampling.
        // Parent indices are validated after the complete node table is available.
        let parent = r.optional_index(4096)?;
        checked_matrix(matrix.unwrap_or_else(|| local.matrix())).map_err(|e| e.to_string())?;
        Ok(Node {
            local,
            matrix,
            parent,
        })
    })?;
    require(!nodes.is_empty(), "empty hierarchy")?;
    let order = r.vec(4096, 4, |r| r.index(nodes.len()))?;
    require(order.len() == nodes.len(), "hierarchy order length")?;
    r.charge(nodes.len() * size_of::<usize>(), nodes.len())?;
    let mut depths = vec![0usize; nodes.len()];
    for &index in &order {
        require(depths[index] == 0, "duplicate hierarchy order")?;
        let depth = if let Some(parent) = nodes[index].parent {
            require(
                parent < nodes.len() && depths[parent] != 0,
                "hierarchy parent/order/cycle",
            )?;
            depths[parent] + 1
        } else {
            1
        };
        require(depth <= 128, "hierarchy depth")?;
        depths[index] = depth;
    }
    let skins = r.vec(256, 4, |r| {
        let n = r.count::<usize>(MAX_SKIN_JOINTS, 68)?;
        require(n > 0, "empty skin")?;
        r.charge(n * size_of::<Mat4>(), n)?;
        let mut joints = Vec::new();
        joints
            .try_reserve_exact(n)
            .map_err(|_| "joint allocation".to_string())?;
        let mut inverse_bind = Vec::new();
        inverse_bind
            .try_reserve_exact(n)
            .map_err(|_| "bind allocation".to_string())?;
        r.charge(0, n * n)?;
        for _ in 0..n {
            let joint = r.index(nodes.len())?;
            require(!joints.contains(&joint), "duplicate skin joint")?;
            joints.push(joint);
            inverse_bind.push(r.matrix()?);
        }
        Ok(Skin {
            joints,
            inverse_bind,
        })
    })?;
    let mut keys = 0usize;
    let mut channel_count = 0usize;
    let clips = r.vec(256, 12, |r| {
        let name = r.string()?;
        let duration = r.f32()?;
        require((0.0..=1e7).contains(&duration), "clip duration")?;
        // A fixed bit table avoids hash allocation or adversarial hash work.
        r.charge(nodes.len(), nodes.len())?;
        let mut targets = vec![0u8; nodes.len()];
        let mut last = 0.0f32;
        let channels = r.vec(RECORDS, 16, |r| {
            channel_count += 1;
            require(channel_count <= RECORDS, "total channels")?;
            let node = r.index(nodes.len())?;
            require(nodes[node].matrix.is_none(), "matrix animation target")?;
            let tag = r.u32()?;
            let property = match tag {
                0 => Property::Translation,
                1 => Property::Rotation,
                2 => Property::Scale,
                _ => return Err("invalid cooked animation property".into()),
            };
            let bit = 1u8 << tag;
            require(targets[node] & bit == 0, "duplicate animation target")?;
            targets[node] |= bit;
            let step = r.boolean()?;
            let n = r.count::<f32>(1_000_000, 20)?;
            keys += n;
            require(n > 0 && keys <= 1_000_000, "total animation keys")?;
            r.charge(n * size_of::<Vec4>(), n)?;
            let mut times = Vec::new();
            times
                .try_reserve_exact(n)
                .map_err(|_| "key allocation".to_string())?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(n)
                .map_err(|_| "value allocation".to_string())?;
            for _ in 0..n {
                let time = r.f32()?;
                require(
                    (0.0..=duration).contains(&time) && times.last().is_none_or(|&t| t < time),
                    "key time range/order",
                )?;
                let value = Vec4::from_array(r.array()?);
                match property {
                    Property::Rotation => require(
                        (value.length_squared() - 1.0).abs() <= 0.001,
                        "key quaternion",
                    )?,
                    Property::Scale => require(
                        value.w == 0.0 && value.truncate().abs().min_element() >= 1e-8,
                        "key scale",
                    )?,
                    Property::Translation => require(value.w == 0.0, "translation key padding")?,
                    Property::MorphTargetWeights => unreachable!(),
                }
                // Keep the exact imported bits: normalizing again would change sampling.
                times.push(time);
                values.push(value);
            }
            last = last.max(times[n - 1]);
            Ok(Channel {
                node,
                property,
                step,
                times,
                values,
            })
        })?;
        require(last == duration, "duration does not match keys")?;
        Ok(AnimationClip {
            name,
            duration,
            channels,
        })
    })?;
    let mut palette_work = skins.iter().map(|s| s.joints.len()).sum::<usize>();
    let draws = r.vec(RECORDS, 100, |r| {
        let instance = read_instance(r, &assets)?;
        require(
            instance.skin.is_none(),
            "animated draw already has sampled palette",
        )?;
        let node = r.index(nodes.len())?;
        let skin = r.optional_index(skins.len())?;
        let palette_len = skin.map_or(1, |i| skins[i].joints.len());
        r.charge(0, assets.meshes.len())?;
        let mesh = assets
            .meshes
            .iter()
            .find(|m| m.handle == instance.mesh)
            .ok_or_else(|| "missing draw mesh".to_string())?;
        // Charge stream validation on every draw, including shared meshes, rather
        // than allowing tiny draw records to multiply unbounded vertex work.
        let stream = mesh
            .skin
            .as_ref()
            .ok_or_else(|| "missing animated skin stream".to_string())?;
        r.charge(0, stream.len() * 8)?;
        require(
            stream
                .iter()
                .all(|v| v.joints.iter().all(|&j| usize::from(j) < palette_len)),
            "draw palette joint index",
        )?;
        palette_work += palette_len;
        Ok(Draw {
            instance,
            node,
            skin,
        })
    })?;
    require(!draws.is_empty(), "empty animated draws")?;
    let model = AnimatedModel {
        assets,
        clips,
        warnings,
        nodes,
        order,
        skins,
        draws,
    };
    // Bound and validate the same hierarchy/palette work that sampling performs.
    r.charge(
        model.nodes.len() * (size_of::<Mat4>() + size_of::<Trs>())
            + palette_work * size_of::<Mat4>() * 2
            + (model.skins.len() + model.draws.len()) * 16
            + model.draws.len() * size_of::<MeshInstance>()
            + model.skins.len() * size_of::<Arc<[Mat4]>>(),
        model.nodes.len() + palette_work,
    )?;
    model.sample_rest().map_err(|e| e.to_string())?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        animated_glb::{AnimationSample, load_animated_glb},
        cooked::{cook_glb, load_animated, load_static},
        glb::tests::Fixture,
    };
    use serde_json::json;

    fn fixture() -> Fixture {
        let mut f = Fixture::triangle();
        f.texture(image::ImageFormat::Png);
        f.json["nodes"] = json!([
            {"translation":[10,0,0],"children":[1,2]},
            {"mesh":0,"skin":0,"translation":[100,0,0]},
            {"translation":[0,2,0]}
        ]);
        let bind = f.floats(
            &[Mat4::from_translation(Vec3::new(-10.0, -2.0, 0.0)).to_cols_array()],
            "MAT4",
        );
        f.json["skins"] = json!([{"joints":[2],"inverseBindMatrices":bind}]);
        let joints = f.accessor(&vec![0; f.vertex_count * 4], f.vertex_count, 5121, "VEC4");
        let weights = f.floats(&vec![[1.0, 0.0, 0.0, 0.0]; f.vertex_count], "VEC4");
        f.attribute("JOINTS_0", joints);
        f.attribute("WEIGHTS_0", weights);
        let input = f.floats(&[[0.0], [2.0]], "SCALAR");
        let output = f.floats(&[[0.0, 2.0, 0.0], [4.0, 2.0, 0.0]], "VEC3");
        let rotation = Quat::from_rotation_z(0.7);
        let rotations = f.floats(&[Quat::IDENTITY.to_array(), rotation.to_array()], "VEC4");
        let scales = f.floats(&[[1.0, 1.0, 1.0], [2.0, 2.0, 2.0]], "VEC3");
        f.json["animations"] = json!([{"name":"walk","samplers":[
            {"input":input,"output":output}, {"input":input,"output":rotations}, {"input":input,"output":scales,"interpolation":"STEP"}
        ],"channels":[
            {"sampler":0,"target":{"node":2,"path":"translation"}},
            {"sampler":1,"target":{"node":2,"path":"rotation"}},
            {"sampler":2,"target":{"node":2,"path":"scale"}}
        ]}]);
        f
    }
    fn equal_draws(a: Vec<MeshInstance>, b: Vec<MeshInstance>) {
        assert_eq!(a.len(), b.len());
        for (mut a, b) in a.into_iter().zip(b) {
            a.mesh = b.mesh;
            a.texture = b.texture;
            a.normal_texture = b.normal_texture;
            a.metallic_roughness_texture = b.metallic_roughness_texture;
            assert_eq!(a, b);
        }
    }
    #[test]
    fn animated_roundtrip_rest_sampling_blending_and_fresh_handles() {
        let source = fixture().bytes();
        let original = load_animated_glb(&source).unwrap();
        let bytes = cook_glb(&source, true).unwrap();
        assert_eq!(bytes, cook_glb(&source, true).unwrap());
        let model = load_animated(&bytes).unwrap();
        let second = load_animated(&bytes).unwrap();
        assert_ne!(
            model.assets.meshes[0].handle,
            second.assets.meshes[0].handle
        );
        assert_ne!(
            model.assets.textures[0].handle,
            second.assets.textures[0].handle
        );
        assert_eq!(model.order, original.order);
        assert_eq!(model.clips[0].name, original.clips[0].name);
        assert_eq!(model.clips[0].duration, original.clips[0].duration);
        assert_eq!(
            model.assets.meshes[0].indices,
            original.assets.meshes[0].indices
        );
        assert_eq!(
            model.assets.textures[0].rgba,
            original.assets.textures[0].rgba
        );
        for (a, b) in model.assets.meshes[0]
            .skin
            .as_ref()
            .unwrap()
            .iter()
            .zip(original.assets.meshes[0].skin.as_ref().unwrap())
        {
            assert_eq!(a.joints, b.joints);
            assert_eq!(a.weights, b.weights);
        }
        equal_draws(
            original.sample_rest().unwrap(),
            model.sample_rest().unwrap(),
        );
        for seconds in [-1.0, 0.0, 0.5, 1.0, 1.9, 2.0, 5.0] {
            for looping in [false, true] {
                equal_draws(
                    original.sample(0, seconds, looping).unwrap(),
                    model.sample(0, seconds, looping).unwrap(),
                );
            }
        }
        let a = AnimationSample {
            clip: 0,
            seconds: 0.5,
            looping: false,
        };
        let b = AnimationSample { seconds: 1.5, ..a };
        for weight in [0.0, 0.25, 0.5, 1.0] {
            equal_draws(
                original.sample_blended(a, b, weight).unwrap(),
                model.sample_blended(a, b, weight).unwrap(),
            );
        }
        assert!(load_static(&bytes).is_err());
        for end in 0..bytes.len() {
            assert!(load_animated(&bytes[..end]).is_err());
            if end >= 32 {
                let mut prefix = bytes[..end].to_vec();
                prefix[24..32].copy_from_slice(&((end - 32) as u64).to_le_bytes());
                assert!(load_animated(&prefix).is_err(), "payload prefix {end}");
            }
        }
        for offset in 0..bytes.len() {
            let mut b = bytes.clone();
            b[offset] ^= 0xff;
            let _ = load_animated(&b);
        }
    }
    #[test]
    fn rigid_matrix_nodes_and_empty_clips_roundtrip() {
        let mut f = Fixture::triangle();
        // An affine shear must retain its matrix, not be rebuilt from lossy TRS.
        f.json["nodes"][0] = json!({"mesh":0,"matrix":[1,0,0,0, 0.25,1,0,0, 0,0,1,0, 2,3,4,1]});
        let source = f.bytes();
        let original = load_animated_glb(&source).unwrap();
        let model = load_animated(&cook_glb(&source, true).unwrap()).unwrap();
        assert!(model.clips.is_empty());
        assert!(model.skins.is_empty());
        assert_eq!(model.nodes[0].matrix, original.nodes[0].matrix);
        equal_draws(
            original.sample_rest().unwrap(),
            model.sample_rest().unwrap(),
        );
    }

    #[test]
    fn invalid_private_graph_invariants_are_rejected() {
        let source = fixture().bytes();
        for case in 0..8 {
            let mut m = load_animated_glb(&source).unwrap();
            // No mip chain in this test's 2x1 image until explicitly prepared.
            for t in &mut m.assets.textures {
                t.mipmaps =
                    weaver_render_wgpu::prepare_rgba_mipmaps(t.width, t.height, &t.rgba).unwrap();
            }
            match case {
                0 => m.order[1] = m.order[0],
                1 => m.nodes[0].parent = Some(2),
                2 => m.clips[0].channels[0].times[1] = 0.0,
                3 => m.clips[0].channels[0].node = 9999,
                4 => m.skins[0].joints[0] = 9999,
                5 => m.assets.meshes[0].skin.as_mut().unwrap()[0].joints[7] = 1,
                6 => m.clips[0].channels[0].values[0].x = f32::NAN,
                7 => m.draws[0].skin = Some(999),
                _ => unreachable!(),
            }
            let mut w = Writer::new();
            encode(&mut w, &m).unwrap();
            let bytes = w.finish().unwrap();
            assert!(decode(&mut Reader::new(&bytes)).is_err(), "case {case}");
        }
    }
}
