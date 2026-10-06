"use strict";

const $ = (id) => document.getElementById(id);
const IN_APP = new URLSearchParams(location.search).has("app");

const state = {
  settings: null,
  preview: null,
  mode: "drawing",
  view: new URLSearchParams(location.search).get("view") || localGet("hw-view") || "one",
  pass: 1,
  zoom: { f: 1, px: 0, py: 0 },
};

function localGet(k) { try { return localStorage.getItem(k); } catch { return null; } }
function localPut(k, v) { try { localStorage.setItem(k, v); } catch { } }

async function api(method, url, body) {
  const r = await fetch(url, {
    method,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    cache: "no-store",
  });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw Object.assign(new Error(data.detail || (data.errors || []).join("; ") || r.statusText), { data });
  return data;
}

let busyCount = 0;
function busy(on, text = "считаю…") {
  busyCount += on ? 1 : -1;
  $("busyText").textContent = text;
  $("busy").hidden = busyCount <= 0;
}

let toastTimer = 0;
function toast(text, ms = 5000) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), ms);
}

function fmtTime(s) {
  if (!(s > 0)) return "—";
  const m = Math.round(s / 60);
  if (m < 1) return "меньше минуты";
  const h = Math.floor(m / 60);
  return h ? `${h} ч ${m % 60} мин` : `${m} мин`;
}

function showMessages(errors, warnings) {
  const box = $("messages");
  box.innerHTML = "";
  for (const [list, cls] of [[errors, "err"], [warnings, "warn"]]) {
    for (const text of list || []) {
      const d = document.createElement("div");
      d.className = `msg ${cls}`;
      d.textContent = text;
      box.appendChild(d);
    }
  }
}

let saveTimer = 0;
function saveSettings() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => api("PUT", "/api/settings", state.settings).catch(() => { }), 400);
}

let previewSeq = 0;
async function refreshPreview() {
  if (!state.settings) return;
  const seq = ++previewSeq;
  busy(true);
  try {
    const p = await api("POST", "/api/drawing/preview", state.settings);
    if (seq !== previewSeq) return;
    state.preview = p;
    const n = p.parts?.length || 0;
    if (state.pass > n) state.pass = Math.max(1, n);
    showMessages(p.errors, p.warnings);
  } catch (e) {
    if (seq !== previewSeq) return;
    state.preview = null;
    showMessages(e.data?.errors || [e.message], []);
  } finally {
    busy(false);
  }
  renderChrome();
  draw();
}

function totalTime(p) {
  if (!p) return 0;
  const parts = p.parts || [];
  return parts.length ? parts.reduce((a, q) => a + (q.stats?.time_s || 0), 0) : p.stats?.time_s || 0;
}

function renderChrome() {
  const p = state.preview;
  const n = p?.parts?.length || 0;
  for (const b of $("modeSeg").children) b.classList.toggle("on", b.dataset.mode === state.mode);
  for (const b of $("viewSeg").children) b.classList.toggle("on", b.dataset.view === state.view);
  const drawing = state.mode === "drawing";
  $("viewSeg").hidden = !drawing;
  $("notebookStub").hidden = drawing;
  $("cv").hidden = !drawing;
  $("messages").hidden = !drawing;
  $("pager").hidden = !drawing || state.view !== "one" || n < 2;
  $("passLabel").textContent = n ? `проход ${state.pass} из ${n}` : "";
  $("prevPass").disabled = state.pass <= 1;
  $("nextPass").disabled = state.pass >= n;
  $("timeVal").textContent = fmtTime(totalTime(p));
  $("downloadBtn").disabled = !drawing || !p || (p.errors || []).length > 0 || n === 0;
  const imp = p?.import;
  $("fileName").textContent = imp ? imp.name : "";
  $("fileName").title = imp ? `${imp.name}${imp.units_note ? " — " + imp.units_note : ""}` : "";
}

const cv = $("cv");
const ctx = cv.getContext("2d");
const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();

function polyline(pts, T) {
  if (!pts.length) return;
  let [x, y] = T(pts[0]);
  ctx.moveTo(x, y);
  for (let i = 1; i < pts.length; i++) {
    [x, y] = T(pts[i]);
    ctx.lineTo(x, y);
  }
}

function rotationText(part) {
  const r = part.rotation === 0 ? "без поворота" : `поворот ${part.rotation}°`;
  return `${r}, в упоре угол ${part.corner} (${part.corner_name})`;
}

function drawSheet(box, part, caption) {
  const p = state.preview;
  const W = p.sheet.width, H = p.sheet.height;
  const pad = 22, top = caption ? 40 : 14;
  const kFit = Math.max(0.05, Math.min((box.w - 2 * pad) / W, (box.h - top - pad) / H));
  const k = kFit * state.zoom.f;
  const cx = box.x + box.w / 2 + state.zoom.px, cy = box.y + top + (box.h - top - pad) / 2 + state.zoom.py;
  const ox = cx - (W * k) / 2, oy = cy + (H * k) / 2;
  const T = ([x, y]) => [ox + x * k, oy - y * k];

  ctx.save();
  ctx.beginPath();
  ctx.rect(box.x, box.y, box.w, box.h);
  ctx.clip();

  ctx.save();
  ctx.shadowColor = css("--paper-edge");
  ctx.shadowBlur = 14;
  ctx.shadowOffsetY = 3;
  ctx.fillStyle = css("--paper");
  ctx.fillRect(ox, oy - H * k, W * k, H * k);
  ctx.restore();

  if (part) {
    const [x0, y0, x1, y1] = part.region;
    const a = T([x0, y1]), b = T([x1, y0]);
    ctx.fillStyle = css("--region");
    ctx.fillRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
    ctx.setLineDash([6, 4]);
    ctx.strokeStyle = css("--region-line");
    ctx.lineWidth = 1;
    ctx.strokeRect(a[0] + 0.5, a[1] + 0.5, b[0] - a[0] - 1, b[1] - a[1] - 1);
    ctx.setLineDash([]);
  }

  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const ink = css("--ink"), faint = css("--ink-faint");
  const base = Math.max(0.8, 0.3 * k);
  for (const pass of [false, true]) {
    for (const thick of [false, true]) {
      ctx.beginPath();
      let any = false;
      for (const st of p.strokes) {
        const mine = !part || st.k === part.index;
        if (mine !== pass || !!st.t !== thick) continue;
        polyline(st.p, T);
        any = true;
      }
      if (!any) continue;
      ctx.strokeStyle = pass ? ink : faint;
      ctx.lineWidth = thick ? base * 1.7 : base;
      ctx.stroke();
    }
  }

  if (caption) {
    ctx.fillStyle = css("--text");
    ctx.font = "600 13px system-ui, sans-serif";
    ctx.textBaseline = "top";
    ctx.fillText(caption.title, box.x + pad, box.y + 10, box.w - 2 * pad);
    if (caption.sub) {
      const w = ctx.measureText(caption.title).width;
      ctx.fillStyle = css("--muted");
      ctx.font = "13px system-ui, sans-serif";
      ctx.fillText(caption.sub, box.x + pad + w + 10, box.y + 10, Math.max(0, box.w - 2 * pad - w - 10));
    }
  }
  ctx.restore();
}

function partCaption(part, n) {
  return {
    title: `Проход ${part.index} из ${n}`,
    sub: `${rotationText(part)} · ${fmtTime(part.stats?.time_s)}`,
  };
}

function draw() {
  const d = window.devicePixelRatio || 1;
  const r = cv.getBoundingClientRect();
  if (cv.width !== Math.round(r.width * d) || cv.height !== Math.round(r.height * d)) {
    cv.width = Math.round(r.width * d);
    cv.height = Math.round(r.height * d);
  }
  ctx.setTransform(d, 0, 0, d, 0, 0);
  ctx.clearRect(0, 0, r.width, r.height);
  const p = state.preview;
  if (!p?.sheet || state.mode !== "drawing") return;
  const parts = p.parts || [];
  const n = parts.length;
  if (state.view === "one" || n < 2) {
    const part = n ? parts[Math.min(state.pass, n) - 1] : null;
    drawSheet({ x: 0, y: 0, w: r.width, h: r.height }, part, part && n > 1 ? partCaption(part, n) : part ? { title: rotationText(part) } : null);
    return;
  }
  const cols = n <= 2 ? (r.width >= r.height ? n : 1) : 2;
  const rows = Math.ceil(n / cols);
  const cw = r.width / cols, ch = r.height / rows;
  parts.forEach((part, i) => {
    const box = { x: (i % cols) * cw, y: Math.floor(i / cols) * ch, w: cw, h: ch };
    drawSheet(box, part, partCaption(part, n));
    ctx.save();
    ctx.strokeStyle = css("--line");
    ctx.strokeRect(box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1);
    ctx.restore();
  });
}

let drag = null;
cv.addEventListener("pointerdown", (e) => {
  drag = { x: e.clientX, y: e.clientY, px: state.zoom.px, py: state.zoom.py };
  cv.setPointerCapture(e.pointerId);
  cv.classList.add("drag");
});
cv.addEventListener("pointermove", (e) => {
  if (!drag) return;
  state.zoom.px = drag.px + e.clientX - drag.x;
  state.zoom.py = drag.py + e.clientY - drag.y;
  draw();
});
cv.addEventListener("pointerup", () => { drag = null; cv.classList.remove("drag"); });
cv.addEventListener("dblclick", () => { state.zoom = { f: 1, px: 0, py: 0 }; draw(); });
cv.addEventListener("wheel", (e) => {
  e.preventDefault();
  const f = Math.exp(-e.deltaY * 0.0015);
  const nf = Math.min(40, Math.max(0.3, state.zoom.f * f));
  const r = cv.getBoundingClientRect();
  const mx = e.clientX - r.left - r.width / 2 - state.zoom.px, my = e.clientY - r.top - r.height / 2 - state.zoom.py;
  const g = nf / state.zoom.f;
  state.zoom.px -= mx * (g - 1);
  state.zoom.py -= my * (g - 1);
  state.zoom.f = nf;
  draw();
}, { passive: false });
new ResizeObserver(draw).observe(cv);
addEventListener("themechange", draw);

$("modeSeg").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  state.mode = b.dataset.mode;
  renderChrome();
  draw();
});
$("viewSeg").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  state.view = b.dataset.view;
  localPut("hw-view", state.view);
  state.zoom = { f: 1, px: 0, py: 0 };
  renderChrome();
  draw();
});
function setPass(i) {
  const n = state.preview?.parts?.length || 0;
  state.pass = Math.min(Math.max(1, i), Math.max(1, n));
  renderChrome();
  draw();
}
$("prevPass").onclick = () => setPass(state.pass - 1);
$("nextPass").onclick = () => setPass(state.pass + 1);
addEventListener("keydown", (e) => {
  if (e.target.closest("input, select, textarea") || state.view !== "one") return;
  if (e.key === "ArrowLeft") setPass(state.pass - 1);
  if (e.key === "ArrowRight") setPass(state.pass + 1);
});

$("settingsBtn").onclick = (e) => {
  e.stopPropagation();
  $("settingsPop").hidden = !$("settingsPop").hidden;
};
document.addEventListener("click", (e) => {
  const pop = $("settingsPop");
  if (!pop.hidden && !pop.contains(e.target)) pop.hidden = true;
});
$("themeSel").value = localGet("hw-theme") || "auto";
$("themeSel").onchange = () => { localPut("hw-theme", $("themeSel").value); window.hwApplyTheme(); };

$("fileInput").addEventListener("change", async (e) => {
  const f = e.target.files[0];
  e.target.value = "";
  if (!f) return;
  const bytes = new Uint8Array(await f.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  busy(true, "загружаю…");
  try {
    const r = await api("POST", "/api/drawing/upload", { filename: f.name, content_b64: btoa(bin) });
    const dr = state.settings.drawing;
    dr.file = r.spec;
    dr.imp.pdf_page = 1;
    dr.weights.layers = {};
    state.pass = 1;
    state.zoom = { f: 1, px: 0, py: 0 };
    saveSettings();
    await refreshPreview();
  } catch (err) {
    showMessages(["Файл не загружен: " + err.message], []);
  } finally {
    busy(false);
  }
});

function fileNameFrom(resp, fallback) {
  const cd = resp.headers.get("Content-Disposition") || "";
  const star = cd.match(/filename\*=UTF-8''([^;]+)/)?.[1];
  return star ? decodeURIComponent(star) : cd.match(/filename="(.+?)"/)?.[1] || fallback;
}

$("downloadBtn").onclick = async () => {
  const tests = state.settings.drawing.test_files !== false;
  busy(true, "готовлю архив…");
  try {
    if (IN_APP) {
      const r = await api("POST", `/api/drawing/zip/save?tests=${tests}`, state.settings);
      toast(`Сохранено: ${r.path}`, 8000);
      return;
    }
    const r = await fetch(`/api/drawing/zip?tests=${tests}`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(state.settings),
    });
    if (!r.ok) {
      const d = await r.json().catch(() => ({}));
      throw Object.assign(new Error(r.statusText), { data: d });
    }
    const a = document.createElement("a");
    a.href = URL.createObjectURL(await r.blob());
    a.download = fileNameFrom(r, "drawing.zip");
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  } catch (e) {
    showMessages(e.data?.errors || [e.message], state.preview?.warnings || []);
  } finally {
    busy(false);
  }
};

(async function init() {
  renderChrome();
  try {
    state.settings = await api("GET", "/api/settings");
  } catch (e) {
    showMessages(["Настройки не загрузились: " + e.message], []);
    return;
  }
  await refreshPreview();
})();
