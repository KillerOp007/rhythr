// crates/gui/ui/kit/turntable-math.js
// Pure maths for the turned view: an orbit camera about the note's centre
// (the game's frame: x right, y up, +z toward the player), column-major
// 4x4 matrices for WebGL, and the baked mesh expanded into flat triangles
// with one normal per face, so relief shades without any smoothing across
// the seams the kernel keeps sharp. No DOM here, so it is testable.

// The game only ever shows a note from the front; the turn is a look
// around the corner, not a free orbit.
export const ORBIT_LIMIT = 60;
export const DEFAULT_DISTANCE = 4.5;
export const ZOOM_MIN = 2.5;
export const ZOOM_MAX = 9;
export const FOV_DEGREES = 35;
export const THREE_QUARTER = { yaw: 35, pitch: 25 };

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

export function clampOrbit(o) {
  return {
    yaw: clamp(o.yaw, -ORBIT_LIMIT, ORBIT_LIMIT),
    pitch: clamp(o.pitch, -ORBIT_LIMIT, ORBIT_LIMIT),
    dist: clamp(o.dist, ZOOM_MIN, ZOOM_MAX),
  };
}

// The eye for an orbit: yaw turns about y (positive looks from the right),
// pitch lifts the eye (positive looks from above).
export function orbitEye(o) {
  const y = (o.yaw * Math.PI) / 180;
  const p = (o.pitch * Math.PI) / 180;
  return [o.dist * Math.sin(y) * Math.cos(p), o.dist * Math.sin(p), o.dist * Math.cos(y) * Math.cos(p)];
}

export function perspective(fovDeg, aspect, near, far) {
  const f = 1 / Math.tan((fovDeg * Math.PI) / 360);
  const nf = 1 / (near - far);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) * nf;
  m[11] = -1;
  m[14] = 2 * far * near * nf;
  return m;
}

const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
function normalize(v) {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
}

export function lookAt(eye, target, up) {
  const z = normalize(sub(eye, target));
  const x = normalize(cross(up, z));
  const y = cross(z, x);
  const m = new Float32Array(16);
  m[0] = x[0]; m[4] = x[1]; m[8] = x[2]; m[12] = -dot(x, eye);
  m[1] = y[0]; m[5] = y[1]; m[9] = y[2]; m[13] = -dot(y, eye);
  m[2] = z[0]; m[6] = z[1]; m[10] = z[2]; m[14] = -dot(z, eye);
  m[15] = 1;
  return m;
}

export function multiply(a, b) {
  const m = new Float32Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      m[c * 4 + r] = s;
    }
  }
  return m;
}

export function transformPoint(m, p) {
  const x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
  const y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
  const z = m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14];
  const w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15] || 1;
  return [x / w, y / w, z / w];
}

// The view's rotation as a 3x3 for normals (the view has no scale).
export function normalMatrix(view) {
  return new Float32Array([view[0], view[1], view[2], view[4], view[5], view[6], view[8], view[9], view[10]]);
}

// The camera matrices for an orbit over a canvas of the given aspect.
export function cameraMatrices(o, aspect) {
  const view = lookAt(orbitEye(o), [0, 0, 0], [0, 1, 0]);
  const proj = perspective(FOV_DEGREES, aspect || 1, 0.1, 50);
  return { view, proj, mvp: multiply(proj, view), nrm: normalMatrix(view) };
}

// Indexed triangles to flat triangles with a face normal per vertex. The
// vertex at position i of the output belongs to index i of the input, so
// the kernel's index ranges (solid, glass) apply unchanged.
export function expandFaces(positions, indices) {
  const n = Math.floor(indices.length / 3);
  const pos = new Float32Array(n * 9);
  const nrm = new Float32Array(n * 9);
  for (let t = 0; t < n; t++) {
    const a = positions[indices[t * 3]];
    const b = positions[indices[t * 3 + 1]];
    const c = positions[indices[t * 3 + 2]];
    const fn = normalize(cross(sub(b, a), sub(c, a)));
    for (let k = 0; k < 3; k++) {
      const v = [a, b, c][k];
      pos.set(v, t * 9 + k * 3);
      nrm.set(fn, t * 9 + k * 3);
    }
  }
  return { positions: pos, normals: nrm, triangles: n };
}

// The note box as line segments at the hit plane (z 0), for orientation.
export function boxLines() {
  const c = [[-1, -1, 0], [1, -1, 0], [1, 1, 0], [-1, 1, 0]];
  const out = [];
  for (let i = 0; i < 4; i++) out.push(...c[i], ...c[(i + 1) % 4]);
  return new Float32Array(out);
}
