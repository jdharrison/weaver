# Portfolio wall — reproducible offline composition

Runtime textures: `portfolio_board.png` (997×722) and `portfolio_wall.png`
(1536×870, includes lighting). `portfolio_preview.png` shows both at native size.
Original `*_no_qr.png`, SVGs, layout, documentation and ZIP remain byte-for-byte intact.

```sh
python3 tool/import_portfolio_wall_assets.py --source-dir web/weaver/assets/theater/portfolio_wall
python3 tool/import_portfolio_wall_assets.py --source-dir web/weaver/assets/theater/portfolio_wall --check
python3 -B -m unittest discover -s test -p import_portfolio_wall_assets_test.py
```

Initial import also accepts `/home/jdharrison/Downloads`, containing the pinned ZIP
and both supplied SVGs. The pinned system DejaVu font/license is used only if absent
from source-dir. Subsequent imports need no Downloads, network or system font.
Both bundles contain every input including the full source ZIP, font and license.
Do not ship archival inputs to clients unless needed; runtime uses only one texture.

## Rendering and geometry

The exact supplied nested SVGs are hash-validated. Data rects and standard nested
finder shapes are reconstructed on a binary grid, not generated from the captions.
This is deliberately not a general SVG renderer. Four-module white quiet zones and
integer 3-pixel modules are used, centered in the exact manifest mount insets:
GitHub [293,383,113,113], LinkedIn [754,383,111,113] (board x/y/w/h).
The 25×25 and 29×29 data grids retain crisp black/white pixels with no resampling
other than integer nearest-neighbor enlargement. Runtime filtering/distance may
still affect scanning. Decoded destinations are not asserted by this importer.

Pixel comparison verifies the wide board offset is [270,100], NOT the reference
canvas crop [270,102]. Both output variants contain exactly the same edited board.
The importer first proves blank-board + manifest components reconstructs the
original exactly, then omits old captions/rules and draws the replacements.
No solid-color erasure is painted over textured backgrounds. Other artwork and
wide lighting remain unchanged. Caption strips are [107,519,431,558] and
[569,519,890,558]; original rule sprite pixels are shortened around each URL.

Captions are `github.com/jdharrison` and `linkedin.com/in/jacobdharrison`.
Inter was unavailable locally (checked with fc-list). DejaVu Sans Regular 15px is
an intentional readable substitute, bundled under `DejaVuSans-LICENSE.txt`.
Original typography elsewhere is untouched. The original layout JSON still has
upstream placeholder URLs; runtime links are managed separately by the scene.

## Provenance and checks

`provenance_manifest.json` records SHA-256/size for all original and derived files,
source archive members, supplied SVGs, font/license, importer, renderer versions,
and exact geometry. No timestamps or absolute local source paths are embedded.
`--check` writes nothing and rejects missing/stale/tampered files in either copy.
Unknown files and destination symlinks are rejected, never removed.
Byte-identical PNG reproduction requires the recorded Pillow/FreeType/zlib versions;
font bytes and BASIC text layout are pinned to avoid system font/shaping variation.
No downloads, external SVG renderer, QR encoder or new dependencies are needed.

## Rights

User-supplied Starlit Cinema portfolio wall pack and QR SVGs; original bytes preserved. The supplied documentation does not provide an explicit redistribution license. Ownership, licensing, and redistribution rights have not been independently verified; confirm permission before publishing. DejaVu font licensing is preserved separately.
