"use strict";

const $ = (id) => document.getElementById(id);
const IN_APP = new URLSearchParams(location.search).has("app");

const state = {
  settings: null,
  preview: null,
  text: null,
  fonts: [],
  mode: new URLSearchParams(location.search).get("mode") || localGet("hw-mode") || "drawing",
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
  const drawing = state.mode === "drawing";
  try {
    const p = await api("POST", drawing ? "/api/drawing/preview" : "/api/preview", state.settings);
    if (seq !== previewSeq) return;
    if (drawing) {
      state.preview = p;
      const n = p.parts?.length || 0;
      if (state.pass > n) state.pass = Math.max(1, n);
    } else {
      state.text = p;
      renderTextPanel();
    }
    showMessages(p.errors, p.warnings);
  } catch (e) {
    if (seq !== previewSeq) return;
    if (drawing) state.preview = null;
    else state.text = null;
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
  const drawing = state.mode === "drawing";
  const p = state.preview;
  const n = p?.parts?.length || 0;
  for (const b of $("modeSeg").children) b.classList.toggle("on", b.dataset.mode === state.mode);
  for (const b of $("viewSeg").children) b.classList.toggle("on", b.dataset.view === state.view);
  $("viewSeg").hidden = !drawing;
  $("pager").hidden = !drawing || state.view !== "one" || n < 2;
  $("passLabel").textContent = n ? `проход ${state.pass} из ${n}` : "";
  $("prevPass").disabled = state.pass <= 1;
  $("nextPass").disabled = state.pass >= n;
  for (const id of ["uploadBtn", "paramsBtn"]) $(id).hidden = !drawing;
  for (const id of ["textBtn", "handBtn"]) $(id).hidden = drawing;
  if (drawing) {
    $("timeVal").textContent = fmtTime(totalTime(p));
    $("downloadBtn").disabled = !p || (p.errors || []).length > 0 || n === 0;
    $("downloadBtn").title = $("downloadBtn").disabled ? "Сначала исправь ошибки на листе" : "Архив всех проходов";
    const imp = p?.import;
    $("fileName").textContent = imp ? imp.name : "";
    $("fileName").title = imp ? `${imp.name}${imp.units_note ? " — " + imp.units_note : ""}` : "";
  } else {
    const t = state.text;
    $("timeVal").textContent = fmtTime(t?.stats?.time_s);
    $("downloadBtn").disabled = !t || (t.errors || []).length > 0 || !t.strokes?.length;
    $("downloadBtn").title = $("downloadBtn").disabled ? "Сначала исправь ошибки на листе" : "Gcode этого листа";
    $("fileName").textContent = t?.font ? t.font.name : "";
    $("fileName").title = t?.word_count != null ? `Слов в тексте: ${t.word_count}` : "";
  }
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
  if (state.mode === "notebook") {
    drawNotebook(r);
    return;
  }
  const p = state.preview;
  if (!p?.sheet) return;
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
cv.addEventListener("pointerup", (e) => {
  if (drag && state.mode === "notebook" && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 4) pickLetter(e);
  drag = null;
  cv.classList.remove("drag");
});
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
  if (!b || b.dataset.mode === state.mode) return;
  state.mode = b.dataset.mode;
  localPut("hw-mode", state.mode);
  state.zoom = { f: 1, px: 0, py: 0 };
  closePopovers();
  buildForms();
  showMessages([], []);
  renderChrome();
  draw();
  refreshPreview();
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
let handItems = [];
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
  if (f.type === "font") {
    opts = state.fonts.map((x) => [x.spec, `${x.label} · ${x.mode === "outlines" ? "контуры" : "штрихи"}`]);
    const S = state.settings;
    if (S && !state.fonts.some((x) => x.spec === S.font)) opts.push([S.font, `${S.font} (не найден)`]);
  }
  if (f.type === "profile") {
    const S = state.settings;
    opts = Object.keys(S?.profiles || {}).map((k) => [k, k]);
    if (S && !S.profiles[S.active_profile]) opts.push(["", "(свои параметры)"]);
  }
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
    case "font": return S.font;
    case "profile": return S.profiles[S.active_profile] ? S.active_profile : "";
    case "seed":
    case "fontUpload":
    case "testFile": return null;
    default: return fieldValue(item.f, S);
  }
}

function syncForms() {
  const S = state.settings, P = state.preview;
  if (!S) return;
  const groups = new Map();
  for (const item of [...quickItems, ...handItems, ...settingsItems]) {
    const { f, g, row, input } = item;
    specialOptions(item);
    const P2 = state.mode === "notebook" ? state.text : P;
    const visible = (!g.mode || g.mode === state.mode) && (!g.show || g.show(S, P2)) && (!f.show || f.show(S, P2));
    if (f.type === "info") {
      const t = f.text(S, P2);
      input.textContent = t;
      row.hidden = !visible || !t;
      if (!groups.has(item.group)) groups.set(item.group, false);
      continue;
    }
    row.hidden = !visible;
    if (row.nextElementSibling?.classList.contains("fhint")) row.nextElementSibling.hidden = !visible;
    input.disabled = !!f.disabled?.(S, P);
    if (!groups.has(item.group)) groups.set(item.group, false);
    if (visible) groups.set(item.group, true);
    if (document.activeElement === input || input.tagName === "BUTTON") continue;
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
  if (f.type === "fontUpload") {
    $("fontInput").click();
    return;
  }
  if (f.type === "testFile") {
    await downloadTextGcode("/api/testfile");
    return;
  }
  if (f.type === "seed") {
    S.randomness.seed = Math.floor(Math.random() * 1e6);
  } else if (f.type === "font") {
    S.font = input.value;
    const font = state.fonts.find((x) => x.spec === S.font);
    if (font) S.mode = font.mode;
  } else if (f.type === "profile") {
    const prof = S.profiles[input.value];
    if (prof) {
      S.active_profile = input.value;
      S.sheet = JSON.parse(JSON.stringify(prof.sheet));
      S.typography.size_mm = prof.size_mm;
      S.typography.baseline_shift = prof.baseline_shift;
    }
  } else if (f.type === "file") {
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
  handItems = buildForm($("handForm"), HAND, { change: onFieldChange });
  const nav = $("settingsTabs");
  nav.innerHTML = "";
  for (const t of TABS.filter((x) => !x.mode || x.mode === state.mode)) {
    const b = document.createElement("button");
    b.textContent = t.title;
    b.dataset.tab = t.id;
    b.onclick = () => showTab(t.id);
    nav.appendChild(b);
  }
  showTab(activeTab);
}

function showTab(id) {
  const tabs = TABS.filter((x) => !x.mode || x.mode === state.mode);
  const tab = tabs.find((t) => t.id === id) || tabs[0];
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
    closePopovers();
    syncForms();
  }
}
$("settingsBtn").onclick = () => openSettings(true);
$("settingsClose").onclick = () => openSettings(false);
$("settingsPanel").addEventListener("mousedown", (e) => { if (e.target === $("settingsPanel")) openSettings(false); });

const POPS = [["paramsPop", "paramsBtn"], ["handPop", "handBtn"], ["textPop", "textBtn"]];

function openPop(id, open) {
  for (const [pid, bid] of POPS) {
    const on = pid === id ? open : false;
    $(pid).hidden = !on;
    $(bid).setAttribute("aria-expanded", String(on));
  }
  if (open) {
    syncForms();
    if (id === "textPop") renderTextPanel();
  }
}
function closePopovers() {
  openPop("", false);
}
function openParams(open) {
  openPop("paramsPop", open);
}
for (const [pid, bid] of POPS) {
  $(bid).addEventListener("click", (e) => { e.stopPropagation(); openPop(pid, $(pid).hidden); });
}
$("paramsClose").onclick = closePopovers;
$("handClose").onclick = closePopovers;
$("textClose").onclick = closePopovers;
document.addEventListener("mousedown", (e) => {
  for (const [pid, bid] of POPS) {
    const pop = $(pid);
    if (!pop.hidden && !pop.contains(e.target) && !$(bid).contains(e.target)) openPop(pid, false);
  }
});
addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (!$("settingsPanel").hidden) openSettings(false);
  else closePopovers();
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
  if (state.mode === "notebook") {
    await downloadTextGcode("/api/gcode");
    return;
  }
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


let nbView = null;

function textTransform(S) {
  const t = S.typography, a = (t.rotation_deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  return ([x, y]) => [c * x - s * y + t.dx, s * x + c * y + t.dy];
}

function drawNotebook(r) {
  const S = state.settings, P = state.text;
  if (!S) return;
  const sh = S.sheet, W = sh.width, H = sh.height;
  const pad = 26;
  const k = Math.max(0.05, Math.min((r.width - 2 * pad) / W, (r.height - 2 * pad) / H)) * state.zoom.f;
  const ox = r.width / 2 + state.zoom.px - (W * k) / 2, oy = r.height / 2 + state.zoom.py + (H * k) / 2;
  nbView = { k, ox, oy };
  const T = ([x, y]) => [ox + x * k, oy - y * k];
  const line = (a, b) => { const p = T(a), q = T(b); ctx.moveTo(p[0], p[1]); ctx.lineTo(q[0], q[1]); };

  ctx.save();
  ctx.shadowColor = css("--paper-edge");
  ctx.shadowBlur = 14;
  ctx.shadowOffsetY = 3;
  ctx.fillStyle = css("--paper");
  ctx.fillRect(ox, oy - H * k, W * k, H * k);
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(ox, oy - H * k, W * k, H * k);
  ctx.clip();
  const bases = P?.baselines?.map((b) => b - S.typography.baseline_shift) || [];
  if (S.preview.show_ruling) {
    ctx.lineWidth = 1;
    ctx.strokeStyle = css("--rule");
    ctx.beginPath();
    if (sh.ruling === "grid" && sh.grid_step > 0 && sh.grid_step * k > 3) {
      const y0 = H - sh.first_line_top;
      for (let y = ((y0 % sh.grid_step) + sh.grid_step) % sh.grid_step; y <= H; y += sh.grid_step) line([0, y], [W, y]);
      for (let x = sh.margin_left % sh.grid_step; x <= W; x += sh.grid_step) line([x, 0], [x, H]);
    } else if (sh.ruling === "lines") {
      for (const b of bases) line([0, b], [W, b]);
    }
    ctx.stroke();
  }
  ctx.strokeStyle = css("--margin-rule");
  ctx.beginPath();
  line([sh.margin_left, 0], [sh.margin_left, H]);
  line([W - sh.margin_right, 0], [W - sh.margin_right, H]);
  ctx.stroke();
  ctx.setLineDash([5, 4]);
  ctx.strokeStyle = css("--muted");
  const top = H - sh.first_line_top;
  const a = T([sh.margin_left, top]), b = T([W - sh.margin_right, sh.bottom_limit]);
  ctx.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
  ctx.setLineDash([]);
  ctx.restore();

  ctx.save();
  ctx.fillStyle = css("--resume");
  const z = T([0, 0]);
  ctx.beginPath();
  ctx.arc(z[0], z[1], 4, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  if (!P) return;
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.lineWidth = Math.max(1, 0.35 * k);
  for (const done of [true, false]) {
    ctx.strokeStyle = css(done ? "--ink-done" : "--ink");
    ctx.beginPath();
    for (const st of P.strokes || []) {
      if (!!st.d !== done) continue;
      polyline(st.p.length === 1 ? [st.p[0], [st.p[0][0] + 0.01, st.p[0][1]]] : st.p, T);
    }
    ctx.stroke();
  }
  ctx.restore();

  if (S.preview.show_travel && P.travel?.length) {
    ctx.save();
    ctx.setLineDash([3, 4]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = css("--travel");
    ctx.beginPath();
    for (const [p, q] of P.travel) line(p, q);
    ctx.stroke();
    ctx.restore();
  }

  const TT = textTransform(S), xh = S.typography.size_mm;
  const box = (g) => {
    let pts;
    if (g.b) {
      const pd = Math.max(0.4, xh * 0.08), [x0, y0, x1, y1] = g.b;
      pts = [[x0 - pd, y0 - pd], [x1 + pd, y0 - pd], [x1 + pd, y1 + pd], [x0 - pd, y1 + pd]];
    } else {
      const w = Math.max(g.adv, xh * 0.3);
      pts = [[g.x, g.y - xh * 0.5], [g.x + w, g.y - xh * 0.5], [g.x + w, g.y + xh * 1.5], [g.x, g.y + xh * 1.5]].map(TT);
    }
    ctx.beginPath();
    pts.map(T).forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
    ctx.closePath();
  };
  ctx.save();
  for (const g of P.glyphs || []) {
    if (!g.missing) continue;
    box(g);
    ctx.fillStyle = "rgba(179,38,30,.18)";
    ctx.fill();
    ctx.strokeStyle = "#b3261e";
    ctx.lineWidth = 1;
    ctx.stroke();
  }
  const e = P.end;
  if (e?.last_word) {
    const g = P.glyphs.find((q) => q.w === e.last_word && q.l === e.last_letter && !q.h);
    if (g) {
      box(g);
      ctx.fillStyle = "rgba(255,200,0,.35)";
      ctx.fill();
    }
  }
  const rw = S.text_options.resume_word, rl = S.text_options.resume_letter;
  if (rw) {
    for (const g of P.glyphs.filter((q) => q.w === rw && q.l === rl && !q.h)) {
      box(g);
      ctx.strokeStyle = css("--resume");
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.fillStyle = "rgba(47,95,179,.12)";
      ctx.fill();
    }
  }
  ctx.restore();
}

function glyphAt(x, y) {
  const P = state.text, S = state.settings;
  if (!P?.glyphs) return null;
  let best = null, bestScore = Infinity;
  const xh = S.typography.size_mm;
  for (const g of P.glyphs) {
    if (g.h) continue;
    let cx, cy, inside = false, area = 0;
    if (g.b) {
      const [x0, y0, x1, y1] = g.b, pd = xh * 0.1;
      inside = x >= x0 - pd && x <= x1 + pd && y >= y0 - pd && y <= y1 + pd;
      cx = (x0 + x1) / 2;
      cy = (y0 + y1) / 2;
      area = (x1 - x0) * (y1 - y0);
    } else {
      cx = g.x + g.adv / 2;
      cy = g.y + xh / 2;
    }
    const d = Math.hypot(x - cx, y - cy);
    const score = inside ? area * 1e-3 + d * 1e-3 : 1e6 + d;
    if (score < bestScore && (inside || d < xh * 1.2)) {
      best = g;
      bestScore = score;
    }
  }
  return best;
}

function pickLetter(e) {
  if (!nbView) return;
  const r = cv.getBoundingClientRect();
  const x = (e.clientX - r.left - nbView.ox) / nbView.k, y = (nbView.oy - (e.clientY - r.top)) / nbView.k;
  const g = glyphAt(x, y);
  if (!g) return;
  const to = state.settings.text_options;
  to.resume_word = g.w;
  to.resume_letter = g.l;
  toast(`Печать продолжится со слова ${g.w}, буквы ${g.l} («${g.c}»)`);
  saveSettings();
  schedulePreview();
}

function renderTextPanel() {
  const S = state.settings, P = state.text;
  if (!S) return;
  const ta = $("textArea");
  if (document.activeElement !== ta && ta.value !== S.text) ta.value = S.text;
  if (document.activeElement !== $("startWord")) $("startWord").value = S.text_options.start_word;
  $("textInfo").textContent = P?.word_count != null ? `Слов в тексте: ${P.word_count}` : "";
  const r = P?.resume;
  $("resumeText").textContent = r
    ? `Продолжение со слова ${r.word}, буквы ${r.letter}${r.connected ? " (с точки связки)" : ""}; написанное — серым`
    : "Пишется весь лист";
  $("resumeOff").hidden = !S.text_options.resume_word;
  const e = P?.end;
  if (e?.last_word) {
    $("endText").textContent = e.next_word
      ? `На лист помещаются слова ${e.first_word}–${e.last_word}`
      : `На лист помещаются слова ${e.first_word}–${e.last_word}; текст на этом листе заканчивается`;
    $("nextSheet").hidden = !e.next_word;
  } else {
    $("endText").textContent = "";
    $("nextSheet").hidden = true;
  }
  renderMissing();
}

function renderMissing() {
  const box = $("missingBox"), S = state.settings, list = state.text?.missing || [];
  box.innerHTML = "";
  if (!list.length) return;
  const head = document.createElement("div");
  head.innerHTML = "<b>Этих символов нет в шрифте.</b> Реши для каждого, иначе gcode не создаётся.";
  box.appendChild(head);
  for (const m of list) {
    const choice = S.text_options.missing[m.char];
    const row = document.createElement("div");
    row.className = "m-row" + (m.resolved ? " ok" : "");
    const ch = document.createElement("span");
    ch.className = "ch";
    ch.textContent = m.char === " " ? "␣" : m.char;
    const info = document.createElement("span");
    info.textContent = `${m.count} раз · ${m.code}`;
    info.title = m.name;
    const sel = document.createElement("select");
    for (const [v, t] of [["", "не решено"], ["skip", "пропустить"], ["replace", "заменить на"]]) {
      const o = document.createElement("option");
      o.value = v;
      o.textContent = t;
      sel.appendChild(o);
    }
    sel.value = choice ? choice.action : "";
    const inp = document.createElement("input");
    inp.type = "text";
    inp.value = choice?.replacement || "";
    inp.hidden = sel.value !== "replace";
    const apply = () => {
      if (!sel.value) delete S.text_options.missing[m.char];
      else S.text_options.missing[m.char] = { action: sel.value, replacement: inp.value };
      inp.hidden = sel.value !== "replace";
      saveSettings();
      schedulePreview();
    };
    sel.onchange = apply;
    inp.onchange = apply;
    row.append(ch, info, sel, inp);
    box.appendChild(row);
  }
}

let textTimer = 0;
$("textArea").addEventListener("input", () => {
  state.settings.text = $("textArea").value;
  clearTimeout(textTimer);
  textTimer = setTimeout(() => { saveSettings(); refreshPreview(); }, 500);
});
$("startWord").addEventListener("change", () => {
  const v = Math.round(Number($("startWord").value));
  if (!(v >= 1)) return;
  state.settings.text_options.start_word = v;
  saveSettings();
  schedulePreview();
});
$("resumeOff").onclick = () => {
  const to = state.settings.text_options;
  to.resume_word = 0;
  to.resume_letter = 1;
  saveSettings();
  schedulePreview();
};
$("nextSheet").onclick = () => {
  const e = state.text?.end;
  if (!e?.next_word) return;
  const to = state.settings.text_options;
  to.start_word = e.next_word;
  to.resume_word = 0;
  to.resume_letter = 1;
  saveSettings();
  schedulePreview();
};
$("txtInput").addEventListener("change", async (e) => {
  const f = e.target.files[0];
  e.target.value = "";
  if (!f) return;
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(await f.arrayBuffer());
    state.settings.text = text.replace(/^﻿/, "");
    $("textArea").value = state.settings.text;
    saveSettings();
    refreshPreview();
  } catch {
    toast("Файл не в UTF-8. Сохрани его в кодировке UTF-8 и загрузи снова.");
  }
});

async function loadFonts() {
  try {
    state.fonts = await api("GET", "/api/fonts");
  } catch {
    state.fonts = [];
  }
}

$("fontInput").addEventListener("change", async (e) => {
  const f = e.target.files[0];
  e.target.value = "";
  if (!f) return;
  const payload = { name: f.name.replace(/\.(svg|ttf|otf)$/i, ""), files: [] };
  if (/\.(ttf|otf)$/i.test(f.name)) {
    const bytes = new Uint8Array(await f.arrayBuffer());
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    payload.files.push({ filename: f.name, content_b64: btoa(bin) });
  } else {
    payload.files.push({ filename: f.name, content: await f.text() });
  }
  busy(true, "загружаю шрифт…");
  try {
    const r = await api("POST", "/api/fonts/upload", payload);
    state.settings.font = r.spec;
    state.settings.mode = r.mode;
    await loadFonts();
    saveSettings();
    syncForms();
    refreshPreview();
    if (r.notes?.length) toast(r.notes.join(" "), 9000);
  } catch (err) {
    toast("Шрифт не загружен: " + err.message, 8000);
  } finally {
    busy(false);
  }
});

async function saveText(filename, content) {
  if (IN_APP) {
    const r = await api("POST", "/api/save", { filename, content });
    toast(`Сохранено: ${r.path}`, 8000);
    return;
  }
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([content], { type: "text/plain" }));
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function downloadTextGcode(url) {
  await flushSave();
  busy(true, "готовлю gcode…");
  try {
    const r = await api("POST", url, state.settings);
    await saveText(r.filename, r.gcode);
  } catch (e) {
    showMessages(e.data?.errors || [e.message], state.text?.warnings || []);
  } finally {
    busy(false);
  }
}

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
  await Promise.all([loadFiles(), loadFonts()]);
  const q = new URLSearchParams(location.search);
  if (q.get("tab")) activeTab = q.get("tab");
  buildForms();
  syncForms();
  await refreshPreview();
  if (q.get("open") === "settings") openSettings(true);
  const pop = { params: "paramsPop", hand: "handPop", text: "textPop" }[q.get("open")];
  if (pop) openPop(pop, true);
})();
