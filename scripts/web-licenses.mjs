#!/usr/bin/env node
// A reviewed snapshot for the default web labs, not an SPDX policy engine.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const RUST_LICENSE_FILES = ["licenses/Rust-INVENTORY.json", "licenses/Rust-THIRD-PARTY.txt"];
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const catalogPath = path.join(root, "scripts/web-license-inventory.json");
const labs = { "first-person": "first-person-lab", render: "render-lab", space: "space-lab" };
const target = "wasm32-unknown-unknown";
const decoder = new TextDecoder("utf-8", { fatal: true });
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const json = (value) => Buffer.from(`${JSON.stringify(value, null, 2)}\n`);

export function regularBytes(filename) {
  const absolute = path.resolve(filename);
  let current = path.parse(absolute).root;
  for (const component of absolute.slice(current.length).split(path.sep).filter(Boolean)) {
    current = path.join(current, component);
    if (lstatSync(current).isSymbolicLink()) throw new Error(`license input symlink: ${current}`);
  }
  const stat = lstatSync(absolute);
  if (!stat.isFile() || stat.size === 0 || stat.size > 4 * 1024 * 1024) {
    throw new Error(`expected bounded nonempty regular license input: ${absolute}`);
  }
  return readFileSync(absolute);
}

export function loadCatalog() {
  const catalog = JSON.parse(decoder.decode(regularBytes(catalogPath)));
  if (catalog.schema !== 1 || catalog.target !== target) throw new Error("unsupported reviewed license inventory");
  for (const [hash, source] of Object.entries(catalog.texts)) {
    if (sha256(Buffer.from(source)) !== hash) throw new Error(`reviewed notice hash mismatch: ${hash}`);
  }
  return catalog;
}

// Normal edges alone still contain host proc-macro dependencies. Skip their
// subtrees, but retain a package if another ordinary target branch reaches it.
export function normalGraph(tree, packages) {
  const byKey = new Map(packages.map((pkg) => [`${pkg.name}@${pkg.version}`, pkg]));
  const found = new Map();
  let skippedDepth = null;
  for (const line of tree.trimEnd().split("\n")) {
    const match = /^(\d+)([A-Za-z0-9_-]+) v([^\s|]+)(?:[^|]*)\|([^|]*)$/.exec(line);
    if (!match) throw new Error(`unrecognized Cargo tree record: ${line}`);
    const depth = Number(match[1]);
    if (skippedDepth !== null && depth > skippedDepth) continue;
    skippedDepth = null;
    const key = `${match[2]}@${match[3]}`;
    const pkg = byKey.get(key);
    if (!pkg) throw new Error(`Cargo metadata missing ${key}`);
    if (pkg.targets.some((entry) => entry.kind.includes("proc-macro"))) {
      skippedDepth = depth;
      continue;
    }
    found.set(key, { package: key, features: match[4].split(",").filter(Boolean).sort() });
  }
  return [...found.values()].sort((a, b) => a.package.localeCompare(b.package, "en"));
}

export function checkGraph(graph, catalog, lab, packages) {
  if (!Object.hasOwn(labs, lab)) throw new Error(`unsupported license lab: ${lab}`);
  if (JSON.stringify(graph) !== JSON.stringify(catalog.graphs[lab])) {
    throw new Error(`normal ${target} dependency/features drift for ${lab}; review web-license-inventory.json`);
  }
  const byKey = new Map(packages.map((pkg) => [`${pkg.name}@${pkg.version}`, pkg]));
  for (const { package: key } of graph) {
    const actual = byKey.get(key);
    const reviewed = catalog.packages[key];
    if (!reviewed || actual.license !== reviewed.license || actual.source !== reviewed.source) {
      throw new Error(`reviewed license expression/source drift: ${key}`);
    }
  }
}

function selectedInventory(catalog, lab) {
  if (!Object.hasOwn(labs, lab)) throw new Error(`unsupported license lab: ${lab}`);
  const graph = catalog.graphs[lab];
  return {
    schema: 1, target, lab, scope: catalog.scope,
    packages: graph.map(({ package: key, features }) => ({ package: key, features, ...catalog.packages[key] })),
  };
}

export function noticeArtifacts(lab, catalog = loadCatalog(), texts = catalog.texts) {
  const inventory = selectedInventory(catalog, lab);
  const references = new Map();
  for (const pkg of inventory.packages) {
    for (const notice of pkg.notices) {
      if (!references.has(notice.sha256)) references.set(notice.sha256, []);
      references.get(notice.sha256).push(`${pkg.package}: ${notice.package}/${notice.file}${notice.range ? ` bytes ${notice.range.join("..")}` : ""}`);
    }
  }
  const acknowledgements = inventory.packages.filter((pkg) => pkg.acknowledgement)
    .map((pkg) => `${pkg.package}: ${pkg.acknowledgement}\n`).join("");
  const chunks = [Buffer.from(`Rust third-party notices — ${lab} / ${target}\n\nExact upstream legal texts and source-comment excerpts follow.\nIdentical legal bytes are included once; all package/source references are listed.\nScope and license choices are recorded in Rust-INVENTORY.json.\n\n${acknowledgements}\n`)];
  for (const [hash, refs] of [...references].sort(([a], [b]) => a.localeCompare(b))) {
    const bytes = Buffer.from(texts[hash] ?? "");
    if (sha256(bytes) !== hash) throw new Error(`missing or changed declared notice: ${hash}`);
    chunks.push(Buffer.from(`===== SHA256 ${hash} =====\n${refs.sort().join("\n")}\n\n`), bytes, Buffer.from("\n\n"));
  }
  return new Map([[RUST_LICENSE_FILES[0], json(inventory)], [RUST_LICENSE_FILES[1], Buffer.concat(chunks)]]);
}

// Only legal filenames and reviewed public source excerpts can be read. Source
// roots come from Cargo's locked registry metadata, never from the artifact.
function allowedSource(file) {
  return ["LICENSE", "LICENSE.txt", "LICENSE.md", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-APACHE.md", "LICENSE.APACHE", "LICENSE-LIBM-MIT", "LICENSE-UNICODE", "COPYING", "NOTICE", "NOTICE.txt"].includes(file)
    || ["src/spin/LICENSE", "scripts/ms-use/COPYING", "src/rng.rs", "src/keyboard.rs", "src/lib.rs", "src/codecs/jpeg/transform.rs"].includes(file)
    || /^src\/math\/[a-z0-9_]+\.rs$/.test(file);
}

export function readReviewedNotice(notice, packages) {
  if (!allowedSource(notice.file)) throw new Error(`forbidden license source filename: ${notice.file}`);
  const pkg = packages.find((entry) => `${entry.name}@${entry.version}` === notice.package);
  if (!pkg || pkg.source !== "registry+https://github.com/rust-lang/crates.io-index") {
    throw new Error(`notice source is not a locked crates.io package: ${notice.package}`);
  }
  const bytes = regularBytes(path.join(path.dirname(pkg.manifest_path), notice.file));
  if (sha256(bytes) !== notice.sourceSha256) throw new Error(`upstream legal source drift: ${notice.package}/${notice.file}`);
  const excerpt = notice.range ? bytes.subarray(...notice.range) : bytes;
  if (excerpt.length !== notice.bytes || sha256(excerpt) !== notice.sha256) {
    throw new Error(`upstream legal text drift: ${notice.package}/${notice.file}`);
  }
  decoder.decode(excerpt);
  return excerpt;
}

function cargo(args) {
  return execFileSync("cargo", args, { cwd: root, encoding: "utf8", timeout: 60_000, maxBuffer: 32 * 1024 * 1024 });
}

export function stageRustLicenses(lab, directory) {
  if (!Object.hasOwn(labs, lab)) throw new Error(`unsupported license lab: ${lab}`);
  const catalog = loadCatalog();
  const metadata = JSON.parse(cargo(["metadata", "--offline", "--locked", "--format-version", "1", "--filter-platform", target]));
  const tree = cargo(["tree", "--offline", "--locked", "-p", labs[lab], "--target", target, "--edges", "normal", "--prefix", "depth", "--no-dedupe", "--format", "{p}|{f}"]);
  const graph = normalGraph(tree, metadata.packages);
  checkGraph(graph, catalog, lab, metadata.packages);
  const texts = {};
  let sources = 0;
  for (const { package: key } of graph) {
    for (const notice of catalog.packages[key].notices) {
      texts[notice.sha256] = decoder.decode(readReviewedNotice(notice, metadata.packages));
      sources++;
    }
  }
  const output = path.resolve(directory);
  // Inspect ancestors without reading a directory as a file.
  regularBytes(path.join(output, "licenses/Weaver-LICENSE.txt"));
  for (const [name, bytes] of noticeArtifacts(lab, catalog, texts)) {
    writeFileSync(path.join(output, name), bytes, { flag: "wx" });
  }
  console.log(`Packaged ${graph.length} normal WASM packages (${graph.filter(({ package: key }) => catalog.packages[key].source).length} registry), ${sources} legal source references, ${Object.keys(texts).length} distinct legal texts`);
}

export function checkRustLicenses(directory, lab) {
  // Caller must reject unexpected artifact filenames before invoking this.
  for (const [name, expected] of noticeArtifacts(lab)) {
    const actual = regularBytes(path.join(directory, name));
    if (!actual.equals(expected)) throw new Error(`missing, truncated or changed declared license inventory/notices: ${name}`);
  }
}

export function checkClientLicenses(directory) {
  const reviewed = loadCatalog().client;
  const woven = path.join(root, "../woven/crates/woven-client-ts");
  const client = JSON.parse(decoder.decode(regularBytes(path.join(woven, "package.json"))));
  const flatbuffers = JSON.parse(decoder.decode(regularBytes(path.join(woven, "node_modules/flatbuffers/package.json"))));
  const shell = JSON.parse(decoder.decode(regularBytes(path.join(root, "examples/first-person-lab/platforms/web/package.json"))));
  if (client.name !== reviewed.name || client.version !== reviewed.version || client.license !== "Apache-2.0"
      || JSON.stringify(client.dependencies) !== JSON.stringify(reviewed.dependencies)
      || flatbuffers.version !== reviewed.dependencies.flatbuffers || flatbuffers.license !== "Apache-2.0"
      || Object.keys(flatbuffers.dependencies ?? {}).length || Object.keys(flatbuffers.optionalDependencies ?? {}).length
      || Object.keys(client.optionalDependencies ?? {}).length
      || JSON.stringify(shell.dependencies) !== JSON.stringify({ "@signalweave/woven-client": "file:../../../../../woven/crates/woven-client-ts" })) {
    throw new Error("browser normal dependency/license inventory drift; review web-license-inventory.json");
  }
  if (sha256(regularBytes(path.join(directory, "licenses/FlatBuffers-LICENSE.txt"))) !== reviewed.flatbuffersLicenseSha256) {
    throw new Error("FlatBuffers legal text drift");
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [lab, directory, ...extra] = process.argv.slice(2);
    if (!directory || extra.length) throw new Error("usage: node scripts/web-licenses.mjs <first-person|render|space> <staged-artifact>");
    stageRustLicenses(lab, directory);
    if (lab === "first-person") checkClientLicenses(directory);
  } catch (error) {
    console.error(`Third-party licensing failed: ${error.message}`);
    process.exitCode = 1;
  }
}
