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
  saveTimer = setTimeout(() => api("PUT", "/api/settings", state.settings).catch((e) => showMessages(e.data?.errors || [e.message], [])), 400);
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
  syncForms();
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

  if (state.settings?.drawing.show_travel && p.travel?.length) {
    ctx.save();
    ctx.setLineDash([3, 4]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = css("--travel");
    ctx.beginPath();
    for (const [a, b, k] of p.travel) {
      if (part && k !== part.index) continue;
      const pa = T(a), pb = T(b);
      ctx.moveTo(pa[0], pa[1]);
      ctx.lineTo(pb[0], pb[1]);
    }
    ctx.stroke();
    ctx.restore();
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

let quickItems = [];
let settingsItems = [];
let activeTab = localGet("hw-tab") || "main";
let lastTravel = null;
let appInfo = { lang: "ru", langs: [["ru", "Русский"], ["en", "English"]] };

function specialOptions(item) {
  const { f, input } = item;
  let opts = null;
  if (f.type === "file") opts = (state.files || []).map((x) => [x.spec, x.label]);
  if (f.type === "theme") opts = [["auto", "как в системе"], ["light", "светлая"], ["dark", "тёмная"]];
  if (f.type === "lang") opts = appInfo.langs;
  if (!opts) return;
  const key = JSON.stringify(opts);
  if (input.dataset.opts === key) return;
  input.dataset.opts = key;
  input.innerHTML = "";
  for (const [v, t] of opts) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = t;
    input.appendChild(o);
  }
}

function itemValue(item) {
  const S = state.settings;
  switch (item.f.type) {
    case "file": return S.drawing.file;
    case "theme": return localGet("hw-theme") || "auto";
    case "lang": return appInfo.lang;
    case "travel": return !!S.printer.travel;
    default: return fieldValue(item.f, S);
  }
}

function syncForms() {
  const S = state.settings, P = state.preview;
  if (!S) return;
  const groups = new Map();
  for (const item of [...quickItems, ...settingsItems]) {
    const { f, g, row, input } = item;
    specialOptions(item);
    const visible = (!g.show || g.show(S, P)) && (!f.show || f.show(S, P));
    row.hidden = !visible;
    if (row.nextElementSibling?.classList.contains("fhint")) row.nextElementSibling.hidden = !visible;
    input.disabled = !!f.disabled?.(S, P);
    if (!groups.has(item.group)) groups.set(item.group, false);
    if (visible) groups.set(item.group, true);
    if (document.activeElement === input) continue;
    const v = itemValue(item);
    if (input.type === "checkbox") input.checked = !!v;
    else input.value = v == null ? "" : String(v);
  }
  for (const [box, any] of groups) box.hidden = !any;
}

let previewTimer = 0;
function schedulePreview() {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 250);
}

async function onFieldChange(item) {
  const S = state.settings;
  const { f, input } = item;
  if (f.type === "theme") {
    localPut("hw-theme", input.value);
    window.hwApplyTheme();
    return;
  }
  if (f.type === "lang") {
    try {
      await api("PUT", "/api/app", { lang: input.value });
      await flushSave();
      location.reload();
    } catch (e) {
      toast("Язык не переключился: " + e.message);
    }
    return;
  }
  if (f.type === "file") {
    S.drawing.file = input.value;
    S.drawing.imp.pdf_page = 1;
    S.drawing.weights.layers = {};
    state.pass = 1;
    state.zoom = { f: 1, px: 0, py: 0 };
  } else if (f.type === "travel") {
    if (input.checked) {
      S.printer.travel = lastTravel || { x_min: 0, x_max: S.printer.work_w, y_min: 0, y_max: S.printer.work_h };
    } else {
      lastTravel = S.printer.travel;
      S.printer.travel = null;
    }
  } else {
    const r = readInput(item);
    if (!r.ok) return;
    if (f.set) f.set(S, r.v);
    else setPath(S, f.path, r.v);
    f.onSet?.(S, state.preview);
    if (f.path?.startsWith("printer.travel.")) lastTravel = { ...S.printer.travel };
  }
  syncForms();
  saveSettings();
  if (f.redraw) draw();
  else schedulePreview();
}

async function flushSave() {
  clearTimeout(saveTimer);
  if (state.settings) await api("PUT", "/api/settings", state.settings).catch(() => { });
}

function buildForms() {
  quickItems = buildForm($("quickForm"), QUICK, { change: onFieldChange });
  const nav = $("settingsTabs");
  nav.innerHTML = "";
  for (const t of TABS) {
    const b = document.createElement("button");
    b.textContent = t.title;
    b.dataset.tab = t.id;
    b.onclick = () => showTab(t.id);
    nav.appendChild(b);
  }
  showTab(activeTab);
}

function showTab(id) {
  const tab = TABS.find((t) => t.id === id) || TABS[0];
  activeTab = tab.id;
  localPut("hw-tab", tab.id);
  for (const b of $("settingsTabs").children) b.classList.toggle("on", b.dataset.tab === tab.id);
  $("settingsTitle").textContent = tab.title;
  settingsItems = buildForm($("settingsForm"), tab.groups, { change: onFieldChange });
  syncForms();
}

function openSettings(open) {
  $("settingsPanel").hidden = !open;
  if (open) {
    $("paramsPop").hidden = true;
    syncForms();
  }
}
$("settingsBtn").onclick = () => openSettings(true);
$("settingsClose").onclick = () => openSettings(false);
$("settingsPanel").addEventListener("mousedown", (e) => { if (e.target === $("settingsPanel")) openSettings(false); });

function openParams(open) {
  $("paramsPop").hidden = !open;
  $("paramsBtn").setAttribute("aria-expanded", String(open));
  if (open) syncForms();
}
$("paramsBtn").onclick = (e) => { e.stopPropagation(); openParams($("paramsPop").hidden); };
$("paramsClose").onclick = () => openParams(false);
document.addEventListener("mousedown", (e) => {
  const pop = $("paramsPop");
  if (!pop.hidden && !pop.contains(e.target) && !$("paramsBtn").contains(e.target)) openParams(false);
});
addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (!$("settingsPanel").hidden) openSettings(false);
  else if (!$("paramsPop").hidden) openParams(false);
});
async function loadFiles() {
  try {
    state.files = await api("GET", "/api/drawing/files");
  } catch {
    state.files = [];
  }
}

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
    await loadFiles();
    const dr = state.settings.drawing;
    dr.file = r.spec;
    dr.imp.pdf_page = 1;
    dr.weights.layers = {};
    state.pass = 1;
    state.zoom = { f: 1, px: 0, py: 0 };
    saveSettings();
    syncForms();
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
  await flushSave();
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
  try {
    appInfo = await api("GET", "/api/app");
  } catch { }
  await loadFiles();
  const q = new URLSearchParams(location.search);
  if (q.get("tab")) activeTab = q.get("tab");
  buildForms();
  syncForms();
  await refreshPreview();
  if (q.get("open") === "settings") openSettings(true);
  if (q.get("open") === "params") openParams(true);
})();
