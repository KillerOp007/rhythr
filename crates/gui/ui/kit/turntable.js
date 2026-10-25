// crates/gui/ui/kit/turntable.js
// The turned view: the baked note (kit_mesh, the same triangles the export
// writes) drawn with WebGL2 from an orbit camera, so depth work can be
// judged before the game shows it through parallax. Inspection only: no
// editing happens here. Off, it costs nothing (no calls, no drawing).
//
// The game draws notes unlit in one tint, so "Shaded" is a help for the
// eye (a headlight on the face normals) and can be switched off to see the
// flat truth; "Fade" draws the note translucent as the approach fade does,
// where overlapping parts brighten.
import { coalesced } from "./ipc.js";
import { clampOrbit, cameraMatrices, expandFaces, boxLines, DEFAULT_DISTANCE, THREE_QUARTER } from "./turntable-math.js";

export const TURNTABLE_KEY = "rhythr.kit.turntable";
export const GLASS_ALPHA = 0.57;
export const DEFAULT_NOTE = "Turned view of the baked note: drag turns it (60 degrees each way), wheel zooms. The game draws the note flat; its relief shows only on the approach.";
const NO_WEBGL = "The turned view needs WebGL 2, which this system does not provide.";
const DRAG_DEGREES_PER_PX = 0.5;

const VERT = `#version 300 es
in vec3 a_pos;
in vec3 a_nrm;
uniform mat4 u_mvp;
uniform mat3 u_nrm;
out vec3 v_nrm;
void main() {
  v_nrm = u_nrm * a_nrm;
  gl_Position = u_mvp * vec4(a_pos, 1.0);
}`;
const FRAG = `#version 300 es
precision mediump float;
in vec3 v_nrm;
uniform vec3 u_color;
uniform float u_alpha;
uniform float u_shade;
out vec4 o_color;
void main() {
  float l = length(v_nrm) > 0.0 ? abs(normalize(v_nrm).z) : 1.0;
  float s = mix(1.0, 0.45 + 0.55 * l, u_shade);
  o_color = vec4(u_color * s, u_alpha);
}`;

export function hexToRgb(hex) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(String(hex || ""));
  return m ? [1, 2, 3].map((i) => parseInt(m[i], 16) / 255) : [1, 1, 1];
}

function compile(gl, type, src) {
  const sh = gl.createShader(type);
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh) || "shader");
  return sh;
}

export function mountTurntable(store, els, invoke) {
  const orbit = { yaw: 0, pitch: 0, dist: DEFAULT_DISTANCE };
  let enabled = false;
  let lastDesign = null;
  let gl = null;
  let prog = null;
  let loc = null;
  let meshBuf = null;
  let lineBuf = null;
  let mesh = null; // { triangles, solid: [a, b], glass: [a, b] }
  let shaded = true;
  let fade = 1;
  let failure = "";
  let meshError = "";

  function note() {
    const text = failure || (meshError ? `Turned view: ${meshError}` : DEFAULT_NOTE);
    if (els.note.textContent !== text) els.note.textContent = text;
    els.note.classList.toggle("err", Boolean(failure || meshError));
  }

  function init() {
    if (gl || failure) return Boolean(gl);
    const ctx = els.canvas.getContext("webgl2", { antialias: true, alpha: false, preserveDrawingBuffer: true });
    if (!ctx) {
      failure = NO_WEBGL;
      note();
      return false;
    }
    try {
      gl = ctx;
      prog = gl.createProgram();
      gl.attachShader(prog, compile(gl, gl.VERTEX_SHADER, VERT));
      gl.attachShader(prog, compile(gl, gl.FRAGMENT_SHADER, FRAG));
      gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog) || "program");
      loc = {
        pos: gl.getAttribLocation(prog, "a_pos"),
        nrm: gl.getAttribLocation(prog, "a_nrm"),
        mvp: gl.getUniformLocation(prog, "u_mvp"),
        nmat: gl.getUniformLocation(prog, "u_nrm"),
        color: gl.getUniformLocation(prog, "u_color"),
        alpha: gl.getUniformLocation(prog, "u_alpha"),
        shade: gl.getUniformLocation(prog, "u_shade"),
      };
      meshBuf = { pos: gl.createBuffer(), nrm: gl.createBuffer() };
      lineBuf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, lineBuf);
      gl.bufferData(gl.ARRAY_BUFFER, boxLines(), gl.STATIC_DRAW);
    } catch (e) {
      failure = `The turned view could not start: ${e.message}`;
      gl = null;
      note();
      return false;
    }
    return true;
  }

  function setMesh(built) {
    if (!init()) return;
    const f = expandFaces(built.positions, built.indices);
    gl.bindBuffer(gl.ARRAY_BUFFER, meshBuf.pos);
    gl.bufferData(gl.ARRAY_BUFFER, f.positions, gl.STATIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, meshBuf.nrm);
    gl.bufferData(gl.ARRAY_BUFFER, f.normals, gl.STATIC_DRAW);
    mesh = { triangles: f.triangles, solid: built.solid, glass: built.glass };
    meshError = "";
    note();
    draw();
  }

  const fetchMesh = coalesced(
    "kit_mesh",
    setMesh,
    (msg) => {
      // The last good note stays on screen; the caption says what is wrong.
      meshError = msg;
      note();
    },
    invoke,
  );

  function resize() {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(els.canvas.clientWidth * dpr));
    const h = Math.max(1, Math.round(els.canvas.clientHeight * dpr));
    if (els.canvas.width !== w || els.canvas.height !== h) {
      els.canvas.width = w;
      els.canvas.height = h;
    }
  }

  function bindAttrib(buffer, which) {
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.enableVertexAttribArray(which);
    gl.vertexAttribPointer(which, 3, gl.FLOAT, false, 0, 0);
  }

  function draw() {
    if (!enabled || !gl || els.canvas.clientWidth === 0) return;
    resize();
    gl.viewport(0, 0, els.canvas.width, els.canvas.height);
    gl.clearColor(0.043, 0.055, 0.071, 1);
    gl.enable(gl.DEPTH_TEST);
    gl.depthMask(true);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.useProgram(prog);
    const cam = cameraMatrices(orbit, els.canvas.width / els.canvas.height);
    gl.uniformMatrix4fv(loc.mvp, false, cam.mvp);
    gl.uniformMatrix3fv(loc.nmat, false, cam.nrm);
    // The note box at the hit plane, for orientation.
    gl.disable(gl.BLEND);
    gl.uniform1f(loc.shade, 0);
    gl.uniform3f(loc.color, 0.55, 0.55, 0.6);
    gl.uniform1f(loc.alpha, 1);
    bindAttrib(lineBuf, loc.pos);
    gl.disableVertexAttribArray(loc.nrm);
    gl.vertexAttrib3f(loc.nrm, 0, 0, 1);
    gl.drawArrays(gl.LINES, 0, 8);
    if (!mesh) return;
    bindAttrib(meshBuf.pos, loc.pos);
    bindAttrib(meshBuf.nrm, loc.nrm);
    const colour = hexToRgb((store.state.design.colors || [])[0]);
    gl.uniform3fv(loc.color, colour);
    gl.uniform1f(loc.shade, shaded ? 1 : 0);
    // The approach fade: the whole note at one alpha, no depth test, so
    // overlapping parts brighten as they do in the game.
    const translucent = fade < 0.999;
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    if (translucent) {
      gl.disable(gl.DEPTH_TEST);
      gl.enable(gl.BLEND);
    }
    const [s0, s1] = mesh.solid;
    if (s1 > s0) {
      gl.uniform1f(loc.alpha, fade);
      gl.drawArrays(gl.TRIANGLES, s0, s1 - s0);
    }
    const [g0, g1] = mesh.glass;
    if (g1 > g0) {
      gl.enable(gl.BLEND);
      gl.depthMask(false);
      gl.uniform1f(loc.alpha, GLASS_ALPHA * fade);
      gl.drawArrays(gl.TRIANGLES, g0, g1 - g0);
      gl.depthMask(true);
    }
  }

  function setOrbit(next) {
    Object.assign(orbit, clampOrbit({ ...orbit, ...next }));
    draw();
  }

  let drag = null;
  els.canvas.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    els.canvas.setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, yaw: orbit.yaw, pitch: orbit.pitch };
  });
  els.canvas.addEventListener("pointermove", (e) => {
    if (!drag || !els.canvas.hasPointerCapture(e.pointerId)) return;
    setOrbit({
      yaw: drag.yaw + (e.clientX - drag.x) * DRAG_DEGREES_PER_PX,
      pitch: drag.pitch + (e.clientY - drag.y) * DRAG_DEGREES_PER_PX,
    });
  });
  const end = (e) => {
    if (els.canvas.hasPointerCapture(e.pointerId)) els.canvas.releasePointerCapture(e.pointerId);
    drag = null;
  };
  els.canvas.addEventListener("pointerup", end);
  els.canvas.addEventListener("pointercancel", end);
  els.canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  els.canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= 16;
      else if (e.deltaMode === 2) dy *= 400;
      setOrbit({ dist: orbit.dist * Math.exp(dy * 0.0015) });
    },
    { passive: false },
  );
  els.front.addEventListener("click", () => setOrbit({ yaw: 0, pitch: 0 }));
  els.threeQuarter.addEventListener("click", () => setOrbit(THREE_QUARTER));
  els.shaded.addEventListener("change", () => {
    shaded = els.shaded.checked;
    draw();
  });
  els.fade.addEventListener("input", () => {
    fade = Number(els.fade.value) || 1;
    draw();
  });
  new ResizeObserver(() => draw()).observe(els.canvas.parentElement);

  let saved = false;
  try {
    saved = localStorage.getItem(TURNTABLE_KEY) === "1";
  } catch {
    saved = false;
  }
  // The band below the canvas shows when either the previews or this
  // view is on; the previews' own figures follow their toggle.
  function apply(on) {
    enabled = on;
    store.setUi({ turntable: on });
    els.figure.hidden = !on;
    els.toggle.classList.toggle("on", on);
    const band = on || store.state.ui.previews;
    els.box.hidden = !band;
    els.splitter.hidden = !band;
    try {
      localStorage.setItem(TURNTABLE_KEY, on ? "1" : "0");
    } catch {
      // Best effort: the toggle still works for this session.
    }
    if (on) {
      note();
      if (init()) {
        if (lastDesign && !mesh) fetchMesh({ design: lastDesign });
        else draw();
      }
    }
  }
  els.toggle.addEventListener("click", () => apply(!store.state.ui.turntable));
  apply(saved);
  return {
    apply,
    toggle: () => apply(!store.state.ui.turntable),
    designChanged(d) {
      lastDesign = d;
      if (enabled && gl) fetchMesh({ design: d });
    },
    // The parts' colour can change without the geometry: redraw only.
    colorsChanged: () => draw(),
    orbit: () => ({ ...orbit }),
    setOrbit,
  };
}
