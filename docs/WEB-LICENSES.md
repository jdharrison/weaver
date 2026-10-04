# Reviewed web redistribution notices

This is a bounded engineering inventory for Weaver's default-feature web labs,
not legal advice, a general SPDX resolver, or proof of all redistribution duties.
It does not cover native archives, source distributions, Rust's prebuilt standard
library/compiler/toolchain, or externally supplied assets. Review those separately
when applicable. No new dependency or network license lookup is used.

## Release identity

This inventory accompanies **Weaver 0.2.0** and exact Woven source
**`2bc74d3298b2f317cc00a1508b09b54250c612d8`**: **runtime 0.4.0**, **protocol,
Rust/npm clients 0.3.0**, and **wire version 1 (`WVN1`)**. Both workspaces require
**minimum Rust 1.98**; the verified local/CI toolchain is **1.98.0**. Weaver's source
identity is the commit containing its 0.2.0 manifest and release handoff, recorded
in external release/CI metadata rather than as a self-referential in-file hash.

CI is configured with that exact Woven pin, real sibling checkouts, locked Rust
1.98.0 checks/builds, credential persistence disabled, and a validated web artifact
upload including these legal files. This is configuration evidence, **not a claim
that GitHub Actions ran or that native/live-release gates passed**.

## Inventory and enforcement

`scripts/web-license-inventory.json` records each package/version, manifest license
expression, chosen alternative, target features, source legal filenames, full-source
SHA-256, excerpt byte ranges where necessary, and exact legal bytes. Public legal
texts were extracted from the installed crates.io sources for the locked versions,
not generated from copyright/license templates. Repeated **identical bytes** are
deduplicated; texts with different attribution or formatting remain separate.

`scripts/web-licenses.mjs` runs offline/locked `cargo metadata` and per-lab
`cargo tree --target wasm32-unknown-unknown --edges normal --prefix depth
--no-dedupe --format '{p}|{f}'`. Metadata locates sources and identifies proc-macro
packages; it is **not** used alone to infer the selected graph. The tree chooses
the actual lab/target normal dependencies. Host proc-macro subtrees are removed;
a package reached again through an ordinary target branch is retained. Build/dev
dependencies and build tools such as cfg_aliases/esbuild/TypeScript are excluded.
Default feature names can mention native backends even though the target-selected
normal package graph excludes their platform dependencies.

The review conservatively includes legal notices for normal target libraries even
if a particular function/module is optimized away. It is not a linked-symbol audit.
No native Woven/QUIC/server crates are in the WASM inventory. First-Person's browser
networking is the separate Woven TypeScript client.

Before staging publication, the helper:

1. Compares the actual package/feature graph and manifest license/source identity
   with the reviewed snapshot. Changes require explicit review, not automatic
   licensing inference or silent regeneration.
2. Reads only allowlisted public legal filenames/source excerpts under the locked
   registry package directories. Rejects symlinked files/ancestors, empty/nonregular
   or oversized inputs, and changed full-source/excerpt hashes. No ignored
   credentials, environment files or arbitrary artifact-controlled paths are read.
3. Generates exactly two fixed-name files using exclusive creation:
   `licenses/Rust-INVENTORY.json` and `licenses/Rust-THIRD-PARTY.txt`. The bundle
   includes full exact upstream texts/excerpts, source attribution headers and the
   IJG acknowledgement. No MIT or Apache text is truncated or synthesized.
4. Retains the existing Weaver/Noto/Woven/FlatBuffers regular-file copying,
   runtime allowlists, staged validation, successful-publication replacement and
   rollback. The standalone validator rejects unexpected filenames **before any
   artifact contents are read**, then compares both new files byte-for-byte with
   the trusted repository snapshot. An artifact's own hashes cannot waive a notice
   or authorize an extra filename.

The repository stores the legal bytes as well as their hashes so regression
fixtures and standalone validation do not require installed registry sources.
Production packaging still reads and verifies the real installed sources.

## Reviewed scope and counts

| Web lab | Normal packages | Registry / local | Legal references | Distinct source files | Distinct legal texts |
|---|---:|---:|---:|---:|---:|
| First-Person | 118 | 112 / 6 | 181 | 168 | 63 |
| Render | 114 | 108 / 6 | 177 | 164 | 62 |
| Space | 115 | 109 / 6 | 178 | 165 | 63 |

The union is 121 packages and 64 distinct legal texts. Local packages inherit
Weaver's Apache license, packaged separately; references can share one legal text
or include several legal files for one crate. The First-Person artifact has
**12 files**: the historical 10 runtime/legal files plus the two inventory/bundle
files. Its inventory is 99,153 bytes and its notice bundle is 339,508 bytes.
In the original focused packaging pass, Render/Space inventories were covered
by regression fixtures and only First-Person's complete release artifact was
rebuilt. Prior demo Hosting on port 8002 passed 13 HTTP checks, including exact
served legal texts. This remains historical evidence: no emulator/browser smoke
was newly rerun in the final documentation handoff, and the HTTP checks did not
validate renderer, peers or a live endpoint.

First-Person also includes `@signalweave/woven-client` **0.3.0** with the sole npm
normal dependency `flatbuffers` **25.9.23**, which has no normal/optional dependencies.
Packaging checks these manifests and the FlatBuffers license hash after locked npm
installation. Woven/Woven Client/FlatBuffers full Apache files remain separate;
esbuild's legal comments are retained inline. Their reviewed roots had no
`NOTICE`/`NOTICE.txt`; future notices with those exact names are still copied.

## License choices and non-obvious notices

- **MIT-only / MIT-selected:** exact upstream notices include fontdb 0.16.2,
  rustybuzz 0.14.1, libm, lru, slab, strum, tracing, tracing-core, simd-adler32,
  zmij, and the MIT alternative for byteorder-lite/memchr/termcolor. The rustybuzz
  root license credits **HarfBuzz developers and Evgeniy Reizner**; its generated
  runtime USE table also consumes Microsoft's ms-use data, so that exact
  `scripts/ms-use/COPYING` MIT notice is preserved.
- **Separate vendored MIT:** tracing-core's runtime spin implementation has
  `src/spin/LICENSE`; UUID's enabled WASM `js` WebCrypto branch carries a getrandom
  MIT notice in `src/rng.rs`. Root Apache alternatives do not erase these notices.
- **Combined, not alternative:** dpi 0.1.2 declares `Apache-2.0 AND MIT`; preserve
  `LICENSE` **and** `LICENSE-LIBM-MIT`. unicode-ident 1.0.24 declares
  `(MIT OR Apache-2.0) AND Unicode-3.0`; choose Apache **and** preserve the complete
  `LICENSE-UNICODE`. The downloaded Unicode V3 notice is also retained as
  supplementary provenance for the other generated Unicode tables; this does not
  assert that their crate-level SPDX expressions were changed.
- **Embedded BSD/permissive terms:** retain libm's **whole** composite `LICENSE.txt`
  and 53 exact source-comment excerpts with retained copyrights/conditions,
  including Sun's notice-preservation permission, FreeBSD/BSD terms and CORE-MATH
  attribution. Do not reduce that file to its first MIT block or invent a generic
  ISC notice in place of the actual permissive source terms.
- **BSD alternatives:** choose BSD-3-Clause for moxcms/pxfm and retain their exact
  full `LICENSE.md`. Their source headers also use BSD terms. Other valid
  alternative crates, such as zerocopy, use their declared Apache alternative.
- **W3C:** Winit's public keyboard types and cursor-icon's cursor types retain
  their exact embedded W3C license, short notice, document links and upstream
  derivation/change attribution. Package-level Apache/MIT alternatives alone
  would not reproduce these independent notices.
- **IJG:** image's enabled JPEG module contains an IJG-derived encoder transform.
  Preserve its exact legal block and include: **This software is based in part on
  the work of the Independent JPEG Group.** This is retained conservatively even
  if the particular encoder is eliminated by linking.
- **Zlib:** slotmap/foldhash exact licenses are preserved for provenance and source
  redistribution, without asserting a universal binary-notice condition.
- **Apache choice:** where manifests declare valid `MIT OR Apache-2.0` (or legacy
  slash-separated alternatives), choose Apache and preserve the actual downloaded
  Apache legal text. This includes self_cell's Apache alternative rather than
  GPL. AND expressions and embedded independent terms remain additional duties.
  No root NOTICE was found among the reviewed Rust normal dependency sources.
- **Missing packaged Apache text:** naga 25.0.1 and wgpu-core-deps-wasm 25.0.0
  belong to WGPU's upstream workspace but omit license files from their crate
  archives; svg_fmt 0.4.5 and profiling 1.0.18 also explicitly permit Apache yet
  ship no license text. The inventory documents these exceptions and supplies
  the **existing downloaded WGPU 25.0.2 `LICENSE.APACHE`** terms, not a newly
  authored copyright/license template. No claim is made that these are independent
  upstream NOTICE files.
- **CC0 exception:** hexf-parse 0.2.1's original and normalized manifests declare
  CC0-1.0, but its archive has no legal text. CC0 has no mandatory attribution
  notice; the inventory records the exception instead of inventing a legal file.
- **Separate asset:** bundled Noto Sans's copyright/full OFL remains mandatory and
  separately packaged. fontdb's test-font license, cosmic-text sample emoji data,
  non-distributed tests/examples and host build-only notices are not shipped as
  application assets.

## Review/update and validation

Dependency, target feature, license-expression or legal-byte changes deliberately
fail packaging. Resolve the selected graph again, inspect the new installed
manifests and any root/nested/source notices, revise only the reviewed snapshot,
and rerun focused tests/build/validation. Do not change hashes merely to silence a
failure, scan every lockfile entry as if shipped, or accept an artifact-controlled
filename manifest. No automatic inventory updater is supplied.

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked -p xtask --all-targets -- -D warnings
cargo test --offline --locked -p xtask
CARGO_NET_OFFLINE=true npm_config_offline=true cargo xtask build first-person --platform web
node scripts/check-web-artifact.mjs dist/first-person
```

The 21 xtask tests include 9 Node licensing regressions. Tests cover graph/feature/
license drift, host proc-macro pruning, combined and vendored obligations, each
individual First-Person legal text removed or truncated, inventory omission,
source/artifact symlinks, and forbidden-name rejection before contents are read.

## Final-candidate local validation

The latest comprehensive local pass recorded **168 Rust tests**, **9 Node licensing
regressions**, **45 browser-shell tests** plus typechecking, a locked/offline
workspace build, First-Person desktop/WASM checks and Render/Space web checks.
The offline release web rebuild and final artifact validator passed at **12 files**.
Fresh locked normal target graphs and exact upstream legal bytes matched **all
three snapshots** after the Rust 1.98 MSRV change; no snapshot update was needed,
and all reviewed local package versions remain 0.2.0.

Final confirmation at the exact clean Woven SHA above reran formatting, strict
locked/offline workspace Clippy and **33 focused adapter tests**, all passing.
The comprehensive suite/web build was not repeated by that final confirmation.
Prior demo **13 HTTP checks** and offline Firefox **14 startup checks**
(Firefox 155.0.1, `Weaver ready on Gl`, no page/console errors or external page
requests) are **not newly rerun** results. No live anonymous bootstrap/lobby,
two-peer QA, WebGPU or mobile chat validation is implied. The historical
packaging-only pass and pre-bump baseline remain in
[PORTFOLIO-RELEASE.md](PORTFOLIO-RELEASE.md).

## Native archive inventory — separate redistribution gate

**Do not reuse this WASM bundle as a native archive's complete notice inventory.**
The observed default First-Person **x86_64-unknown-linux-gnu** normal graph has
**287 packages (270 registry / 17 local/path)**, with **178 package/version IDs
absent from the WASM inventory**. It includes Woven runtime 0.4.0 / protocol and
client 0.3.0, native QUIC/TLS, platform/windowing and additional Unicode libraries.
These are graph observations, not a completed native legal-text review.

In particular, native review must not miss ring 0.17.14's
**`Apache-2.0 AND ISC`**, matchit 0.8.4's **`MIT AND BSD-3-Clause`**, ISC-only
libloading/rustls-webpki/untrusted, BSD arrayref/octets/tiny-skia/subtle, ICU Unicode
terms and Wayland/X11 notices. Preserve any nested/vendored notices in their
installed sources, not just a generated list of SPDX expressions.

From Weaver's root, the verified Linux inventory route is:

```sh
cargo metadata --offline --locked --format-version 1 --filter-platform x86_64-unknown-linux-gnu > target/weaver-native-license-metadata.json
cargo tree --offline --locked -p first-person-lab --target x86_64-unknown-linux-gnu --edges normal --prefix depth --no-dedupe --format '{p}|{f}' > target/first-person-native-normal-tree.txt
```

Use the same `normalGraph()` host-proc-macro pruning exported by
`scripts/web-licenses.mjs`, with **native metadata**, to enumerate packages.
Regenerate for each actual released binary/package, target triple and release
feature set; Linux does not cover macOS/Windows. Review the legal texts separately
and package them alongside the binaries with Weaver's full Apache license,
Noto's full copyright/OFL and the relevant local/path Woven licenses/notices.
The native archive publisher must complete that target-specific notice review.
Use the exact Woven SHA above and the Weaver commit containing its 0.2.0 manifest
and release handoff for source metadata; do not substitute moving branch HEADs or
invent Weaver's own hash. Future approved publication pushes Woven first, then
Weaver with its pinned CI dependency. No push/tag/deploy is authorized by this
handoff, and the configured web CI job or local checks are not proof of completed
native licensing, a GitHub run, or live MVP deployment.
