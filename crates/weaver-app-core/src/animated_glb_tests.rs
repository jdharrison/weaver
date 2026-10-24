use super::*;
use crate::glb::tests::Fixture;
use serde_json::json;

fn skin(f: &mut Fixture) {
    f.json["nodes"] = json!([
        {"translation":[10,0,0],"children":[1,2]},
        {"mesh":0,"skin":0,"translation":[100,0,0]},
        {"translation":[0,2,0]}
    ]);
    let bind = Mat4::from_translation(Vec3::new(-10.0, -2.0, 0.0));
    let bind = f.floats(&[bind.to_cols_array()], "MAT4");
    f.json["skins"] = json!([{"joints":[2],"inverseBindMatrices":bind}]);
    let joints = f.accessor(&vec![0; f.vertex_count * 4], f.vertex_count, 5121, "VEC4");
    let weights = f.floats(&vec![[1.0, 0.0, 0.0, 0.0]; f.vertex_count], "VEC4");
    f.attribute("JOINTS_0", joints);
    f.attribute("WEIGHTS_0", weights);
}
fn animate(f: &mut Fixture, property: &str, values: &[[f32; 3]]) {
    let input = f.floats(&[[0.0], [2.0]], "SCALAR");
    let output = f.floats(values, "VEC3");
    f.json["animations"] = json!([{"name":"walk","samplers":[{"input":input,"output":output}],"channels":[{"sampler":0,"target":{"node":2,"path":property}}]}]);
}
fn fixture() -> Fixture {
    let mut f = Fixture::triangle();
    skin(&mut f);
    animate(&mut f, "translation", &[[0.0, 2.0, 0.0], [4.0, 2.0, 0.0]]);
    f
}
fn translation(model: &AnimatedModel, seconds: f32, looping: bool) -> Vec3 {
    model.sample(0, seconds, looping).unwrap()[0]
        .skin
        .as_ref()
        .unwrap()[0]
        .w_axis
        .truncate()
}
fn close(a: Vec3, b: Vec3) {
    assert!((a - b).length() < 1e-5, "{a:?} != {b:?}");
}

#[test]
fn hierarchy_inverse_bind_and_mesh_node_cancellation() {
    let f = fixture();
    assert!(matches!(
        crate::glb::load_glb(&f.bytes()),
        Err(GlbError::Unsupported(_))
    ));
    let m = load_animated_glb(&f.bytes()).unwrap();
    assert_eq!(m.assets.meshes[0].vertices[1].position, [1.0, 0.0, 0.0]);
    close(
        m.sample_rest().unwrap()[0].skin.as_ref().unwrap()[0]
            .w_axis
            .truncate(),
        Vec3::ZERO,
    );
    close(translation(&m, 1.0, false), Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(m.clips[0].name, "walk");
    assert_eq!(m.clips[0].duration, 2.0);
}
#[test]
fn clamp_wrap_step_and_zero_duration() {
    let mut f = fixture();
    let m = load_animated_glb(&f.bytes()).unwrap();
    for (time, looping, x) in [
        (-1.0, false, 0.0),
        (2.0, false, 4.0),
        (200.0, false, 4.0),
        (2.0, true, 0.0),
        (3.0, true, 2.0),
        (-1.0, true, 2.0),
    ] {
        close(translation(&m, time, looping), Vec3::new(x, 0.0, 0.0));
    }
    f.json["animations"][0]["samplers"][0]["interpolation"] = json!("STEP");
    let m = load_animated_glb(&f.bytes()).unwrap();
    close(translation(&m, 1.9, false), Vec3::ZERO);
    close(translation(&m, 2.0, false), Vec3::new(4.0, 0.0, 0.0));
    let input = f.floats(&[[0.0]], "SCALAR");
    let output = f.floats(&[[3.0, 2.0, 0.0]], "VEC3");
    f.json["animations"][0]["samplers"][0]["input"] = json!(input);
    f.json["animations"][0]["samplers"][0]["output"] = json!(output);
    let m = load_animated_glb(&f.bytes()).unwrap();
    close(translation(&m, 999.0, true), Vec3::new(3.0, 0.0, 0.0));
}
#[test]
fn assets_can_be_drained_and_actor_palettes_are_independent() {
    let mut f = fixture();
    f.texture(image::ImageFormat::Png);
    let mut m = load_animated_glb(&f.bytes()).unwrap();
    let assets = std::mem::take(&mut m.assets);
    let a = m.sample(0, 0.0, false).unwrap();
    let b = m.sample(0, 2.0, false).unwrap();
    assert_eq!(a[0].mesh, b[0].mesh);
    assert_eq!(a[0].mesh, assets.meshes[0].handle);
    assert_eq!(a[0].texture, Some(assets.textures[0].handle));
    assert!(!Arc::ptr_eq(
        a[0].skin.as_ref().unwrap(),
        b[0].skin.as_ref().unwrap()
    ));
    close(a[0].skin.as_ref().unwrap()[0].w_axis.truncate(), Vec3::ZERO);
    close(
        b[0].skin.as_ref().unwrap()[0].w_axis.truncate(),
        Vec3::new(4.0, 0.0, 0.0),
    );
}
#[test]
fn blends_local_trs_and_shortest_quaternions() {
    let mut f = fixture();
    let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let output = f.floats(&[rotation.to_array(), (-rotation).to_array()], "VEC4");
    f.json["animations"][0]["samplers"][0]["output"] = json!(output);
    f.json["animations"][0]["channels"][0]["target"]["path"] = json!("rotation");
    let m = load_animated_glb(&f.bytes()).unwrap();
    let locals = m
        .local_sample(AnimationSample {
            clip: 0,
            seconds: 1.0,
            looping: false,
        })
        .unwrap();
    close(locals[2].rotation * Vec3::X, Vec3::Y);
    let m = load_animated_glb(&fixture().bytes()).unwrap();
    let a = AnimationSample {
        clip: 0,
        seconds: 0.0,
        looping: false,
    };
    let b = AnimationSample { seconds: 2.0, ..a };
    let blended = m.sample_blended(a, b, 0.25).unwrap();
    close(
        blended[0].skin.as_ref().unwrap()[0].w_axis.truncate(),
        Vec3::X,
    );
    assert!(m.sample_blended(a, b, f32::NAN).is_err());
    assert!(m.sample(99, 0.0, false).is_err());
    assert!(m.sample(0, f32::INFINITY, false).is_err());
}
#[test]
fn eight_influences_survive_u16_chunk_remap() {
    let count = 65_538;
    let positions: Vec<_> = (0..count).map(|i| [i as f32, 0.0, 0.0]).collect();
    let mut f = Fixture::new(&positions, Some([0.0, 1.0, 0.0]));
    skin(&mut f);
    f.json["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    f.json["nodes"][0]["children"] = json!([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    for _ in 0..7 {
        f.json["nodes"].as_array_mut().unwrap().push(json!({}));
    }
    f.json["skins"][0]["joints"] = json!([2, 3, 4, 5, 6, 7, 8, 9]);
    for set in 0..2 {
        let js: Vec<u8> = (0..count)
            .flat_map(|i| (0..4).map(move |j| ((i + set * 4 + j) % 8) as u8))
            .collect();
        let j = f.accessor(&js, count, 5121, "VEC4");
        let w = f.floats(&vec![[0.125; 4]; count], "VEC4");
        f.attribute(&format!("JOINTS_{set}"), j);
        f.attribute(&format!("WEIGHTS_{set}"), w);
    }
    f.indices(&(0..count as u32).collect::<Vec<_>>(), 5125);
    let m = load_animated_glb(&f.bytes()).unwrap();
    assert_eq!(m.assets.meshes.len(), 2);
    let mut seen = 0;
    for mesh in &m.assets.meshes {
        for &index in &mesh.indices {
            let v = &mesh.vertices[index as usize];
            let s = &mesh.skin.as_ref().unwrap()[index as usize];
            let original = v.position[0] as usize;
            assert_eq!(
                s.joints,
                std::array::from_fn(|j| ((original + j) % 8) as u16)
            );
            assert_eq!(s.weights, [0.125; 8]);
            seen += 1;
        }
    }
    assert_eq!(seen, count);
    let draws = m.sample_rest().unwrap();
    assert!(Arc::ptr_eq(
        draws[0].skin.as_ref().unwrap(),
        draws[1].skin.as_ref().unwrap()
    ));
}
#[test]
fn normalized_integer_weights_and_missing_inverse_binds() {
    let mut f = fixture();
    f.json["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    let w = f.accessor(&[255, 0, 0, 0].repeat(3), 3, 5121, "VEC4");
    f.json["accessors"][w]["normalized"] = json!(true);
    f.attribute("WEIGHTS_0", w);
    let m = load_animated_glb(&f.bytes()).unwrap();
    close(translation(&m, 0.0, false), Vec3::new(10.0, 2.0, 0.0));
    assert_eq!(m.assets.meshes[0].skin.as_ref().unwrap()[0].weights[0], 1.0);
}
#[test]
fn rigid_animated_nodes_and_matrix_ancestors_preserve_local_semantics() {
    let mut f = Fixture::triangle();
    f.json["nodes"] = json!([{"matrix":Mat4::from_translation(Vec3::new(7.0,0.0,0.0)).to_cols_array(),"children":[1]}, {"children":[2]}, {"mesh":0}]);
    animate(&mut f, "translation", &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
    let m = load_animated_glb(&f.bytes()).unwrap();
    close(translation(&m, 1.0, false), Vec3::new(8.0, 0.0, 0.0));
}
#[test]
fn animated_nonjoint_parent_and_local_blending_retain_defaults() {
    let mut f = fixture();
    let output = f.floats(&[[10.0, 0.0, 0.0], [14.0, 0.0, 0.0]], "VEC3");
    f.json["animations"][0]["samplers"][0]["output"] = json!(output);
    f.json["animations"][0]["channels"][0]["target"]["node"] = json!(0);
    let mut second = f.json["animations"][0].clone();
    let output = f.floats(&[[0.0, 2.0, 0.0], [0.0, 6.0, 0.0]], "VEC3");
    second["samplers"][0]["output"] = json!(output);
    second["channels"][0]["target"]["node"] = json!(2);
    f.json["animations"].as_array_mut().unwrap().push(second);
    let m = load_animated_glb(&f.bytes()).unwrap();
    close(translation(&m, 1.0, false), Vec3::new(2.0, 0.0, 0.0));
    let a = AnimationSample {
        clip: 0,
        seconds: 2.0,
        looping: false,
    };
    let b = AnimationSample { clip: 1, ..a };
    let blended = m.sample_blended(a, b, 0.5).unwrap();
    close(
        blended[0].skin.as_ref().unwrap()[0].w_axis.truncate(),
        Vec3::new(2.0, 2.0, 0.0),
    );
}

#[test]
fn palette_limit_and_singular_intermediate_pose_fail_explicitly() {
    let mut f = fixture();
    f.json["skins"][0]["joints"] = json!(vec![2; MAX_SKIN_JOINTS + 1]);
    assert!(matches!(
        load_animated_glb(&f.bytes()),
        Err(GlbError::LimitExceeded("skin joints"))
    ));
    let mut f = fixture();
    animate(&mut f, "scale", &[[1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]);
    let mut m = load_animated_glb(&f.bytes()).unwrap();
    assert!(m.sample(0, 1.0, false).is_err());
    assert!(m.sample(0, 2.0, false).is_ok());
    m.clips[0].duration = f32::NAN;
    assert!(m.sample(0, 0.0, false).is_err());
}

#[test]
fn rejects_out_of_range_animation_target_node_without_panicking() {
    let mut f = fixture();
    f.json["animations"][0]["channels"][0]["target"]["node"] =
        json!(f.json["nodes"].as_array().unwrap().len());
    let bytes = f.bytes();
    let result = std::panic::catch_unwind(|| load_animated_glb(&bytes))
        .expect("out-of-range animation target must not panic");
    assert!(matches!(result, Err(GlbError::Invalid(_))));
}

#[test]
fn rejects_invalid_animation_target_path_without_panicking() {
    let mut f = fixture();
    f.json["animations"][0]["channels"][0]["target"]["path"] = json!("invalid_path");
    let bytes = f.bytes();
    let result = std::panic::catch_unwind(|| load_animated_glb(&bytes))
        .expect("invalid animation target path must not panic");
    assert!(matches!(result, Err(GlbError::Invalid(_))));
}

#[test]
fn rejects_bad_skin_and_animation_data_without_panics() {
    let original = fixture();
    let bytes = original.bytes();
    for end in 0..bytes.len() {
        assert!(load_animated_glb(&bytes[..end]).is_err());
    }
    for change in 0..14 {
        let mut f = fixture();
        match change {
            0 => f.json["nodes"][2]["children"] = json!([0]),
            1 => f.json["nodes"][1]["children"] = json!([2]),
            2 => f.json["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE"),
            3 => {
                let v = f.floats(&[[0.0], [0.0]], "SCALAR");
                f.json["animations"][0]["samplers"][0]["input"] = json!(v);
            }
            4 => {
                let v = f.floats(&[[f32::NAN, 0.0, 0.0]; 2], "VEC3");
                f.json["animations"][0]["samplers"][0]["output"] = json!(v);
            }
            5 => {
                let v = f.accessor(&[1, 0, 0, 0].repeat(3), 3, 5121, "VEC4");
                f.attribute("JOINTS_0", v);
            }
            6 => {
                let v = f.floats(&[[0.0; 4]; 3], "VEC4");
                f.attribute("WEIGHTS_0", v);
            }
            7 => {
                let v = f.floats(&[[f32::NAN, 0.0, 0.0, 0.0]; 3], "VEC4");
                f.attribute("WEIGHTS_0", v);
            }
            8 => {
                let v = f.floats(&[[1.0; 4]; 3], "VEC4");
                f.attribute("WEIGHTS_1", v);
            }
            9 => {
                let v = f.floats(&[[1.0; 4]; 3], "VEC4");
                f.attribute("WEIGHTS_2", v);
            }
            10 => f.json["skins"][0]["joints"] = json!([2, 2]),
            11 => f.json["accessors"][0]["byteOffset"] = json!(10_000_000),
            12 => f.json["buffers"][0]["uri"] = json!("external.bin"),
            13 => {
                f.texture(image::ImageFormat::Png);
                f.json["images"][0]["uri"] = json!("external.png");
            }
            _ => unreachable!(),
        }
        assert!(load_animated_glb(&f.bytes()).is_err(), "case {change}");
    }
}
