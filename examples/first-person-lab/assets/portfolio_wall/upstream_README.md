# Starlit Cinema — Portfolio Wall Asset Pack v1

This is an implementation-focused separation of the **approved Starlit Cinema portfolio contact plaque**. All output is PNG/SVG/JSON; no font binaries included.

## Fastest integration
1. Apply `billboard/board_assembled_no_qr.png` on a flat plane using RGBA blending; it includes the frame, recreated clean surface, graphics, and **blank QR mounts**.
2. To use the lamps as additional flat planes, position `lighting/left_tube_lamp.png`, `right_tube_lamp.png`, and `top_marquee_light.png`. Or use `billboard/wide_billboard_with_lighting_no_qr.png` as one flat texture if you prefer.
3. Generate your actual GitHub and LinkedIn QR codes from final URLs in code. Overlay in the blank mounts using the inset bounds in `layout_manifest.json`.
4. Overlay clickable invisible hit rectangles for email, Github logo/mount and LinkedIn logo/mount. `layout_manifest.json` gives coordinates relative to board texture.
5. For modular assembly, start from `billboard/board_blank.png`, then add separate icons, QR mounts, captions, rules and ornaments.

## Contents
- `billboard/`: blank framed sign, fully assembled sign **without QR data**, wide sign with lamps
- `frame/`: ornamental bevel overlay; optional cropped wall support columns
- `textures/`: dark board background as independent albedo/texture
- `icons/`: source-derived GitHub and LinkedIn icon plates
- `qr_mounts/`: 2 **blank** QR frames with no code; small blank paper inset
- `typography/`: raster and scalable text assets
- `lines/`: modular gold divider strokes as PNG/SVG
- `ornaments/`: compass rose pieces as transparent sprites
- `lighting/`: wall lamps, top light, rough emissive helpers
- `layout_manifest.json`: explicit fixed pixel layout and click regions
- `FONT_GUIDE.md`: closest font matches; not exact font identification

## Important production limitations
- The approved input is a **single flattened generated image**, not separate editable layers. Literal extraction of unobstructed icon plates, metal frame and lamps is possible. The inner backing and typography have been **reconstructed** to make them reconfigurable. These layers are a close visual approximation, not an exact recreation of the original source pixels.
- QR content from the generated concept is intentionally **not exported**. The artwork's original QR patterns should not be trusted as scannable.
- The emissive helpers are color/luminance-derived **starting masks**, not physically authored emissive maps.
- The RGB/alpha isolation of inset lights and ornaments is conservative but should be reviewed in-engine for edge fringes.

## Coordinates
The root source canvas is 1536×1024; the framed board crops to x=270..1267, y=102..824, resolution **997×722**. All element anchors in `layout_manifest.json` are relative to this framed board.

## Suggested runtime
This board can be displayed on a flat in-world mesh in Flutter/custom WebGL engine. Use a single 997×722 base texture, then add QR rectangles as separate quads or pre-compose at runtime. Alpha blending is required for the standalone ornaments and lamps.
