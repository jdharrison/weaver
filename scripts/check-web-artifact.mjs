#!/usr/bin/env node
// Usage: node scripts/check-web-artifact.mjs <artifact-directory> [first-person|render|space]
// Checks files without loading/evaluating the application or making network requests.
import { lstatSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { checkRustLicenses, RUST_LICENSE_FILES } from "./web-licenses.mjs";

const LABS = {
  "first-person": "first_person_lab",
  render: "render_lab",
  space: "space_lab",
};
const COMMON_LICENSES = ["licenses/Weaver-LICENSE.txt", "licenses/NotoSans-OFL.txt", ...RUST_LICENSE_FILES];
const CLIENT_LICENSES = ["licenses/Woven-LICENSE.txt", "licenses/Woven-Client-LICENSE.txt", "licenses/FlatBuffers-LICENSE.txt"];
const NOTICE_LABELS = ["Weaver", "Woven", "Woven-Client", "FlatBuffers"];
const decoder = new TextDecoder("utf-8", { fatal: true });

function fail(message) {
  throw new Error(message);
}

function rejectSymlinkAncestors(directory) {
  const parsed = path.parse(directory);
  let current = parsed.root;
  for (const component of directory.slice(parsed.root.length).split(path.sep).filter(Boolean)) {
    current = path.join(current, component);
    if (lstatSync(current).isSymbolicLink()) fail(`symlinks are forbidden: ${current}`);
  }
}

function text(buffer, name) {
  const value = decoder.decode(buffer);
  if (value.trim().length === 0 || value.includes("\0")) fail(`empty or invalid text: ${name}`);
  return value;
}

function checkStylesheet(source) {
  const stack = [];
  let quote = null;
  let comment = false;
  for (let index = 0; index < source.length; index++) {
    const character = source[index];
    if (comment) {
      if (character === "*" && source[index + 1] === "/") {
        comment = false;
        index++;
      }
    } else if (quote !== null) {
      if (character === "\\") index++;
      else if (character === quote) quote = null;
    } else if (character === "/" && source[index + 1] === "*") {
      comment = true;
      index++;
    } else if (character === "\"" || character === "'") {
      quote = character;
    } else if ("{([".includes(character)) {
      stack.push(character);
    } else if ("})]".includes(character)) {
      if (stack.pop() !== { "}": "{", ")": "(", "]": "[" }[character]) {
        fail("incomplete stylesheet: styles.css");
      }
    }
  }
  if (comment || quote !== null || stack.length !== 0) fail("incomplete stylesheet: styles.css");
  const withoutComments = source.replace(/\/\*[\s\S]*?\*\//g, "").trimEnd();
  if (!withoutComments.endsWith("}")) fail("incomplete stylesheet: styles.css");
}

function assetPath(from, specifier, moduleImport = false) {
  if (moduleImport && !specifier.startsWith("./") && !specifier.startsWith("../")) {
    fail(`unbundled or external module import in ${from}: ${specifier}`);
  }
  if (/[:?#\\]/.test(specifier) || specifier.startsWith("/")) {
    fail(`non-relative runtime asset in ${from}: ${specifier}`);
  }
  const resolved = path.posix.normalize(path.posix.join(path.posix.dirname(from), specifier));
  if (resolved === ".." || resolved.startsWith("../")) fail(`asset escapes artifact: ${specifier}`);
  return resolved;
}

// SourceTextModule parses real ES modules and reports static imports, without
// evaluating them. It is isolated in a child so the public CLI needs no VM flag.
const PARSE_MODULES = `
import { readFileSync } from 'node:fs';
import { SourceTextModule } from 'node:vm';
const inputs = JSON.parse(readFileSync(0, 'utf8'));
const dependencies = Object.fromEntries(inputs.map(([name, source]) => [
  name, new SourceTextModule(source, { identifier: name }).dependencySpecifiers
]));
process.stdout.write(JSON.stringify(dependencies));
`;

export function checkWebArtifact(directory, lab = "first-person") {
  if (!Object.hasOwn(LABS, lab)) fail(`unsupported lab: ${lab}`);
  const root = path.resolve(directory);
  rejectSymlinkAncestors(root);
  if (!lstatSync(root).isDirectory()) fail(`artifact is not a directory: ${root}`);
  const stem = LABS[lab];
  const glue = `pkg/${stem}.js`;
  const wasm = `pkg/${stem}_bg.wasm`;
  const required = new Set(["index.html", "styles.css", glue, wasm, ...COMMON_LICENSES]);
  if (lab === "first-person") {
    required.add("main.js");
    for (const license of CLIENT_LICENSES) required.add(license);
  }
  const labels = lab === "first-person" ? NOTICE_LABELS : ["Weaver"];
  const optional = new Set(labels.flatMap((label) => [
    `licenses/${label}-NOTICE.txt`, `licenses/${label}-NOTICE-text.txt`,
  ]));
  const files = new Set();

  function walk(relative = "") {
    const names = readdirSync(path.join(root, relative)).sort();
    if (relative && names.length === 0) fail(`unexpected empty artifact directory: ${relative}`);
    for (const name of names) {
      const entry = relative ? `${relative}/${name}` : name;
      const metadata = lstatSync(path.join(root, entry));
      if (metadata.isSymbolicLink()) fail(`symlinks are forbidden: ${entry}`);
      if (metadata.isDirectory()) {
        const allowed = entry === "pkg" || entry === "licenses"
          || /^pkg\/snippets(?:\/[A-Za-z0-9_-]+)*$/.test(entry);
        if (!allowed) fail(`unexpected artifact directory: ${entry}`);
        walk(entry);
      } else {
        // Check names before reading anything, including accidentally placed credentials.
        const snippet = /^pkg\/snippets\/(?:[A-Za-z0-9_-]+\/)+[A-Za-z0-9_-]+\.js$/.test(entry);
        if (!required.has(entry) && !optional.has(entry) && !snippet) {
          fail(`unexpected artifact file: ${entry}`);
        }
        if (!metadata.isFile() || metadata.size === 0) fail(`expected a nonempty regular file: ${entry}`);
        files.add(entry);
      }
    }
  }
  walk();
  for (const name of required) if (!files.has(name)) fail(`missing required artifact: ${name}`);
  checkRustLicenses(root, lab);

  const content = new Map();
  for (const name of files) {
    if (name !== wasm) content.set(name, text(readFileSync(path.join(root, name)), name));
  }
  const html = content.get("index.html");
  if (!/^\s*<!doctype html>/i.test(html) || !/<\/body>\s*<\/html>\s*$/i.test(html)) {
    fail("incomplete HTML document: index.html");
  }
  checkStylesheet(content.get("styles.css"));
  for (const name of ["licenses/Weaver-LICENSE.txt", ...(lab === "first-person" ? CLIENT_LICENSES : [])]) {
    if (!content.get(name).includes("END OF TERMS AND CONDITIONS")) {
      fail(`incomplete Apache license text: ${name}`);
    }
  }
  if (!content.get("licenses/NotoSans-OFL.txt").includes("DISCLAIMER")
      || !content.get("licenses/NotoSans-OFL.txt").trimEnd().endsWith("OTHER DEALINGS IN THE FONT SOFTWARE.")) {
    fail("incomplete font license: licenses/NotoSans-OFL.txt");
  }

  const bytes = readFileSync(path.join(root, wasm));
  if (!WebAssembly.validate(bytes)) fail(`invalid or truncated WebAssembly: ${wasm}`);
  const wasmExports = WebAssembly.Module.exports(new WebAssembly.Module(bytes));
  if (!wasmExports.some((entry) => entry.kind === "memory")
      || !wasmExports.some((entry) => entry.kind === "function")) {
    fail(`WebAssembly lacks runtime exports: ${wasm}`);
  }

  const modules = new Map([...content].filter(([name]) => name.endsWith(".js")));
  const htmlRoots = new Set();
  const assetReferences = new Set();
  for (const match of html.matchAll(/<(?:script|link)\b[^>]*\b(?:src|href)\s*=\s*["']([^"']+)["'][^>]*>/gi)) {
    if (match[1] === "data:,") continue;
    const target = assetPath("index.html", match[1]);
    if (!files.has(target)) fail(`HTML references missing asset: ${target}`);
    assetReferences.add(target);
    if (target.endsWith(".js")) htmlRoots.add(target);
  }
  let inlineIndex = 0;
  for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    if (/\bsrc\s*=/.test(match[1])) continue;
    if (!/\btype\s*=\s*["']module["']/.test(match[1])) fail("unexpected non-module inline script");
    const name = `index.html#module-${inlineIndex++}`;
    modules.set(name, match[2]);
    htmlRoots.add(name);
  }
  if (!assetReferences.has("styles.css")) fail("index.html does not reference styles.css");
  if (lab === "first-person" && !htmlRoots.has("main.js")) fail("index.html does not load main.js");

  const parsed = spawnSync(process.execPath, ["--experimental-vm-modules", "--input-type=module", "-e", PARSE_MODULES], {
    input: JSON.stringify([...modules]), encoding: "utf8", maxBuffer: 4 * 1024 * 1024, timeout: 30_000,
  });
  if (parsed.error) throw parsed.error;
  if (parsed.status !== 0) fail(`invalid or truncated JavaScript module: ${parsed.stderr.trim()}`);
  const dependencies = JSON.parse(parsed.stdout);
  const reached = new Set();
  function visit(name) {
    if (reached.has(name)) return;
    if (!modules.has(name)) fail(`missing imported module: ${name}`);
    reached.add(name);
    const from = name.startsWith("index.html#") ? "index.html" : name;
    const specifiers = [...dependencies[name]];
    // wasm-bindgen currently uses static imports; also cover literal dynamic imports.
    for (const match of modules.get(name).matchAll(/\bimport\(\s*["']([^"']+)["']\s*\)/g)) specifiers.push(match[1]);
    for (const specifier of specifiers) visit(assetPath(from, specifier, true));
  }
  for (const name of htmlRoots) visit(name);
  if (!reached.has(glue)) fail(`HTML module graph does not reach ${glue}`);
  for (const name of files) {
    if (name.endsWith(".js") && !reached.has(name)) fail(`unreferenced runtime module: ${name}`);
  }
  const wasmReferences = [...content.get(glue).matchAll(/new URL\(\s*["']([^"']+)["']\s*,\s*import\.meta\.url\s*\)/g)]
    .map((match) => assetPath(glue, match[1]));
  if (!wasmReferences.includes(wasm)) fail(`${glue} does not reference ${wasm}`);
  for (const name of wasmReferences) if (!files.has(name)) fail(`glue references missing asset: ${name}`);

  return { directory: root, lab, files: files.size };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (args.length < 1 || args.length > 2) fail("usage: node scripts/check-web-artifact.mjs <artifact-directory> [first-person|render|space]");
    const result = checkWebArtifact(args[0], args[1]);
    console.log(`Validated ${result.lab} web artifact: ${result.files} files at ${result.directory}`);
  } catch (error) {
    console.error(`Web artifact validation failed: ${error.message}`);
    process.exitCode = 1;
  }
}
