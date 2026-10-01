"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { createRoomMesh, updateCamera, updateLook } = require("./main.js");

test("room mesh builds all six inward-facing surfaces", () => {
  const uploaded = [];
  const gl = {
    ARRAY_BUFFER: 1,
    ELEMENT_ARRAY_BUFFER: 2,
    STATIC_DRAW: 3,
    CURRENT_PROGRAM: 4,
    FLOAT: 5,
    createVertexArray: () => ({}),
    bindVertexArray: () => {},
    createBuffer: () => ({}),
    bindBuffer: () => {},
    bufferData: (target, data) => uploaded.push([target, data]),
    getParameter: () => ({}),
    getAttribLocation: (_program, name) => ({ a_position: 0, a_normal: 1, a_color: 2 })[name],
    enableVertexAttribArray: () => {},
    vertexAttribPointer: () => {},
  };

  const room = createRoomMesh(gl);
  const vertexUpload = uploaded.find(([target]) => target === gl.ARRAY_BUFFER);
  const indexUpload = uploaded.find(([target]) => target === gl.ELEMENT_ARRAY_BUFFER);

  assert.equal(room.indexCount, 36);
  assert.equal(vertexUpload[1].length, 24 * 9);
  assert.equal(indexUpload[1].length, 36);
});

test("camera movement advances and remains inside the room", () => {
  const camera = { position: [0, 1.7, 5], yaw: 0, pitch: 0 };
  updateCamera(camera, new Set(["KeyW"]), 1);
  assert.deepEqual(camera.position, [0, 1.7, 1]);

  updateCamera(camera, new Set(["ArrowUp"]), 10);
  assert.deepEqual(camera.position, [0, 1.7, -7.7]);
});

test("touch-look deltas rotate and clamp the camera", () => {
  const camera = { position: [0, 1.7, 5], yaw: 0, pitch: 0 };
  updateLook(camera, 100, -1000, 0.005);
  assert.equal(camera.yaw, 0.5);
  assert.equal(camera.pitch, 1.55);
});
