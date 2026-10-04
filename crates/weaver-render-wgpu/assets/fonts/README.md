# Bundled default font

`NotoSans-Regular.ttf` is unmodified Noto Sans Regular version 2.004, copyright
2015 Google LLC, distributed under the SIL Open Font License 1.1 in `OFL.txt`.
Upstream: <https://github.com/notofonts/latin-greek-cyrillic>.

The checked-in file was imported from Debian's `fonts-noto-core` package
(`/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf`). Its SHA-256 is
`89c3c497f618fdaa0b2d1e98fef93582f28c71debd2c4a8cdf41f190ced2909d`.

The renderer embeds this face into desktop and WASM binaries and selects it as
the default sans-serif font for both shaping and measurement. Browser WASM has
no system font database, so relying on installed fonts would panic at runtime.
The font requires no HTTP download. Native system fonts remain available for
fallback; this single bundled face does not promise complete Unicode coverage.

Web lab builds include the license at `licenses/NotoSans-OFL.txt`. When
redistributing native binaries, include `OFL.txt` alongside them.
