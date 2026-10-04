// Run through the local-only Firebase Hosting emulator; never accepts a remote URL.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { checkWebArtifact } from "./check-web-artifact.mjs";

checkWebArtifact("dist/first-person");
const config = JSON.parse(readFileSync("firebase.json", "utf8"));
assert.equal(config.hosting.target, "portfolio");
assert.equal(config.hosting.public, "dist/first-person");
assert.equal(config.hosting.rewrites, undefined, "missing runtime assets must not become the room HTML");
const origin = "http://127.0.0.1:8002";
const paths = ["/", "/main.js", "/styles.css", "/pkg/first_person_lab.js", "/pkg/first_person_lab_bg.wasm", "/licenses/Weaver-LICENSE.txt", "/licenses/Rust-INVENTORY.json", "/licenses/Rust-THIRD-PARTY.txt"];
for (const resource of paths) {
  const response = await fetch(`${origin}${resource}`, { signal: AbortSignal.timeout(10_000) });
  assert.equal(response.status, 200, `${resource}: expected 200`);
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  assert.equal(response.headers.get("referrer-policy"), "no-referrer");
  assert.equal(response.headers.get("x-frame-options"), "DENY");
  assert.match(response.headers.get("cache-control") ?? "", /no-cache/);
  assert.match(response.headers.get("cache-control") ?? "", /must-revalidate/);
  assert.match(response.headers.get("content-security-policy-report-only") ?? "", /wasm-unsafe-eval/);
  if (resource.endsWith(".wasm")) {
    assert.equal(response.headers.get("content-type")?.split(";")[0], "application/wasm");
    assert.ok(WebAssembly.validate(await response.arrayBuffer()), "served WASM must be valid");
  } else {
    const body = await response.text();
    assert.ok(body.length > 0, `${resource}: expected nonempty body`);
    if (resource.startsWith("/licenses/")) {
      assert.equal(body, readFileSync(`dist/first-person${resource}`, "utf8"), `${resource}: served legal bytes must be complete`);
    }
    if (resource === "/") assert.match(response.headers.get("content-type") ?? "", /text\/html/);
    if (resource.endsWith(".js")) assert.match(response.headers.get("content-type") ?? "", /(?:javascript|ecmascript)/);
    if (resource.endsWith(".css")) assert.match(response.headers.get("content-type") ?? "", /text\/css/);
  }
  console.log(`PASS ${resource}: runtime asset and release headers`);
}
for (const resource of ["/missing.js", "/pkg/missing.wasm", "/woven.local-token", "/main.ts", "/api/first-person-lobby"]) {
  const response = await fetch(`${origin}${resource}`, { signal: AbortSignal.timeout(10_000) });
  assert.equal(response.status, 404, `${resource}: expected 404, not SPA fallback or credentials`);
  await response.arrayBuffer();
  console.log(`PASS ${resource}: 404`);
}
console.log("Local Firebase Hosting smoke test passed. No renderer, peer, or live endpoint validation is implied.");
