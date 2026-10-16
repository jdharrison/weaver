# Curated theater environment assets

Canonical RGB/RGBA outputs are generated offline by `tool/import_environment_assets.py`
and retained in `web/weaver/assets/theater/environment/` with original source provenance.
`weaver-source/examples/first-person-lab/assets/environment/` may retain separately pinned
palette-quantized runtime variants; its manifest records them under `runtime_variant`.
Without a runtime variant, both generated bundles are exact copies.
No scene geometry, poster art, seat boxes, world coordinates, or layout are changed by this tool.

## Reproduce and verify

```sh
python3 tool/import_environment_assets.py --source-dir /path/to/env_integration
python3 tool/import_environment_assets.py --source-dir /path/to/env_integration --check
python3 -m unittest discover -s test -p import_environment_assets_test.py
```

Python 3 and the existing Pillow installation are required; no downloads, network calls,
or extra dependencies. Supply both original ZIPs (names and pinned SHA-256 hashes in
`provenance_manifest.json`). ZIP members are read in memory, not extracted. Check mode
revalidates archive hashes, source members/upstream slice manifests, recipes, original PNG
hashes/dimensions/pixel hashes and docs by regenerating canonical outputs in memory; it writes nothing.
Engine runtime pins separately bind each optimized PNG to its canonical encoded/pixel hashes
and record current encoded size/hash, decoded RGBA8 hash, mode and unchanged dimensions.
Import preserves validated runtime PNGs rather than replacing them with larger originals.
Missing/tampered pins or runtime files fail before either destination is written; divergent
existing engine PNGs without pins also fail closed. Recover missing variants/pins from a
reviewed backup; the unknown optimization recipe is not reproduced or automatically re-pinned.
Import otherwise writes only changed managed files and refuses unknown files or symlinked
destinations. No timestamps or machine-specific source paths are stored.
Canonical PNG generation uses Lanczos, premultiplied-alpha RGBA filtering, compression level 9,
and no source metadata. The manifest records Pillow/zlib versions; encoder changes may
require intentional regeneration/review. The main theater asset manifest is untouched.

## Canonical source-generation / usage contract (layout must remain unchanged)

The recipes, exact pixels/alpha and native decode budgets below describe canonical RGB/RGBA
outputs, not a lossless-equivalence claim for quantized runtime art. Quantization can change
color, alpha and edge pixels; canvas dimensions and geometry/UV contracts remain unchanged.

- Bronze steel: existing walls. The curated sci-fi trim PNG retains its source-door intended use;
  the active engine door uses `assets/textures/door_starlit_cinema.png`, outside this bundle.
  No duplicate `door_starlit_cinema*.png` files belong in either environment directory.
- Lobby stone: existing lobby floor. Gunmetal: existing ceilings.
- Auditorium base: undecorated compass-carpet source crop `[144,144,400,400)` in a
  1254x1254 source. No color adjustment or resampling: TL original 256px crop, TR mirror,
  BL vertical flip, BR both flips, forming a 512px tile. Opposite edges match exactly.
  Crop and resulting tile were visually inspected; neither contains the compass or gold border.
- Compass rug: the **full** original carpet, downsampled to 1024px, for one floor overlay, not a repeating giant compass.
- Sconce, step light, star marker, and curtains: selected source alpha preserved with
  premultiplied-alpha resizing; proportions unchanged to nearest raster pixel.
- Glows: white/amber `(255,244,224)`, `alpha=round(48*max(0,1-r²)^3)`;
  `r²=(2*x/(width-1)-1)^2+(2*y/(height-1)-1)^2`. Fully transparent outer edge;
  max alpha 48/255. Tintable, non-depth-writing translucent sprite approximations,
  **not real dynamic lights**. The floor glow uses the same radial field in a 2:1 raster.

## Seat atlas UVs

1024x1024 RGB; four 512px quadrants. Each **full** source fits within 256x256px
plus 128px edge-extruded gutters on each side. Keep the same five boxes per seat.
Safe UV rectangles below use texel centers, `[u_min,v_min,u_max,v_max]`,
origin top-left, U right, V down. Reverse V only if the consumer's convention requires it.
Each safe rectangle is `[x+128.5,y+128.5,x+383.5,y+383.5]/1024`, with span `255/1024`.
Clamp sampling to these rectangles. With aligned 2x2 box mip generation, every bilinear tap
stays within its source quadrant through **mip levels 0..8**, including all safe UV corners.
This also protects trilinear blends when **both** sampled mip levels are <=8; a blend
involving mip 9 is outside the guarantee. At coarser mips the 256px sampled content region
is subpixel and quadrants may mix. This is a finite bilinear-footprint guarantee, not
protection for arbitrary coarse levels, wider mip-generation kernels, or anisotropic filters.

| Quadrant | Full source | Safe UV rectangle |
| --- | --- | --- |
| TL | `seat_leather_burgundy_alt.png` | `[0.12548828125, 0.12548828125, 0.37451171875, 0.37451171875]` |
| TR | `seat_upholstery_burgundy.png` | `[0.62548828125, 0.12548828125, 0.87451171875, 0.37451171875]` |
| BL | `wall_gunmetal_riveted_panels.png` | `[0.12548828125, 0.62548828125, 0.37451171875, 0.87451171875]` |
| BR | `wall_bronze_steel_panels.png` | `[0.62548828125, 0.62548828125, 0.87451171875, 0.87451171875]` |

## Poster frame openings

Only modular `variant_03_industrial_amber` pieces 01,02,03,06,07,08,09,10,11 are used.
A solid dark-bronze trim bed is overlaid with a cap-free piece-06 rail crop
`[2,96,79,602)`, uniformly resized to border thickness and alternating mirrored repeats.
Full alpha-bounded corners and lamp pieces are uniformly fitted, never stretched along a rail.
All decorations stay within the border; the center contains no poster pixels and is RGBA zero.
One transparent outer guard pixel prevents filtering against an opaque image boundary.

Nominal border is exactly **0.08m** around the existing art on every side. Use the existing
art size and intended outer dimensions below; do not resize/reposition artwork to fit pixels.
Integer PNG dimensions cannot represent every exact rational aspect at a <=1024px budget.
Each axis rounds by <=0.5px; openings round **outward** to avoid covering art. For exact world
bounds, set outer width **and** height explicitly, rather than deriving width from the rounded
raster aspect. If only scaling by outer height is supported, the nominal width has a small
raster-quantization error; do not compensate by changing the existing poster layout.
Exact rational dimensions, nominal subpixel opening, normalized UVs, cleared integer bounds,
all ornament placements, and rail repeat sizes are recorded per frame in the manifest.

| Frame | Existing art (m) | Intended outer size (m) | Raster | Transparent opening `[x0,y0,x1,y1)` |
| --- | --- | --- | --- | --- |
| `poster_frame_01.png` | [2.0, 3.0] | [2.16, 3.16] | 700x1024 | `[25, 25, 675, 999]` |
| `poster_frame_02.png` | [3.6, 2.994392523364486] | [3.76, 3.154392523364486] | 1024x859 | `[21, 21, 1003, 838]` |
| `poster_frame_03.png` | [4.2, 2.3625] | [4.36, 2.5225] | 1024x592 | `[18, 18, 1006, 574]` |
| `poster_frame_04.png` | [4.0, 2.25] | [4.16, 2.41] | 1024x593 | `[19, 19, 1005, 574]` |
| `poster_frame_05.png` | [4.0, 2.250909090909091] | [4.16, 2.410909090909091] | 1024x593 | `[19, 19, 1005, 574]` |
| `poster_frame_06.png` | [4.8, 2.701090909090909] | [4.96, 2.861090909090909] | 1024x591 | `[16, 16, 1008, 575]` |
| `poster_frame_07.png` | [4.8, 2.701090909090909] | [4.96, 2.861090909090909] | 1024x591 | `[16, 16, 1008, 575]` |

## Inventory and budgets

| PNG | Dimensions | Mode | Bytes |
| --- | --- | --- | --- |
| `curtain_left.png` | 272x1024 | RGBA | 384,081 |
| `curtain_right.png` | 272x1024 | RGBA | 382,281 |
| `door_sci_fi_trim_panels.png` | 1024x1024 | RGB | 2,045,275 |
| `ground_auditorium_carpet_base.png` | 512x512 | RGB | 541,881 |
| `ground_compass_rug.png` | 1024x1024 | RGB | 2,288,852 |
| `ground_lobby_stone_tiles.png` | 1024x1024 | RGB | 2,291,526 |
| `poster_frame_01.png` | 700x1024 | RGBA | 122,129 |
| `poster_frame_02.png` | 1024x859 | RGBA | 89,606 |
| `poster_frame_03.png` | 1024x592 | RGBA | 59,492 |
| `poster_frame_04.png` | 1024x593 | RGBA | 64,258 |
| `poster_frame_05.png` | 1024x593 | RGBA | 64,258 |
| `poster_frame_06.png` | 1024x591 | RGBA | 50,956 |
| `poster_frame_07.png` | 1024x591 | RGBA | 50,956 |
| `seat_material_atlas.png` | 1024x1024 | RGB | 517,099 |
| `star_marker_light.png` | 250x256 | RGBA | 118,388 |
| `step_light_amber.png` | 512x166 | RGBA | 178,124 |
| `wall_bronze_steel_panels.png` | 1024x1024 | RGB | 2,194,615 |
| `wall_gunmetal_riveted_panels.png` | 1024x1024 | RGB | 2,105,653 |
| `wall_sconce_weathered_amber.png` | 208x512 | RGBA | 194,004 |
| `warm_floor_glow.png` | 256x128 | RGBA | 3,014 |
| `warm_glow.png` | 128x128 | RGBA | 2,242 |

Canonical/source bundle: **21 PNGs; 13,748,690 encoded bytes; 41,617,408 native RGB/RGBA decoded bytes**.
RGBA8 base-level estimate: 48,171,008 bytes; with all mip levels: 64,215,836 bytes (estimates, not measured GPU allocation).
Palette encoding does not reduce full-resolution RGBA8 decode/GPU storage.
Only curated outputs are shipped, not all pack variants or previews.

## Pinned engine runtime variant

User-optimized palette-quantized engine PNGs; exact optimizer/settings and transformation recipe are not verified.

These palette-quantized variants are **not lossless**; encoded and decoded hashes
are pinned independently. Exact optimizer/settings are unknown. Original `assets`,
`sources`, `archives`, recipes and `totals` above remain canonical-source provenance.

| Runtime PNG | Mode | Bytes |
| --- | --- | --- |
| `curtain_left.png` | P | 138,993 |
| `curtain_right.png` | P | 139,851 |
| `door_sci_fi_trim_panels.png` | P | 848,644 |
| `ground_auditorium_carpet_base.png` | P | 239,043 |
| `ground_compass_rug.png` | P | 985,007 |
| `ground_lobby_stone_tiles.png` | P | 956,870 |
| `poster_frame_01.png` | P | 46,755 |
| `poster_frame_03.png` | P | 24,107 |
| `poster_frame_04.png` | P | 25,642 |
| `poster_frame_05.png` | P | 25,642 |
| `poster_frame_06.png` | P | 20,958 |
| `poster_frame_07.png` | P | 20,958 |
| `seat_material_atlas.png` | P | 222,853 |
| `star_marker_light.png` | P | 36,888 |
| `step_light_amber.png` | P | 52,382 |
| `wall_bronze_steel_panels.png` | P | 939,991 |
| `wall_gunmetal_riveted_panels.png` | P | 897,176 |
| `wall_sconce_weathered_amber.png` | P | 66,037 |

Engine runtime bundle: **21 PNGs; 5,782,659 encoded bytes**.
Files absent from `runtime_variant.assets` are exact canonical copies.
Smaller encoded payloads do not imply a lower decoded RGBA8 memory footprint.

## Provenance and rights

User-supplied Starlit Cinema packs; original archives unchanged. The supplied READMEs describe usage but do not provide an explicit redistribution license. Ownership, licensing, and redistribution rights have not been independently verified; confirm permission before publishing. Generated glow math adds no third-party art.

`provenance_manifest.json` records original archive hashes, selected member hashes,
upstream sheet crop rows, output hashes/dimensions/modes, exact processing, and budgets.
The generated radial glows cite their mathematical recipe instead of an external image.
These assets are albedo/concept art, not PBR, lighting, or emissive-map claims.
