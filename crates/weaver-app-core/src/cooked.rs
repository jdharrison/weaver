//! Versioned CPU-only runtime assets. Files contain local indices, never process handles.
//!
//! The 32-byte envelope is magic, version, endian marker, type, reserved zero and
//! u64 payload length. All integers and IEEE f32 values are explicitly little-endian.
//! Decode budgets bound cumulative allocations and scalar/record work, not peak
//! process memory or GPU residency. Decode charges at most 512 MiB of cumulative
//! allocations and 256 million work units (scalars, pixels, records and validation
//! visits), with tighter per-table, image, mesh, key and hierarchy limits. No source
//! image decoding or mip generation occurs on load.

use std::{collections::HashMap, mem::size_of, sync::Arc};

use glam::{Mat4, Quat, Vec3};
use weaver_render::{
    MeshHandle, MeshInstance, RenderTransform, SpriteHandle,
    mesh::{MAX_SKIN_JOINTS, MeshLighting, MeshRenderMode, MeshTextureFit},
};
use weaver_render_wgpu::{SkinVertex, Vertex};

use crate::{AppAssets, MeshAsset, TextureAsset, animated_glb::AnimatedModel, glb::GlbModel};

type Result<T> = std::result::Result<T, String>;
/// Current cooked binary schema. Incompatible schemas are rejected, not guessed.
pub const FORMAT_VERSION: u32 = 1;
/// Maximum complete file size, and cumulative decode allocation budget (512 MiB).
pub const MAX_COOKED_BYTES: usize = 512 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"WVCOOK\0\0";
const ENDIAN: u32 = 0x0102_0304;
const HEADER: usize = 32;
pub(crate) const RECORDS: usize = 16_384;
const MAX_WORK: usize = 256_000_000;

fn bad(message: &str) -> String {
    format!("invalid cooked asset: {message}")
}
pub(crate) fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(bad(message)) }
}

/// Import a self-contained GLB and prepare a deterministic runtime binary.
/// Animated and static assets have distinct types and cannot be loaded interchangeably.
pub fn cook_glb(bytes: &[u8], animated: bool) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    w.raw(MAGIC);
    w.u32(FORMAT_VERSION);
    w.u32(ENDIAN);
    w.u32(if animated { 2 } else { 1 });
    w.u32(0);
    w.raw(&0u64.to_le_bytes());
    if animated {
        let mut model = crate::animated_glb::load_animated_glb(bytes).map_err(|e| e.to_string())?;
        prepare_textures(&mut model.assets)?;
        crate::animated_glb::cooked_animation::encode(&mut w, &model)?;
    } else {
        let mut model = crate::glb::load_glb(bytes).map_err(|e| e.to_string())?;
        prepare_textures(&mut model.assets)?;
        write_assets(&mut w, &model.assets)?;
        w.array(&model.bounds_min.to_array());
        w.array(&model.bounds_max.to_array());
        write_strings(&mut w, &model.warnings);
        let refs = References::new(&model.assets)?;
        w.count(model.instances.len());
        for instance in &model.instances {
            refs.instance(&mut w, instance)?;
        }
    }
    let mut bytes = w.finish()?;
    let payload_length = (bytes.len() - HEADER) as u64;
    bytes[24..32].copy_from_slice(&payload_length.to_le_bytes());
    // The encoder and decoder share the schema's invariants. Never emit an asset
    // that this version cannot safely load (including aggregate resource limits).
    if animated {
        load_animated(&bytes)?;
    } else {
        load_static(&bytes)?;
    }
    Ok(bytes)
}

fn prepare_textures(assets: &mut AppAssets) -> Result<()> {
    let mut total = 0usize;
    for texture in &mut assets.textures {
        let base = image_bytes(texture.width, texture.height)?;
        // Charge a conservative upper bound for the full chain before generating it.
        total = total
            .checked_add(base.checked_mul(2).ok_or_else(|| bad("mip budget"))?)
            .ok_or_else(|| bad("mip budget"))?;
        require(total <= MAX_COOKED_BYTES, "mip budget")?;
        texture.mipmaps =
            weaver_render_wgpu::prepare_rgba_mipmaps(texture.width, texture.height, &texture.rgba)?;
    }
    Ok(())
}

fn envelope(bytes: &[u8], kind: u32) -> Result<Reader<'_>> {
    require(
        bytes.len() <= MAX_COOKED_BYTES && bytes.len() >= HEADER,
        "file size",
    )?;
    let mut r = Reader::new(bytes);
    require(r.take(8)? == MAGIC, "magic")?;
    require(r.u32()? == FORMAT_VERSION, "version")?;
    require(r.u32()? == ENDIAN, "endian marker")?;
    require(r.u32()? == kind, "asset type")?;
    require(r.u32()? == 0, "reserved header")?;
    let length = u64::from_le_bytes(r.take(8)?.try_into().map_err(|_| bad("length"))?);
    require(length == (bytes.len() - HEADER) as u64, "payload length")?;
    Ok(r)
}

/// Load static ready-to-upload geometry, images and instances with fresh handles.
pub fn load_static(bytes: &[u8]) -> Result<GlbModel> {
    let mut r = envelope(bytes, 1)?;
    let assets = read_assets(&mut r)?;
    require(
        assets.meshes.iter().all(|m| m.skin.is_none()),
        "static skin stream",
    )?;
    let bounds_min = Vec3::from_array(r.array()?);
    let bounds_max = Vec3::from_array(r.array()?);
    require(bounds_min.cmple(bounds_max).all(), "bounds order")?;
    let warnings = read_strings(&mut r)?;
    let instances = r.vec(RECORDS, 92, |r| read_instance(r, &assets))?;
    require(!instances.is_empty(), "empty static model")?;
    for mesh in &assets.meshes {
        r.charge(0, mesh.indices.len())?;
        for &i in &mesh.indices {
            let p = Vec3::from_array(mesh.vertices[usize::from(i)].position);
            require(
                p.cmpge(bounds_min).all() && p.cmple(bounds_max).all(),
                "geometry outside bounds",
            )?;
        }
    }
    require(instances.iter().all(|i| i.skin.is_none()), "static palette")?;
    r.end()?;
    Ok(GlbModel {
        assets,
        instances,
        bounds_min,
        bounds_max,
        warnings,
    })
}

/// Load an immutable animated graph with fresh geometry/image handles.
pub fn load_animated(bytes: &[u8]) -> Result<AnimatedModel> {
    let mut r = envelope(bytes, 2)?;
    let model = crate::animated_glb::cooked_animation::decode(&mut r)?;
    r.end()?;
    Ok(model)
}

pub(crate) struct Writer {
    bytes: Vec<u8>,
    error: Option<String>,
}
impl Writer {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::new(),
            error: None,
        }
    }
    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        if bytes.len() > MAX_COOKED_BYTES.saturating_sub(self.bytes.len()) {
            self.error = Some(bad("encoded byte budget"));
            return;
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .clamp(256, MAX_COOKED_BYTES);
            if self
                .bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .is_err()
            {
                self.error = Some(bad("encoded allocation"));
                return;
            }
        }
        self.bytes.extend_from_slice(bytes);
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.raw(&v.to_le_bytes());
    }
    pub(crate) fn count(&mut self, v: usize) {
        match u32::try_from(v) {
            Ok(v) => self.u32(v),
            Err(_) => self.error = Some(bad("count overflow")),
        }
    }
    pub(crate) fn boolean(&mut self, v: bool) {
        self.u32(u32::from(v));
    }
    pub(crate) fn f32(&mut self, v: f32) {
        self.raw(&v.to_le_bytes());
    }
    pub(crate) fn array<const N: usize>(&mut self, v: &[f32; N]) {
        for &v in v {
            self.f32(v);
        }
    }
    pub(crate) fn string(&mut self, v: &str) {
        self.count(v.len());
        self.raw(v.as_bytes());
    }
    pub(crate) fn matrix(&mut self, v: Mat4) {
        self.array(&v.to_cols_array());
    }
    pub(crate) fn optional_index(&mut self, v: Option<usize>) {
        match v {
            None => self.u32(u32::MAX),
            Some(v) if v < u32::MAX as usize => self.count(v),
            Some(_) => self.error = Some(bad("optional index overflow")),
        }
    }
    pub(crate) fn finish(self) -> Result<Vec<u8>> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.bytes),
        }
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    allocated: usize,
    work: usize,
}
impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            allocated: 0,
            work: 0,
        }
    }
    pub(crate) fn charge(&mut self, bytes: usize, work: usize) -> Result<()> {
        self.allocated = self
            .allocated
            .checked_add(bytes)
            .ok_or_else(|| bad("allocation overflow"))?;
        self.work = self
            .work
            .checked_add(work)
            .ok_or_else(|| bad("work overflow"))?;
        require(
            self.allocated <= MAX_COOKED_BYTES && self.work <= MAX_WORK,
            "total allocation/work budget",
        )
    }
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        require(
            len <= self.bytes.len().saturating_sub(self.offset),
            "truncated payload",
        )?;
        let result = &self.bytes[self.offset..self.offset + len];
        self.offset += len;
        Ok(result)
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| bad("u32"))?,
        ))
    }
    pub(crate) fn boolean(&mut self) -> Result<bool> {
        let v = self.u32()?;
        require(v <= 1, "boolean")?;
        Ok(v == 1)
    }
    pub(crate) fn f32(&mut self) -> Result<f32> {
        self.charge(0, 1)?;
        let v = f32::from_le_bytes(self.take(4)?.try_into().map_err(|_| bad("f32"))?);
        require(
            v.is_finite() && v.abs() <= 1e12,
            "nonfinite/unbounded float",
        )?;
        Ok(v)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
    pub(crate) fn count<T>(&mut self, max: usize, minimum_bytes: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        require(
            n <= max && n <= (self.bytes.len() - self.offset) / minimum_bytes.max(1),
            "count/remaining bytes",
        )?;
        self.charge(
            n.checked_mul(size_of::<T>())
                .ok_or_else(|| bad("allocation overflow"))?,
            n,
        )?;
        Ok(n)
    }
    pub(crate) fn vec<T>(
        &mut self,
        max: usize,
        minimum_bytes: usize,
        mut read: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let n = self.count::<T>(max, minimum_bytes)?;
        let mut out = Vec::new();
        out.try_reserve_exact(n)
            .map_err(|_| bad("allocation failed"))?;
        for _ in 0..n {
            out.push(read(self)?);
        }
        Ok(out)
    }
    pub(crate) fn string(&mut self) -> Result<String> {
        let n = self.count::<u8>(1024 * 1024, 1)?;
        let s = std::str::from_utf8(self.take(n)?).map_err(|_| bad("UTF-8"))?;
        let mut out = String::new();
        out.try_reserve_exact(n)
            .map_err(|_| bad("string allocation"))?;
        out.push_str(s);
        Ok(out)
    }
    pub(crate) fn index(&mut self, count: usize) -> Result<usize> {
        let v = self.u32()? as usize;
        require(v < count, "index out of range")?;
        Ok(v)
    }
    pub(crate) fn optional_index(&mut self, count: usize) -> Result<Option<usize>> {
        let v = self.u32()?;
        if v == u32::MAX {
            Ok(None)
        } else {
            require((v as usize) < count, "optional index")?;
            Ok(Some(v as usize))
        }
    }
    pub(crate) fn matrix(&mut self) -> Result<Mat4> {
        let m = Mat4::from_cols_array(&self.array()?);
        require(
            m.x_axis.w == 0.0
                && m.y_axis.w == 0.0
                && m.z_axis.w == 0.0
                && m.w_axis.w == 1.0
                && m.inverse().is_finite(),
            "non-affine/singular matrix",
        )?;
        Ok(m)
    }
    pub(crate) fn quat(&mut self) -> Result<Quat> {
        let q = Quat::from_array(self.array()?);
        require(
            (q.length_squared() - 1.0).abs() <= 0.001,
            "non-unit quaternion",
        )?;
        Ok(q)
    }
    fn end(&self) -> Result<()> {
        require(self.offset == self.bytes.len(), "trailing bytes")
    }
}

pub(crate) fn write_strings(w: &mut Writer, strings: &[String]) {
    w.count(strings.len());
    for s in strings {
        w.string(s);
    }
}
pub(crate) fn read_strings(r: &mut Reader<'_>) -> Result<Vec<String>> {
    r.vec(RECORDS, 4, Reader::string)
}
fn image_bytes(width: u32, height: u32) -> Result<usize> {
    require(
        (1..=4096).contains(&width) && (1..=4096).contains(&height),
        "image dimensions",
    )?;
    Ok(width as usize * height as usize * 4)
}

pub(crate) fn write_assets(w: &mut Writer, assets: &AppAssets) -> Result<()> {
    w.count(assets.meshes.len());
    for mesh in &assets.meshes {
        w.count(mesh.vertices.len());
        for v in &mesh.vertices {
            w.array(&v.position);
            w.array(&v.normal);
            w.array(&v.color);
            w.array(&v.uv);
        }
        w.count(mesh.indices.len());
        for i in &mesh.indices {
            w.raw(&i.to_le_bytes());
        }
        w.boolean(mesh.skin.is_some());
        if let Some(skin) = &mesh.skin {
            require(skin.len() == mesh.vertices.len(), "skin stream length")?;
            for v in skin {
                for i in v.joints {
                    w.raw(&i.to_le_bytes());
                }
                w.array(&v.weights);
            }
        }
    }
    w.count(assets.textures.len());
    for t in &assets.textures {
        w.u32(t.width);
        w.u32(t.height);
        w.count(t.rgba.len());
        w.raw(&t.rgba);
        w.count(t.mipmaps.len());
        for mip in &t.mipmaps {
            w.count(mip.len());
            w.raw(mip);
        }
    }
    Ok(())
}

pub(crate) fn read_assets(r: &mut Reader<'_>) -> Result<AppAssets> {
    let meshes = r.vec(RECORDS, 12, |r| {
        let vertices = r.vec(65_536, 48, |r| {
            Ok(Vertex {
                position: r.array()?,
                normal: r.array()?,
                color: r.array()?,
                uv: r.array()?,
            })
        })?;
        require(!vertices.is_empty(), "empty vertices")?;
        let indices = r.vec(12_000_000, 2, |r| {
            let i = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| bad("u16"))?);
            require(usize::from(i) < vertices.len(), "vertex index")?;
            Ok(i)
        })?;
        require(
            !indices.is_empty() && indices.len().is_multiple_of(3),
            "triangle list",
        )?;
        let skin = if r.boolean()? {
            let n = vertices.len();
            r.charge(n * size_of::<SkinVertex>(), n)?;
            require(
                n <= (r.bytes.len() - r.offset) / 48,
                "truncated skin stream",
            )?;
            let mut skin = Vec::new();
            skin.try_reserve_exact(n)
                .map_err(|_| bad("skin allocation"))?;
            for _ in 0..n {
                let mut joints = [0; 8];
                for j in &mut joints {
                    *j = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| bad("joint"))?);
                }
                let weights: [f32; 8] = r.array()?;
                require(
                    joints.iter().all(|&j| usize::from(j) < MAX_SKIN_JOINTS)
                        && weights.iter().all(|w| (0.0..=1.0).contains(w))
                        && (weights.iter().sum::<f32>() - 1.0).abs() <= 0.001,
                    "skin weights/joints",
                )?;
                skin.push(SkinVertex { joints, weights });
            }
            Some(skin)
        } else {
            None
        };
        Ok(MeshAsset {
            handle: MeshHandle::new(),
            vertices,
            indices,
            skin,
        })
    })?;
    require(!meshes.is_empty(), "empty meshes")?;
    let textures = r.vec(256, 16, |r| {
        let width = r.u32()?;
        let height = r.u32()?;
        let expected = image_bytes(width, height)?;
        let rgba = read_pixels(r, expected)?;
        let mip_count = r.count::<Vec<u8>>(12, 4)?;
        let expected_count = width.max(height).ilog2() as usize;
        require(mip_count == expected_count, "incomplete mip chain")?;
        let mut mipmaps = Vec::new();
        mipmaps
            .try_reserve_exact(mip_count)
            .map_err(|_| bad("mip allocation"))?;
        let (mut w, mut h) = (width, height);
        for _ in 0..mip_count {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            mipmaps.push(read_pixels(r, image_bytes(w, h)?)?);
        }
        Ok(TextureAsset {
            handle: SpriteHandle::new(),
            width,
            height,
            rgba,
            mipmaps,
        })
    })?;
    Ok(AppAssets { meshes, textures })
}
fn read_pixels(r: &mut Reader<'_>, expected: usize) -> Result<Vec<u8>> {
    let n = r.u32()? as usize;
    require(n == expected, "pixel data length")?;
    r.charge(n, n.div_ceil(4))?;
    let data = r.take(n)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n)
        .map_err(|_| bad("pixel allocation"))?;
    out.extend_from_slice(data);
    Ok(out)
}

pub(crate) struct References {
    meshes: HashMap<MeshHandle, usize>,
    textures: HashMap<SpriteHandle, usize>,
}
impl References {
    pub(crate) fn new(assets: &AppAssets) -> Result<Self> {
        let meshes: HashMap<_, _> = assets
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| (m.handle, i))
            .collect();
        let textures: HashMap<_, _> = assets
            .textures
            .iter()
            .enumerate()
            .map(|(i, t)| (t.handle, i))
            .collect();
        require(
            meshes.len() == assets.meshes.len() && textures.len() == assets.textures.len(),
            "duplicate asset handles",
        )?;
        Ok(Self { meshes, textures })
    }
    pub(crate) fn instance(&self, w: &mut Writer, i: &MeshInstance) -> Result<()> {
        w.count(
            *self
                .meshes
                .get(&i.mesh)
                .ok_or_else(|| bad("unknown mesh"))?,
        );
        for texture in [i.texture, i.normal_texture, i.metallic_roughness_texture] {
            w.optional_index(
                texture
                    .map(|t| {
                        self.textures
                            .get(&t)
                            .copied()
                            .ok_or_else(|| bad("unknown texture"))
                    })
                    .transpose()?,
            );
        }
        w.u32(match i.render_mode {
            MeshRenderMode::Surface => 0,
            MeshRenderMode::Additive => 1,
            MeshRenderMode::Water => 2,
        });
        w.f32(i.metallic);
        w.f32(i.roughness);
        w.array(&i.color);
        w.f32(i.emissive);
        w.array(&i.transform.translation.to_array());
        w.array(&i.transform.rotation.to_array());
        w.f32(i.transform.scale);
        w.boolean(i.texture_fit.is_some());
        if let Some(fit) = i.texture_fit {
            w.f32(fit.surface_aspect);
            w.f32(fit.content_aspect);
        }
        w.boolean(i.lighting.is_some());
        if let Some(light) = i.lighting {
            w.array(&light.position.to_array());
            w.f32(light.ambient);
            w.f32(light.intensity);
        }
        w.boolean(i.skin.is_some());
        if let Some(palette) = &i.skin {
            w.count(palette.len());
            for m in palette.iter() {
                w.matrix(*m);
            }
        }
        Ok(())
    }
}

pub(crate) fn read_instance(r: &mut Reader<'_>, assets: &AppAssets) -> Result<MeshInstance> {
    let mesh_index = r.index(assets.meshes.len())?;
    let mesh = assets.meshes[mesh_index].handle;
    let mut textures = [None; 3];
    for t in &mut textures {
        *t = r
            .optional_index(assets.textures.len())?
            .map(|i| assets.textures[i].handle);
    }
    let render_mode = match r.u32()? {
        0 => MeshRenderMode::Surface,
        1 => MeshRenderMode::Additive,
        2 => MeshRenderMode::Water,
        _ => return Err(bad("render mode")),
    };
    let metallic = r.f32()?;
    let roughness = r.f32()?;
    require(
        (0.0..=1.0).contains(&metallic) && (0.0..=1.0).contains(&roughness),
        "material factors",
    )?;
    let color = r.array()?;
    let emissive = r.f32()?;
    let transform = RenderTransform {
        translation: Vec3::from_array(r.array()?),
        rotation: r.quat()?,
        scale: r.f32()?,
    };
    require(transform.scale.abs() >= 1e-8, "zero instance scale")?;
    let texture_fit = if r.boolean()? {
        let f = MeshTextureFit {
            surface_aspect: r.f32()?,
            content_aspect: r.f32()?,
        };
        require(
            f.surface_aspect > 0.0 && f.content_aspect > 0.0,
            "texture aspect",
        )?;
        Some(f)
    } else {
        None
    };
    let lighting = if r.boolean()? {
        Some(MeshLighting {
            position: Vec3::from_array(r.array()?),
            ambient: r.f32()?,
            intensity: r.f32()?,
        })
    } else {
        None
    };
    let skin = if r.boolean()? {
        let palette = r.vec(MAX_SKIN_JOINTS, 64, Reader::matrix)?;
        require(!palette.is_empty(), "empty palette")?;
        r.charge(16 + palette.len() * size_of::<Mat4>(), palette.len())?;
        if let Some(stream) = &assets.meshes[mesh_index].skin {
            r.charge(0, stream.len() * 8)?;
            require(
                stream
                    .iter()
                    .all(|v| v.joints.iter().all(|&j| usize::from(j) < palette.len())),
                "instance palette index",
            )?;
        }
        Some(Arc::from(palette))
    } else {
        None
    };
    Ok(MeshInstance {
        mesh,
        skin,
        render_mode,
        texture: textures[0],
        normal_texture: textures[1],
        metallic_roughness_texture: textures[2],
        metallic,
        roughness,
        texture_fit,
        transform,
        color,
        emissive,
        lighting,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb::tests::Fixture;

    #[test]
    fn static_deterministic_geometry_images_materials_bounds_and_fresh_handles() {
        let mut f = Fixture::triangle();
        f.texture(image::ImageFormat::Png);
        f.json["materials"][0]["normalTexture"] = serde_json::json!({"index": 0});
        f.json["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
            serde_json::json!({"index": 0});
        f.json["materials"][0]["pbrMetallicRoughness"]["metallicFactor"] = serde_json::json!(0.65);
        f.json["materials"][0]["pbrMetallicRoughness"]["roughnessFactor"] = serde_json::json!(0.35);
        let source = f.bytes();
        let original = crate::glb::load_glb(&source).unwrap();
        let bytes = cook_glb(&source, false).unwrap();
        assert_eq!(bytes, cook_glb(&source, false).unwrap());
        let a = load_static(&bytes).unwrap();
        let b = load_static(&bytes).unwrap();
        assert_eq!(a.bounds_min, original.bounds_min);
        assert_eq!(a.bounds_max, original.bounds_max);
        assert_eq!(a.warnings, original.warnings);
        assert_ne!(a.assets.meshes[0].handle, b.assets.meshes[0].handle);
        assert_ne!(a.assets.textures[0].handle, b.assets.textures[0].handle);
        for (a, b) in a.assets.meshes.iter().zip(&original.assets.meshes) {
            assert_eq!(a.indices, b.indices);
            for (a, b) in a.vertices.iter().zip(&b.vertices) {
                assert_eq!(a.position, b.position);
                assert_eq!(a.normal, b.normal);
                assert_eq!(a.uv, b.uv);
                assert_eq!(a.color, b.color);
            }
        }
        assert_eq!(a.assets.textures[0].rgba, original.assets.textures[0].rgba);
        assert_eq!(
            a.assets.textures[0].mipmaps,
            weaver_render_wgpu::prepare_rgba_mipmaps(
                a.assets.textures[0].width,
                a.assets.textures[0].height,
                &a.assets.textures[0].rgba
            )
            .unwrap()
        );
        let mut expected = original.instances[0].clone();
        expected.mesh = a.assets.meshes[0].handle;
        expected.texture = Some(a.assets.textures[0].handle);
        expected.normal_texture = expected.texture;
        expected.metallic_roughness_texture = expected.texture;
        assert_eq!(a.instances[0], expected);
        assert!(load_animated(&bytes).is_err());
    }

    #[test]
    fn envelope_and_payload_reject_malformed_without_panics() {
        let bytes = cook_glb(&Fixture::triangle().bytes(), false).unwrap();
        for end in 0..bytes.len() {
            assert!(load_static(&bytes[..end]).is_err(), "prefix {end}");
            if end >= HEADER {
                let mut prefix = bytes[..end].to_vec();
                prefix[24..32].copy_from_slice(&((end - HEADER) as u64).to_le_bytes());
                assert!(load_static(&prefix).is_err(), "payload prefix {end}");
            }
        }
        for (offset, value) in [
            (0, 0xff),
            (8, 0xff),
            (12, 0xff),
            (16, 2),
            (20, 1),
            (24, 0xff),
            (32, 0xff),
        ] {
            let mut b = bytes.clone();
            b[offset] = value;
            assert!(load_static(&b).is_err(), "offset {offset}");
        }
        let mut b = bytes.clone();
        b.push(0);
        assert!(load_static(&b).is_err());
        let length = (b.len() - HEADER) as u64;
        b[24..32].copy_from_slice(&length.to_le_bytes());
        assert!(load_static(&b).is_err());
        let count = u32::MAX.to_le_bytes();
        let mut r = Reader::new(&count);
        assert!(r.vec::<Vertex>(65_536, 48, |_| unreachable!()).is_err());
        let mut r = Reader::new(&[]);
        assert!(r.charge(MAX_COOKED_BYTES + 1, 0).is_err());
        let mut r = Reader::new(&[]);
        assert!(r.charge(0, MAX_WORK + 1).is_err());
        // Mutations may still describe valid assets, but no byte can induce a panic.
        for offset in 0..bytes.len() {
            let mut b = bytes.clone();
            b[offset] ^= 0xff;
            let _ = load_static(&b);
        }
    }

    #[test]
    fn preserves_all_instance_fields_without_persisting_handles() {
        let mut f = Fixture::triangle();
        f.texture(image::ImageFormat::Png);
        let mut model = crate::glb::load_glb(&f.bytes()).unwrap();
        prepare_textures(&mut model.assets).unwrap();
        let mut instance = model.instances[0].clone();
        instance.render_mode = MeshRenderMode::Additive;
        instance.transform = RenderTransform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_x(0.5),
            scale: 2.5,
        };
        instance.emissive = 0.75;
        instance.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.0,
            content_aspect: 1.5,
        });
        instance.lighting = Some(MeshLighting {
            position: Vec3::new(3.0, 4.0, 5.0),
            ambient: 0.3,
            intensity: 0.8,
        });
        instance.skin = Some(Arc::from([Mat4::from_translation(Vec3::Y)]));
        let mut w = Writer::new();
        write_assets(&mut w, &model.assets).unwrap();
        References::new(&model.assets)
            .unwrap()
            .instance(&mut w, &instance)
            .unwrap();
        let bytes = w.finish().unwrap();
        let mut r = Reader::new(&bytes);
        let assets = read_assets(&mut r).unwrap();
        let decoded = read_instance(&mut r, &assets).unwrap();
        r.end().unwrap();
        assert_ne!(instance.mesh, decoded.mesh);
        instance.mesh = decoded.mesh;
        instance.texture = decoded.texture;
        assert_eq!(instance, decoded);
    }

    #[test]
    fn invalid_mesh_and_mip_records_are_rejected() {
        for case in 0..9 {
            let mut f = Fixture::triangle();
            f.texture(image::ImageFormat::Png);
            let mut model = crate::glb::load_glb(&f.bytes()).unwrap();
            prepare_textures(&mut model.assets).unwrap();
            match case {
                0 => model.assets.meshes[0].indices[0] = u16::MAX,
                1 => model.assets.meshes[0].vertices[0].position[0] = f32::INFINITY,
                2 => {
                    model.assets.meshes[0].indices.pop();
                }
                3 => model.assets.textures[0].width = u32::MAX,
                4 => model.assets.textures[0].height = 0,
                5 => {
                    model.assets.textures[0].rgba.pop();
                }
                6 => model.assets.textures[0].mipmaps.clear(),
                7 => {
                    model.assets.textures[0].mipmaps[0].pop();
                }
                8 => model.assets.textures[0].mipmaps.push(vec![0; 4]),
                _ => unreachable!(),
            }
            let mut w = Writer::new();
            write_assets(&mut w, &model.assets).unwrap();
            let bytes = w.finish().unwrap();
            assert!(
                read_assets(&mut Reader::new(&bytes)).is_err(),
                "case {case}"
            );
        }
    }
}
