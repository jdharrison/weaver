import assert from "node:assert/strict";
import test from "node:test";
import { decodeApplicationPayload, MAX_APPLICATION_MESSAGE_BYTES } from "./application-payload.ts";

const encoder = new TextEncoder();

test("accepts UTF-8 up to the exact application byte ceiling", () => {
  const text = "é".repeat(MAX_APPLICATION_MESSAGE_BYTES / 2);
  assert.equal(decodeApplicationPayload(encoder.encode(text)), text);
  assert.equal(decodeApplicationPayload(new Uint8Array()), "");
});

test("drops oversized messages by bytes, including otherwise valid UTF-8", () => {
  assert.equal(decodeApplicationPayload(encoder.encode("x".repeat(2049))), null);
  assert.equal(decodeApplicationPayload(encoder.encode("é".repeat(1025))), null);
});

test("malformed UTF-8 drops only that message and does not poison the decoder", () => {
  for (const bytes of [[0xff], [0xc3], [0xc0, 0xaf], [0xed, 0xa0, 0x80]]) {
    assert.equal(decodeApplicationPayload(new Uint8Array(bytes)), null);
    assert.equal(decodeApplicationPayload(encoder.encode('{"kind":"profile"}')), '{"kind":"profile"}');
  }
});
