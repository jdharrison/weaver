import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, symlinkSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  checkGraph, checkRustLicenses, loadCatalog, normalGraph, noticeArtifacts,
  readReviewedNotice, regularBytes, RUST_LICENSE_FILES, sha256,
} from "./web-licenses.mjs";
import { checkWebArtifact } from "./check-web-artifact.mjs";

const catalog = loadCatalog();
const temporary = (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "weaver-license-test-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(path.join(root, "licenses"));
  return root;
};
const fixture = (root, lab) => {
  for (const [name, bytes] of noticeArtifacts(lab)) writeFileSync(path.join(root, name), bytes);
};
const metadata = Object.entries(catalog.packages).map(([key, pkg]) => {
  const [name, version] = key.split("@");
  return { name, version, license: pkg.license, source: pkg.source };
});

test("normal target graph prunes host proc-macro subtrees, retaining ordinary repeated reachability", () => {
  const packages = ["app", "derive", "host-only", "shared"].map((name) => ({ name, version: "1.0.0", targets: [{ kind: [name === "derive" ? "proc-macro" : "lib"] }] }));
  const graph = normalGraph("0app v1.0.0 (/public/app)|default\n1derive v1.0.0 (proc-macro)|\n2host-only v1.0.0|\n2shared v1.0.0|\n1shared v1.0.0|std\n", packages);
  assert.deepEqual(graph, [{ package: "app@1.0.0", features: ["default"] }, { package: "shared@1.0.0", features: ["std"] }]);
  assert.throws(() => normalGraph("not a tree", packages), /unrecognized/);
});

test("locked package, target feature, SPDX expression and registry source drift fail closed", () => {
  const graph = catalog.graphs["first-person"];
  checkGraph(graph, catalog, "first-person", metadata);
  assert.throws(() => checkGraph(graph.slice(1), catalog, "first-person", metadata), /dependency\/features drift/);
  const features = structuredClone(graph);
  features[0].features.push("unreviewed");
  assert.throws(() => checkGraph(features, catalog, "first-person", metadata), /dependency\/features drift/);
  for (const field of ["license", "source"]) {
    const changed = structuredClone(metadata);
    changed.find((pkg) => pkg.name === "fontdb")[field] = "unreviewed";
    assert.throws(() => checkGraph(graph, catalog, "first-person", changed), /expression\/source drift/);
  }
});

test("all three labs have exact declared inventory and complete legal bytes", (t) => {
  const root = temporary(t);
  for (const lab of ["first-person", "render", "space"]) {
    fixture(root, lab);
    checkRustLicenses(root, lab);
    const artifacts = noticeArtifacts(lab);
    const inventory = JSON.parse(artifacts.get(RUST_LICENSE_FILES[0]));
    assert.equal(inventory.packages.length, catalog.graphs[lab].length);
    for (const pkg of inventory.packages) for (const notice of pkg.notices) {
      assert.equal(Buffer.byteLength(catalog.texts[notice.sha256]), notice.bytes);
    }
  }
});

test("every distinct declared upstream notice is individually required, including its final byte", (t) => {
  const root = temporary(t);
  fixture(root, "first-person");
  const complete = noticeArtifacts("first-person").get(RUST_LICENSE_FILES[1]);
  const hashes = new Set(catalog.graphs["first-person"].flatMap(({ package: key }) => catalog.packages[key].notices.map((n) => n.sha256)));
  for (const hash of hashes) {
    const bytes = Buffer.from(catalog.texts[hash]);
    const marker = Buffer.from(`===== SHA256 ${hash} =====\n`);
    const offset = complete.indexOf(bytes, complete.indexOf(marker) + marker.length);
    assert.ok(offset >= 0);
    for (const removed of [bytes.length, 1]) {
      const cut = offset + bytes.length - removed;
      writeFileSync(path.join(root, RUST_LICENSE_FILES[1]), Buffer.concat([complete.subarray(0, cut), complete.subarray(cut + removed)]));
      assert.throws(() => checkRustLicenses(root, "first-person"), /declared license inventory\/notices/);
    }
  }
});

test("declared inventory cannot omit an obligation or self-authorize additional filenames", (t) => {
  const root = temporary(t);
  fixture(root, "first-person");
  const inventory = JSON.parse(noticeArtifacts("first-person").get(RUST_LICENSE_FILES[0]));
  inventory.packages.find((pkg) => pkg.package === "fontdb@0.16.2").notices = [];
  inventory.extraFiles = ["private-token"];
  writeFileSync(path.join(root, RUST_LICENSE_FILES[0]), JSON.stringify(inventory));
  assert.throws(() => checkRustLicenses(root, "first-person"), /declared license inventory\/notices/);
  rmSync(path.join(root, RUST_LICENSE_FILES[0]));
  assert.throws(() => checkRustLicenses(root, "first-person"), /ENOENT/);
  fixture(root, "first-person");
  writeFileSync(path.join(root, RUST_LICENSE_FILES[1]), "");
  assert.throws(() => checkRustLicenses(root, "first-person"), /nonempty regular/);
});

test("reviewed mandatory and supplementary notices are explicit, not inferred from OR expressions", () => {
  for (const [key, files] of [
    ["fontdb@0.16.2", ["LICENSE"]],
    ["rustybuzz@0.14.1", ["LICENSE", "scripts/ms-use/COPYING"]],
    ["dpi@0.1.2", ["LICENSE", "LICENSE-LIBM-MIT"]],
    ["unicode-ident@1.0.24", ["LICENSE-APACHE", "LICENSE-UNICODE"]],
    ["tracing-core@0.1.36", ["LICENSE", "src/spin/LICENSE"]],
    ["uuid@1.26.1", ["LICENSE-APACHE", "src/rng.rs"]],
    ["winit@0.30.13", ["LICENSE", "src/keyboard.rs"]],
    ["cursor-icon@1.2.0", ["LICENSE-APACHE", "src/lib.rs"]],
    ["image@0.25.10", ["LICENSE-APACHE", "src/codecs/jpeg/transform.rs"]],
  ]) {
    for (const file of files) assert.ok(catalog.packages[key].notices.some((notice) => notice.file === file), `${key}/${file}`);
  }
  assert.equal(catalog.packages["dpi@0.1.2"].selected, "Apache-2.0 AND MIT");
  assert.equal(catalog.packages["unicode-ident@1.0.24"].selected, "Apache-2.0 AND Unicode-3.0");
  assert.match(catalog.texts[catalog.packages["rustybuzz@0.14.1"].notices[0].sha256], /HarfBuzz/);
  const libm = catalog.packages["libm@0.2.16"];
  assert.ok(libm.notices.some((n) => catalog.texts[n.sha256].includes("Redistribution and use in source and binary forms")));
  assert.ok(libm.notices.some((n) => catalog.texts[n.sha256].includes("provided that this notice")));
  const bundle = noticeArtifacts("first-person").get(RUST_LICENSE_FILES[1]).toString();
  assert.match(bundle, /This software is based in part on the work of the Independent JPEG Group\./);
  assert.equal(catalog.packages["hexf-parse@0.2.1"].notices.length, 0);
});

test("only bounded regular public legal inputs are read; drift and private filename requests fail", (t) => {
  const root = temporary(t);
  const bytes = Buffer.from("synthetic legal fixture\n");
  writeFileSync(path.join(root, "LICENSE"), bytes);
  const pkg = { name: "fixture", version: "1", source: "registry+https://github.com/rust-lang/crates.io-index", manifest_path: path.join(root, "Cargo.toml") };
  const notice = { package: "fixture@1", file: "LICENSE", sourceSha256: sha256(bytes), bytes: bytes.length, sha256: sha256(bytes) };
  assert.deepEqual(readReviewedNotice(notice, [pkg]), bytes);
  assert.throws(() => readReviewedNotice({ ...notice, file: "../private-token" }, [pkg]), /forbidden license source filename/);
  assert.throws(() => readReviewedNotice(notice, [{ ...pkg, source: null }]), /not a locked crates.io/);
  writeFileSync(path.join(root, "LICENSE"), bytes.subarray(0, bytes.length - 1));
  assert.throws(() => readReviewedNotice(notice, [pkg]), /legal source drift/);
  assert.throws(() => regularBytes(path.join(root, "licenses")), /regular license input/);
});

test("source, artifact legal file and ancestor symlinks are rejected", (t) => {
  const root = temporary(t);
  fixture(root, "first-person");
  const filename = path.join(root, RUST_LICENSE_FILES[1]);
  rmSync(filename);
  writeFileSync(path.join(root, "public-fixture"), "synthetic legal fixture");
  symlinkSync(path.join(root, "public-fixture"), filename);
  assert.throws(() => checkRustLicenses(root, "first-person"), /symlink/);
  symlinkSync(path.join(root, "licenses"), path.join(root, "linked"));
  assert.throws(() => regularBytes(path.join(root, "linked/Rust-INVENTORY.json")), /symlink/);
});

test("unexpected artifact names are rejected before invalid contents or declared inventory are read", (t) => {
  const root = temporary(t);
  writeFileSync(path.join(root, RUST_LICENSE_FILES[0]), Buffer.from([0xff]));
  writeFileSync(path.join(root, "woven.local-token"), Buffer.from([0xff]));
  assert.throws(() => checkWebArtifact(root), /unexpected artifact file: woven.local-token/);
});
