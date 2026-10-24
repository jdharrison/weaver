# Bundled default font

`NotoSans-Regular.ttf` and `NotoSans-Bold.ttf` are unmodified Noto Sans
Regular (400) and Bold (700) version 2.004, copyright
2015 Google LLC, distributed under the SIL Open Font License 1.1 in `OFL.txt`.
Upstream: <https://github.com/notofonts/latin-greek-cyrillic>.

The checked-in file was imported from Debian's `fonts-noto-core` package
(`/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf`). Its SHA-256 is
`89c3c497f618fdaa0b2d1e98fef93582f28c71debd2c4a8cdf41f190ced2909d`.

The bold face was imported from the same Debian package at
`/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf`. Its SHA-256 is
`e83493c945848ecd4a9ad0f6d19164541a0d3e23a9c952304a00a46e00272ac5`.

The renderer embeds both faces into desktop and WASM binaries and selects Noto Sans as
the default sans-serif font for both shaping and measurement. Browser WASM has
no system font database, so relying on installed fonts would panic at runtime.
The font requires no HTTP download. Native system fonts remain available for
fallback; these bundled faces do not promise complete Unicode coverage.
`TextRun` defaults to regular; bold runs select the real bold face during shaping
on desktop, WebGPU, and WebGL2, rather than synthesizing duplicate glyphs.

Web lab builds include the license at `licenses/NotoSans-OFL.txt`. When
redistributing native binaries, include `OFL.txt` alongside them.
