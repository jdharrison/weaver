# Directional avatar stills (runtime assets and source provenance)

Nine stable avatars, six optimized PNGs each, all in their character
subdirectories. Standing fronts use `<character>/avatar_<character>.png`;
back/left and seated filenames are unchanged. The runtime PNGs retain the
original canvas dimensions, but their encodings have already been optimized.
Resolution reduction and decoded-pool memory optimization remain deferred.

**The current indexed-color PNGs are not lossless copies of their sources.**
Decoded RGB and alpha differ in all 54 images, including visible pixels; even
zeroing RGB at alpha zero does not make them equivalent. Quantization shifts
alpha >= 16 bounds by at most one pixel in 23 views. This metadata cleanup does
not modify any artwork, dimensions, or calibration: runtime measurements and
historical source measurements are recorded separately. The exact optimizer and
settings are not established by this inventory.

## Inventory and provenance

- External delivery root:
  `/home/jdharrison/Desktop/portfolio_integration/character_integration/`.
- `standing_left_back_all/<character>/back.png` and `left_side.png` map to the
  same filenames in `<character>/` here (18 files).
- `seated_all/<character>/seated_front.png`, `seated_back.png`, and
  `seated_left_side.png` map to the same filenames here (27 files).
- `metadata/inventory.json` records every current runtime PNG's path, encoded
  `bytes` and `sha256`, dimensions, measured `alpha_range`/`alpha_bounds` (alpha
  >= 16), and decoded `rgba_sha256` (row-major RGBA8, including transparent RGB).
- `source`, `source_sha256`, `source_bytes`, `source_alpha_range`,
  `source_alpha_bounds`, and `source_rgba_sha256` preserve the original source
  provenance separately. The nine original fronts come from the immutable Git
  revision in `source_repository_revision`; the other 45 source paths refer to
  the delivery above. Source hashes were verified before measuring source pixels.
- `optimization.lossless` is explicitly false; different PNG hashes alone would
  not establish a pixel difference, but the independently measured RGBA hashes do.
- `docs/standing_left_back_all_{README.md,manifest.json}` and
  `docs/seated_all_{README.md,manifest.json}` are unchanged copies of the upstream
  references. The two supplied JPEG contact sheets are also copied unchanged for
  visual calibration review; they are not runtime textures.

**All nine seated-manifest standing-front reference hashes differ from the nine
pre-optimization repository fronts.** `front_reference_comparison` retains both
historical hashes and original paths; these are source comparisons, not current
runtime paths/hashes. An encoded-hash mismatch does not by itself establish
whether those upstream reference images depict different pixels. The upstream
reference manifests were not rewritten to conceal the discrepancy.

All 54 original encoded hashes were verified against available originals (nine
Git blobs and 45 delivery files). Their combined size was **115,849,956 bytes**;
the current runtime PNGs total **21,913,252 bytes** (about **20.9 MiB**), an 81.1%
encoded-size reduction. Fronts remain 1086×1448; other views remain 1024×1536.
This does not reduce the full-resolution decoded RGBA footprint.

`npc_concession_robot.png` is unrelated and outside the 54-texture avatar inventory.

## Runtime order and directional convention

The flattened pool is avatar 01 through 09, each with:

1. standing front (optimized copy of the original repository PNG),
2. standing back,
3. standing left,
4. seated front,
5. seated back,
6. seated left.

`AvatarTextures` exposes two `[front, back, left]` arrays. Four horizontal
view sectors use body-forward versus the direction from logical eye to viewer;
45-degree ties favor front/back. Body left is `Y × forward`. Right samples left
with UV `[1, 0, -1, 1]`, rather than introducing unapproved right-side artwork.
Seated body-forward is always chair/screen `-Z`, independent of head/free look.
Zero/vertical viewer displacement falls back to body-forward (front view);
invalid/zero standing forward falls back to `-Z`. The quad separately follows
viewer yaw, not body yaw, and does not pitch toward the camera.

## Calibration: full canvas, visible floor anchor

`metadata/views.json` is in exactly the runtime pool order, with paths matching
the current PNG locations. Its calibration remains in original source pixels,
with top-left origin, X right, Y down, and right/bottom-exclusive bounds.
`alpha_bounds` is the source bounding box of pixels with alpha **>= 16**, not
all nonzero-alpha fringes/shadows or the quantized runtime measurements. Each view records:

- original canvas `dimensions`, unaltered;
- measured alpha bounds and threshold;
- `floor_anchor_px`: X is the mean of occupied columns in the bottom 5% of the
  alpha-bound height (the visible feet), Y is the exclusive bound bottom;
- actual full-alpha `visible_height` in world meters (may differ across seated
  views because painted leg lengths differ);
- seated-only `head_above_seat`: the identical per-identity cushion-to-crown
  target stored in each of its three views, consumed directly by the runtime;
- seated-only `boot_floor_offset` in meters: positive floats above the floor,
  negative penetrates it; measured boot baseline, not a hanging tail;
- seated-only `seat_anchor_px`: posterior thigh/pelvis **undersurface contact**
  with a cushion, not the waist, belt, torso midpoint, or hanging coat hem;
- `seat_anchor_method` and a per-view note distinguishing visible contours from
  contacts inferred behind coats/front thighs;
- `previous_visible_height` for seated views, retaining the pre-seat-fit height
  for comparison rather than silently rewriting the calibration history.

The aviary rear view is an exception to using alpha-bound bottom as foot Y:
its feather tail extends to pixel 1472 but both boots end at pixel 1418. Its
`floor_anchor_note` records the boot-only measurement regions.

Standing visible heights remain 2.16–2.21 m, around the existing 2.21 m
presentation. The initial 1.70–1.75 m seated heights put the actual underside of
many long-shin poses above a 0.48 m chair cushion. The subsequent exact-boot fit
made crowns change height when selecting views. Seated placement now prioritizes
an exact cushion contact and **stable per-identity cushion-to-crown height**.
X/Y still use the same pixel scale: calibration does not introduce changes to
painted proportions, nonuniform stretching, cropping, or resampling.

Logical standing eye remains floor + 1.7 m and seated eye floor + 1.25 m; this
module never changes physics, network poses, or labels (eye + .56 m). Headgear
and antenna tips define visible height. Art tops stay below the existing label
anchors. Independent painted perspectives do not permit all four constraints at
once: unchanged proportions, exact cushion placement, identical crown heights,
and exact boot-floor placement. The first three take priority; boot-floor error
is minimized across views and bounded to **±0.069 m** (6.9 cm, including numeric
margin). Actual largest magnitude is **0.068237705 m**, tortoise front/back.
This is an explicit near-ground tolerance, not a claim of exact grounding.

All UVs sample the **entire canvas**; calibration affects scale/placement only.
Standing canvas world height is
`visible_height * canvas_height / alpha_bound_height`.
For each seated view first measure the exact-boot-fit cushion-to-crown height:

`t_i = 0.48 * (seat_anchor_y - alpha_top_y) / (boot_y - seat_anchor_y)`.

For each identity choose the harmonic mean of its extreme heights:

`T = 2 * min(t_i) * max(t_i) / (min(t_i) + max(t_i))`.

This minimizes the maximum absolute boot-floor deviation
`0.48 * (1 - T / t_i)` across its three views. The shared `T` is stored as
`head_above_seat`; no per-frame cross-view computation or texture-pool scan is
needed to calculate the target. Seated canvas world height is
`head_above_seat * canvas_height / (seat_anchor_y - alpha_top_y)`.
The renderer's existing world-sprite aspect correction preserves the original
image aspect ratio.

For seated placement, the local reference uses **seat-anchor X and Y**. Its
transformed position is subtracted from chair floor + 0.48 m to obtain
canvas-center translation. Consequently pelvis contact X is centered over the
chair, pelvis contact Y is exactly floor + 0.48 m, and crown Y is exactly floor +
0.48 m + the shared `head_above_seat`. Boot Y may be slightly above/below floor
within the stated bound; boot X retains its painted forward offset relative to
the pelvis. Mirrored right views reflect pelvis/foot pixel X offsets and UVs
consistently.
Standing placement continues to anchor the visible foot in both X and Y.

Rock is caller-supplied **radians**, not a clock or amplitude multiplier. Standing
finite values are safety-clamped to ±0.18 radians; non-finite rock becomes zero.
**Seated rock is always forced to zero**, even for a malformed caller. Standing
rotation is `yaw * local_Z_roll`; translation compensates the rotated foot point
because generic WGPU world sprites ignore `SpriteInstance.pivot`. No motion
history or locomotion state is held here.

### Seated support calibration and remaining limitations

All 27 individual seated PNGs and the supplied contact sheet were visually
inspected for this pass. The earlier advisory `approximate_hip_px` landmarks
were removed: they described a higher body point, not a cushion contact, and
were not used at runtime. Their replacements are runtime seat/floor constraints.
Front/rear X follows the pelvis midline rather than the feet, backpack, bags, or
silhouette midpoint; left X follows the posterior thigh beneath the torso.

Support X/Y landmarks remain unchanged in `metadata/views.json`. The table
reports each identity's stable crown above chair floor and its boot-floor
offsets in meters (negative = below floor), in **front/back/left** order.
The posterior contact is partially hidden by the coat in several profiles and by
thighs in frontal views; those rows are manual support-surface estimates, not
claims of authored pivots or reconstructed anatomy. Visible support contours,
nearby trouser seams, and the corresponding side view guide those estimates.
Per-view metadata records the occlusion explicitly.

| Avatar | Shared crown Y (m) | Front boot offset | Back boot offset | Left/right boot offset |
| --- | ---: | ---: | ---: | ---: |
| 01 Rust drifter | 1.372250 | -0.022745 | +0.010043 | +0.022745 |
| 02 Chrome usher | 1.328594 | -0.029156 | -0.018088 | +0.029156 |
| 03 Moth mechanic | 1.366931 | -0.042533 | -0.015187 | +0.042533 |
| 04 Velvet bones | 1.364389 | -0.041415 | +0.041415 | +0.006824 |
| 05 Spore scout | 1.589327 | -0.039706 | +0.039706 | +0.009143 |
| 06 Canid courier | 1.512161 | -0.053212 | +0.053212 | +0.002908 |
| 07 Puddle kid | 1.643530 | -0.000538 | -0.016788 | +0.016788 |
| 08 Aviary pilot | 1.444425 | -0.067802 | +0.067802 | -0.009456 |
| 09 Tortoise miner | 1.555389 | -0.068238 | +0.068238 | -0.047876 |

The preventable pelvis-to-chair sideways displacement is corrected rather than
left for QA. Both cushion placement and crown height now remain fixed through
view changes. Visual QA can refine occluded support landmarks and assess painted
perspective, hanging clothes, the accepted small boot-floor offsets, and
chair-back intersection; a single flat yaw billboard cannot reproduce a full
volumetric seated body. A landmark refinement should recompute the identity's
shared minimax target rather than restoring independent per-view head heights.

## Validation and resource caveat

The original import verified all 45 delivery/source byte sequences. Current
runtime files intentionally have different encoded and decoded hashes; source
hashes remain anchored to the unchanged delivery manifests and historical Git
fronts. All 54 source/runtime pairs were rechecked for this metadata cleanup;
none are pixel-identical. Original calibration is preserved, not silently
recomputed from the quantized artwork.

The avatar Rust tests cover runtime paths/byte counts, measured alpha ranges and
bounds, unchanged source calibration with at most one-pixel runtime fringe
shifts, original source hashes against delivery/reference manifests,
transparency, handle layout/uniqueness, stable IDs,
four directional sectors and fallback, mirrored UVs, seated look independence,
full-canvas sampling, finite transforms, calibrated heights, compensated standing
foot rotation, seated rock rejection, and all 27 seated pelvis contacts centered
at cushion height, invariant per-identity crown heights, the harmonic minimax
target, and bounded boot-floor offsets (including mirrored views). Only the
inventory test creates a full cloned decoded pool; geometry tests use synthetic
handles and the small calibration metadata.

The static `OnceLock` decoded pool and returned deep clones intentionally retain
the existing API behavior. At these original dimensions one decoded RGBA pool is
roughly 324 MiB, before clones/GPU allocations. The smaller encoded payload does
not imply lower decoded memory usage or a browser performance guarantee.

Run focused Rust tests from the Weaver root:

```sh
cargo test --offline -p first-person-lab --lib avatars::tests::
```

The current encoded and decoded SHA-256 pins can be checked independently with
the existing Python/Pillow installation, also from the Weaver root:

```sh
python3 -B - <<'PY'
import hashlib
import io
import json
from pathlib import Path
from PIL import Image

root = Path("examples/first-person-lab/assets/avatars")
assets = json.loads((root / "metadata/inventory.json").read_text())["assets"]
for asset in assets:
    png = (root / asset["path"]).read_bytes()
    image = Image.open(io.BytesIO(png)).convert("RGBA")
    assert len(png) == asset["bytes"], asset["path"]
    assert hashlib.sha256(png).hexdigest() == asset["sha256"], asset["path"]
    assert hashlib.sha256(image.tobytes()).hexdigest() == asset["rgba_sha256"], asset["path"]
print(f"Verified {len(assets)} encoded PNG and decoded RGBA fingerprints")
PY
```
