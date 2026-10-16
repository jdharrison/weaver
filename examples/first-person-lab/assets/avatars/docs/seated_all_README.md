# Starlit Cinema — Seated Avatar Stills

Nine characters, three seated stills each: **27 individual 1024 × 1536 RGBA PNGs**. Version 2 adds the nine front-facing seated stills; the approved left and rear PNGs remain byte-for-byte unchanged.

## Files

The original character folder names are retained. Every folder contains:

- `seated_front.png`: front-facing seated pose, using the supplied original front still and approved seated profile as references.
- `seated_left_side.png`: seated left-facing profile, matching the supplied left view.
- `seated_back.png`: seated rear view, matching the supplied rear character design.

The pack also contains `preview_all_seated.jpg`, `preview_seated_front.jpg`, `manifest.json`, and `generation_prompts.json`. This README is included in the ZIP.

## Artwork

Generated with the built-in image-generation tool using the uploaded standing PNGs as identity references. Characters sit with bent knees and hands on their thighs. Original outfits, packs, wings, tails, and other character features are retained as visual references. These are newly generated poses, so fine details and framing may vary from the standing art and between views.

Every PNG contains its generated alpha channel. There are no baked-in chairs or theater scenery. The pale background in the overview is for inspection only. Final PNGs are byte-for-byte copies of the generated files; no color-key extraction, resampling, or destructive trimming was applied. Load the alpha channel when rendering; RGB values beneath transparent pixels are not visible artwork.

## Theater placement

1. Load the seated still for the avatar and view. Draw with PNG alpha blending.
2. Scale the seated character so its head and torso match the standing character's apparent size. Do not reuse the standing billboard height just because the source canvas dimensions are identical: these seated compositions fill their own canvases.
3. Position the hips over your seat cushion and the boots at floor height. Set a per-character seated offset and scale after checking against your actual theater chair; no calibrated engine pivots are supplied.
4. Use your chair geometry or sprite layers to occlude the lower back and legs as appropriate. Allow space for backpacks, the shell, wings, and tail feathers.

These are static pose assets. They do not include sit/stand transition frames or right-side views. A horizontal flip can be used as a placeholder right view if mirrored costume details are acceptable.

## Validation

All 27 images were checked for RGBA format, 1024 × 1536 dimensions, substantial transparent background, and visible seated poses. `manifest.json` records image hashes, source-reference hashes, alpha ranges, and content bounding boxes at alpha >= 16. Bounding boxes are metadata only; the PNGs are untrimmed. The assets have not been tested inside the theater engine.
