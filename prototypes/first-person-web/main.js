"use strict";

const ROOM_HALF_WIDTH = 6;
const ROOM_HALF_DEPTH = 8;
const ROOM_HEIGHT = 4;
const EYE_HEIGHT = 1.7;
const WALL_CLEARANCE = 0.3;
const MOVE_SPEED = 4;
const MOUSE_SENSITIVITY = 0.0025;
const TOUCH_LOOK_SENSITIVITY = 0.005;

function main() {
  const canvas = document.querySelector("#scene");
  const enterButton = document.querySelector("#enter");
  const instructions = document.querySelector("#instructions");
  const mobileControls = document.querySelector("#mobile-controls");
  const errorOutput = document.querySelector("#error");
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });

  if (!gl) {
    showError("WebGL2 is unavailable in this browser.");
    return;
  }

  try {
    const program = createProgram(gl, VERTEX_SHADER, FRAGMENT_SHADER);
    gl.useProgram(program);
    const room = createRoomMesh(gl);
    const viewProjectionLocation = gl.getUniformLocation(program, "u_view_projection");
    gl.enable(gl.DEPTH_TEST);
    gl.enable(gl.CULL_FACE);
    gl.cullFace(gl.BACK);
    gl.frontFace(gl.CCW);
    gl.clearColor(0.01, 0.01, 0.015, 1);

    const camera = {
      position: [0, EYE_HEIGHT, 5],
      yaw: 0,
      pitch: 0,
    };
    const pressed = new Set();
    const touchMode =
      window.matchMedia("(pointer: coarse)").matches || navigator.maxTouchPoints > 0;
    let previousTime = performance.now();
    let lookPointerId = null;
    let lastLookPosition = [0, 0];

    document.body.classList.toggle("touch-controls", touchMode);

    const enterExperience = () => {
      if (touchMode) {
        instructions.classList.add("hidden");
        mobileControls.hidden = false;
      } else {
        canvas.requestPointerLock();
      }
    };
    canvas.addEventListener("click", () => {
      if (!touchMode) enterExperience();
    });
    enterButton.addEventListener("click", enterExperience);

    document.addEventListener("pointerlockchange", () => {
      if (touchMode) return;
      const captured = document.pointerLockElement === canvas;
      instructions.classList.toggle("hidden", captured);
      if (!captured) clearMovement();
    });

    document.addEventListener("mousemove", (event) => {
      if (document.pointerLockElement !== canvas) return;
      updateLook(camera, event.movementX, event.movementY, MOUSE_SENSITIVITY);
    });

    canvas.addEventListener("pointerdown", (event) => {
      if (!touchMode || event.pointerType === "mouse" || lookPointerId !== null) return;
      event.preventDefault();
      enterExperience();
      lookPointerId = event.pointerId;
      lastLookPosition = [event.clientX, event.clientY];
      canvas.setPointerCapture(event.pointerId);
    });
    canvas.addEventListener("pointermove", (event) => {
      if (event.pointerId !== lookPointerId) return;
      event.preventDefault();
      updateLook(
        camera,
        event.clientX - lastLookPosition[0],
        event.clientY - lastLookPosition[1],
        TOUCH_LOOK_SENSITIVITY,
      );
      lastLookPosition = [event.clientX, event.clientY];
    });
    const stopTouchLook = (event) => {
      if (event.pointerId === lookPointerId) lookPointerId = null;
    };
    canvas.addEventListener("pointerup", stopTouchLook);
    canvas.addEventListener("pointercancel", stopTouchLook);

    for (const button of mobileControls.querySelectorAll("[data-code]")) {
      const code = button.dataset.code;
      button.addEventListener("pointerdown", (event) => {
        event.preventDefault();
        event.stopPropagation();
        button.setPointerCapture(event.pointerId);
        button.classList.add("active");
        pressed.add(code);
      });
      const release = (event) => {
        event.preventDefault();
        button.classList.remove("active");
        pressed.delete(code);
      };
      button.addEventListener("pointerup", release);
      button.addEventListener("pointercancel", release);
      button.addEventListener("lostpointercapture", release);
      button.addEventListener("contextmenu", (event) => event.preventDefault());
    }

    window.addEventListener("keydown", (event) => {
      if (MOVEMENT_CODES.has(event.code)) {
        event.preventDefault();
        pressed.add(event.code);
      }
    });
    window.addEventListener("keyup", (event) => {
      if (MOVEMENT_CODES.has(event.code)) {
        event.preventDefault();
        pressed.delete(event.code);
      }
    });
    window.addEventListener("blur", () => {
      lookPointerId = null;
      clearMovement();
    });

    function clearMovement() {
      pressed.clear();
      for (const button of mobileControls.querySelectorAll(".active")) {
        button.classList.remove("active");
      }
    }

    const render = (now) => {
      const deltaSeconds = Math.min((now - previousTime) / 1000, 0.1);
      previousTime = now;
      updateCamera(camera, pressed, deltaSeconds);
      resizeCanvas(gl, canvas);

      const forward = [
        Math.sin(camera.yaw) * Math.cos(camera.pitch),
        Math.sin(camera.pitch),
        -Math.cos(camera.yaw) * Math.cos(camera.pitch),
      ];
      const target = add(camera.position, forward);
      const projection = perspective(
        (70 * Math.PI) / 180,
        canvas.width / Math.max(canvas.height, 1),
        0.05,
        50,
      );
      const view = lookAt(camera.position, target, [0, 1, 0]);
      const viewProjection = multiply(projection, view);

      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.useProgram(program);
      gl.uniformMatrix4fv(viewProjectionLocation, false, viewProjection);
      gl.bindVertexArray(room.vertexArray);
      gl.drawElements(gl.TRIANGLES, room.indexCount, gl.UNSIGNED_SHORT, 0);
      requestAnimationFrame(render);
    };

    requestAnimationFrame(render);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
  }

  function showError(message) {
    errorOutput.textContent = message;
    errorOutput.hidden = false;
    mobileControls.hidden = true;
    instructions.classList.remove("hidden");
  }
}

const MOVEMENT_CODES = new Set([
  "KeyW",
  "KeyA",
  "KeyS",
  "KeyD",
  "ArrowUp",
  "ArrowLeft",
  "ArrowDown",
  "ArrowRight",
]);

const VERTEX_SHADER = `#version 300 es
in vec3 a_position;
in vec3 a_normal;
in vec3 a_color;
uniform mat4 u_view_projection;
out vec3 v_world_position;
out vec3 v_normal;
out vec3 v_color;

void main() {
  v_world_position = a_position;
  v_normal = a_normal;
  v_color = a_color;
  gl_Position = u_view_projection * vec4(a_position, 1.0);
}
`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
in vec3 v_world_position;
in vec3 v_normal;
in vec3 v_color;
out vec4 out_color;

void main() {
  vec3 normal = normalize(v_normal);
  vec3 absolute_normal = abs(normal);
  vec2 surface_uv;
  if (absolute_normal.y > absolute_normal.x && absolute_normal.y > absolute_normal.z) {
    surface_uv = v_world_position.xz;
  } else if (absolute_normal.x > absolute_normal.z) {
    surface_uv = v_world_position.zy;
  } else {
    surface_uv = v_world_position.xy;
  }

  vec2 cell = abs(fract(surface_uv) - 0.5);
  float grid = smoothstep(0.46, 0.49, max(cell.x, cell.y));
  float diffuse = max(dot(normal, normalize(vec3(0.35, 0.8, -0.25))), 0.0);
  vec3 lit = v_color * (0.72 + diffuse * 0.28);
  out_color = vec4(mix(lit, lit * 0.72, grid), 1.0);
}
`;

function createProgram(gl, vertexSource, fragmentSource) {
  const vertex = compileShader(gl, gl.VERTEX_SHADER, vertexSource);
  const fragment = compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource);
  const program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const details = gl.getProgramInfoLog(program) || "unknown link error";
    gl.deleteProgram(program);
    throw new Error(`Unable to link room shaders: ${details}`);
  }
  return program;
}

function compileShader(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const details = gl.getShaderInfoLog(shader) || "unknown compile error";
    gl.deleteShader(shader);
    throw new Error(`Unable to compile room shader: ${details}`);
  }
  return shader;
}

function createRoomMesh(gl) {
  const x = ROOM_HALF_WIDTH;
  const y = ROOM_HEIGHT;
  const z = ROOM_HALF_DEPTH;
  const vertices = [];
  const indices = [];

  pushQuad(vertices, indices, [
    [-x, 0, -z], [-x, 0, z], [x, 0, z], [x, 0, -z],
  ], [0, 1, 0], [0.48, 0.50, 0.54]);
  pushQuad(vertices, indices, [
    [-x, y, z], [-x, y, -z], [x, y, -z], [x, y, z],
  ], [0, -1, 0], [0.72, 0.73, 0.76]);
  pushQuad(vertices, indices, [
    [-x, 0, -z], [x, 0, -z], [x, y, -z], [-x, y, -z],
  ], [0, 0, 1], [0.55, 0.61, 0.68]);
  pushQuad(vertices, indices, [
    [x, 0, z], [-x, 0, z], [-x, y, z], [x, y, z],
  ], [0, 0, -1], [0.62, 0.57, 0.53]);
  pushQuad(vertices, indices, [
    [-x, 0, z], [-x, 0, -z], [-x, y, -z], [-x, y, z],
  ], [1, 0, 0], [0.50, 0.58, 0.55]);
  pushQuad(vertices, indices, [
    [x, 0, -z], [x, 0, z], [x, y, z], [x, y, -z],
  ], [-1, 0, 0], [0.58, 0.53, 0.62]);

  const vertexArray = gl.createVertexArray();
  gl.bindVertexArray(vertexArray);

  const vertexBuffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(vertices), gl.STATIC_DRAW);

  const indexBuffer = gl.createBuffer();
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint16Array(indices), gl.STATIC_DRAW);

  const stride = 9 * Float32Array.BYTES_PER_ELEMENT;
  bindAttribute(gl, "a_position", 3, stride, 0);
  bindAttribute(gl, "a_normal", 3, stride, 3 * Float32Array.BYTES_PER_ELEMENT);
  bindAttribute(gl, "a_color", 3, stride, 6 * Float32Array.BYTES_PER_ELEMENT);

  return { vertexArray, indexCount: indices.length };

  function bindAttribute(context, name, size, attributeStride, offset) {
    const location = context.getAttribLocation(context.getParameter(context.CURRENT_PROGRAM), name);
    context.enableVertexAttribArray(location);
    context.vertexAttribPointer(location, size, context.FLOAT, false, attributeStride, offset);
  }
}

function pushQuad(vertices, indices, positions, normal, color) {
  const base = vertices.length / 9;
  for (const position of positions) vertices.push(...position, ...normal, ...color);
  indices.push(base, base + 1, base + 2, base, base + 2, base + 3);
}

function updateLook(camera, deltaX, deltaY, sensitivity) {
  camera.yaw += deltaX * sensitivity;
  camera.pitch = clamp(camera.pitch - deltaY * sensitivity, -1.55, 1.55);
}

function updateCamera(camera, pressed, deltaSeconds) {
  let forwardInput = 0;
  let rightInput = 0;
  if (pressed.has("KeyW") || pressed.has("ArrowUp")) forwardInput += 1;
  if (pressed.has("KeyS") || pressed.has("ArrowDown")) forwardInput -= 1;
  if (pressed.has("KeyD") || pressed.has("ArrowRight")) rightInput += 1;
  if (pressed.has("KeyA") || pressed.has("ArrowLeft")) rightInput -= 1;

  const length = Math.hypot(forwardInput, rightInput);
  if (length === 0) return;
  forwardInput /= length;
  rightInput /= length;

  const forward = [Math.sin(camera.yaw), 0, -Math.cos(camera.yaw)];
  const right = [Math.cos(camera.yaw), 0, Math.sin(camera.yaw)];
  camera.position[0] = clamp(
    camera.position[0] +
      (forward[0] * forwardInput + right[0] * rightInput) * MOVE_SPEED * deltaSeconds,
    -ROOM_HALF_WIDTH + WALL_CLEARANCE,
    ROOM_HALF_WIDTH - WALL_CLEARANCE,
  );
  camera.position[2] = clamp(
    camera.position[2] +
      (forward[2] * forwardInput + right[2] * rightInput) * MOVE_SPEED * deltaSeconds,
    -ROOM_HALF_DEPTH + WALL_CLEARANCE,
    ROOM_HALF_DEPTH - WALL_CLEARANCE,
  );
  camera.position[1] = EYE_HEIGHT;
}

function resizeCanvas(gl, canvas) {
  const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.max(1, Math.floor(canvas.clientWidth * pixelRatio));
  const height = Math.max(1, Math.floor(canvas.clientHeight * pixelRatio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
    gl.viewport(0, 0, width, height);
  }
}

function perspective(fieldOfView, aspect, near, far) {
  const f = 1 / Math.tan(fieldOfView / 2);
  const range = 1 / (near - far);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) * range, -1,
    0, 0, 2 * far * near * range, 0,
  ]);
}

function lookAt(eye, target, up) {
  const backward = normalize(subtract(eye, target));
  const right = normalize(cross(up, backward));
  const cameraUp = cross(backward, right);
  return new Float32Array([
    right[0], cameraUp[0], backward[0], 0,
    right[1], cameraUp[1], backward[1], 0,
    right[2], cameraUp[2], backward[2], 0,
    -dot(right, eye), -dot(cameraUp, eye), -dot(backward, eye), 1,
  ]);
}

function multiply(a, b) {
  const output = new Float32Array(16);
  for (let column = 0; column < 4; column += 1) {
    const offset = column * 4;
    for (let row = 0; row < 4; row += 1) {
      output[offset + row] =
        a[row] * b[offset] +
        a[4 + row] * b[offset + 1] +
        a[8 + row] * b[offset + 2] +
        a[12 + row] * b[offset + 3];
    }
  }
  return output;
}

function add(a, b) {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

function subtract(a, b) {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

function cross(a, b) {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

function dot(a, b) {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function normalize(vector) {
  const length = Math.hypot(...vector) || 1;
  return vector.map((component) => component / length);
}

function clamp(value, minimum, maximum) {
  return Math.min(Math.max(value, minimum), maximum);
}

if (typeof document !== "undefined") {
  main();
}

if (typeof module !== "undefined") {
  module.exports = { createRoomMesh, updateCamera, updateLook };
}
