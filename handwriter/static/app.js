"use strict";

let S = null;
let P = null;
const cssVar = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
let TEST = null;
let lastTravel = { x_min: -5, x_max: 200, y_min: -5, y_max: 200 };
const $ = (id) => document.getElementById(id);

function getPath(obj, path) { return path.split(".").reduce((o, k) => (o == null ? o : o[k]), obj); }
function setPath(obj, path, v) {
  const ks = path.split("."); let o = obj;
  for (let i = 0; i < ks.length - 1; i++) o = o[ks[i]];
  o[ks[ks.length - 1]] = v;
}

function bindFields() {
  document.querySelectorAll("[data-path]").forEach((el) => {
    const path = el.dataset.path;
    const v = getPath(S, path);
    if (el.type === "checkbox") el.checked = !!v;
    else el.value = v ?? "";
  });
  const t = S.printer.travel;
  $("travelMeasured").checked = !!t;
  if (t) lastTravel = { ...t };
  document.querySelectorAll("[data-travel]").forEach((el) => {
    el.value = lastTravel[el.dataset.travel];
    el.disabled = !t;
  });
  document.querySelectorAll('input[name=mode]').forEach((r) => (r.checked = r.value === S.mode));
  $("outlineBox").style.display = S.mode === "outlines" ? "" : "none";
  buildOutlineSliders($("outlineSliders"), () => S.outline, () => changed("full"));
  buildSliders($("randomSliders"), RANDOM_PARAMS, () => S.randomness, () => changed("full"));
  buildSliders($("connectionSliders"), CONNECTION_PARAMS, () => S.connections, () => changed("full"));
}

document.querySelectorAll('input[name=mode]').forEach((r) => r.addEventListener("change", () => {
  if (!r.checked) return;
  S.mode = r.value;
  const cur = FONTS.find((f) => f.spec === S.font);
  if (!cur || cur.mode !== S.mode) {
    const f = FONTS.find((f) => f.mode === S.mode);
    if (f) { S.font = f.spec; $("fontSelect").value = f.spec; }
  }
  $("outlineBox").style.display = S.mode === "outlines" ? "" : "none";
  changed("full");
}));

function onFieldChange(el) {
  const path = el.dataset.path;
  let v;
  if (el.type === "checkbox") v = el.checked;
  else if (el.type === "number") {
    if (el.value === "" || isNaN(Number(el.value))) return;
    v = Number(el.value);
    if (path === "text_options.start_word") v = Math.max(1, Math.round(v));
  } else v = el.value;
  setPath(S, path, v);
  changed(path.startsWith("preview.") ? "redraw" : "full");
}

document.addEventListener("input", (e) => {
  const el = e.target;
  if (el.dataset && el.dataset.path) onFieldChange(el);
  if (el.dataset && el.dataset.travel) {
    if (el.value === "" || isNaN(Number(el.value))) return;
    lastTravel[el.dataset.travel] = Number(el.value);
    if (S.printer.travel) S.printer.travel = { ...lastTravel };
    changed("full");
  }
});
document.addEventListener("change", (e) => {
  const el = e.target;
  if (el.tagName === "SELECT" && el.dataset.path) onFieldChange(el);
});

$("travelMeasured").addEventListener("change", (e) => {
  S.printer.travel = e.target.checked ? { ...lastTravel } : null;
  document.querySelectorAll("[data-travel]").forEach((el) => (el.disabled = !e.target.checked));
  changed("full");
});

$("text").addEventListener("keydown", (e) => {
  if (e.key === "Tab" && !e.shiftKey) {
    e.preventDefault();
    const ta = e.target, a = ta.selectionStart, b = ta.selectionEnd;
    ta.value = ta.value.slice(0, a) + "\t" + ta.value.slice(b);
    ta.selectionStart = ta.selectionEnd = a + 1;
    onFieldChange(ta);
  }
});

let previewTimer = null, saveTimer = null, previewSeq = 0;
function changed(kind) {
  if (kind === "redraw") { draw(); scheduleSave(); return; }
  clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 250);
  scheduleSave();
}
let savePending = false;
function scheduleSave() {
  savePending = true;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(flushSave, 700);
}
async function flushSave() {
  clearTimeout(saveTimer);
  if (!savePending || !S) return;
  savePending = false;
  await api("PUT", "/api/settings", S).catch(() => {});
}

document.querySelectorAll("a[data-tab]").forEach((a) => a.addEventListener("click", async (e) => {
  e.preventDefault(); await flushSave(); location.href = a.href;
}));
window.addEventListener("pagehide", () => {
  if (savePending && S) fetch("/api/settings", { method: "PUT", keepalive: true,
    headers: { "Content-Type": "application/json" }, body: JSON.stringify(S) });
});

async function api(method, url, body) {
  const r = await fetch(url, {
    method, headers: body ? { "Content-Type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) {
    const err = new Error(data.detail ? (typeof data.detail === "string" ? data.detail : JSON.stringify(data.detail)) : r.statusText);
    err.data = data; err.status = r.status; throw err;
  }
  return data;
}

async function refreshPreview() {
  const seq = ++previewSeq;
  $("busy").textContent = "считаю…";
  try {
    const data = await api("POST", "/api/preview", S);
    if (seq !== previewSeq) return;
    P = data;
    if ($("showTest").checked) await loadTest();
    renderStatus(); renderMissing(); renderFontInfo();
    if (view.auto) fit();
    draw();
  } catch (e) {
    showMessages([`Ошибка сервера: ${e.message}`], []);
  } finally {
    if (seq === previewSeq) $("busy").textContent = "";
  }
}

function fmtTime(s) {
  if (s < 60) return `${s} с`;
  const m = Math.floor(s / 60), r = s % 60;
  return m < 60 ? `${m} мин ${r} с` : `${Math.floor(m / 60)} ч ${m % 60} мин`;
}
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }

function renderStatus() {
  const st = P.stats;
  $("stats").innerHTML =
    `<span>Путь карандаша: <b>${st.draw_mm} мм</b></span>` +
    `<span>Холостые: <b>${st.travel_mm} мм</b></span>` +
    `<span>Штрихов: <b>${st.strokes}</b></span>` +
    `<span>Подъёмов: <b>${st.lifts}</b></span>` +
    `<span>Время ≈ <b>${fmtTime(st.time_s)}</b> <span class="kv">(без разгонов)</span></span>`;
  const e = P.end;
  if (e && e.last_word) {
    let s = `Записано: слова ${e.first_word}–${e.last_word}. Последняя буква: слово ${e.last_word}, буква ${e.last_letter}. `;
    s += e.next_word ? `<b>Следующий лист: слово ${e.next_word}${e.next_letter > 1 ? ", буква " + e.next_letter : ""}.</b>` : "Текст закончился на этом листе.";
    if (P.resume) {
      s += ` <b style="color:#2f5fb3">В gcode: продолжение со слова ${P.resume.word}, буквы ${P.resume.letter}` +
           `${P.resume.connected ? " (с точки связки с предыдущей буквой)" : ""}; написанное серым.</b>`;
    }
    $("endInfo").innerHTML = s;
  } else $("endInfo").textContent = "";
  $("resumeInfo").textContent = P.resume
    ? `продолжение со слова ${P.resume.word}, буквы ${P.resume.letter}` : (S.text_options.resume_word ? "" : "пишется весь лист");
  $("wordCount").textContent = P.word_count != null ? `Слов в тексте: ${P.word_count}` : "";
  showMessages(P.errors, P.warnings);
  $("btnGcode").disabled = P.errors.length > 0;
  $("btnGcode").title = P.errors.length ? "Сначала исправь ошибки внизу" : "";
}

function showMessages(errors, warnings) {
  $("messages").innerHTML =
    errors.map((m) => `<div class="msg err">⛔ ${esc(m)}</div>`).join("") +
    warnings.map((m) => `<div class="msg warn">⚠ ${esc(m)}</div>`).join("");
}

function renderFontInfo() {
  const f = P.font;
  if (!f) { $("fontInfo").textContent = ""; return; }
  let h = `${esc(f.name)}: ${f.glyph_count} глифов, высота строчной ${f.x_height?.toFixed(3)} em (${esc(f.x_height_source)}).`;
  if (f.mode === "outlines") {
    h += `<br>GSUB: ${f.features?.length ? `<span class="tags">${f.features.map((t) => `<span>${esc(t)}</span>`).join("")}</span>` : "нет"}`;
    if (f.gpos_features?.length) h += ` · GPOS: ${f.gpos_features.map(esc).join(", ")}`;
  }
  const src = f.variant_sources || {};
  const variants = Object.entries(f.variants || {});
  const ligs = f.ligatures || [];
  if (variants.length || ligs.length) {
    h += `<details style="border:0;margin:4px 0;background:none"><summary style="padding:2px 0;font-weight:500">` +
         `Варианты: ${variants.length} символов · лигатуры: ${ligs.length}</summary>`;
    h += `<div class="hint">Варианты выбираются случайно, если включены «Случайные варианты букв».</div>`;
    h += variants.map(([c, names]) => {
      const alts = names.slice(1).map((n) => `${esc(n)}${src[c]?.[n] ? " <i>(" + src[c][n].map(esc).join(", ") + ")</i>" : ""}`);
      return `<b>${esc(c)}</b>: ${alts.join("; ")}`;
    }).join("<br>");
    if (ligs.length) h += `<br><b>Лигатуры:</b> ` + ligs.map((l) => `${esc(l.chars)} → ${esc(l.glyph)} (${esc(l.feature)})`).join(", ");
    h += `</details>`;
  } else h += "<br>Вариантов букв и лигатур нет.";
  if (f.notes?.length) h += "<br>" + f.notes.map(esc).join("<br>");
  $("fontInfo").innerHTML = h;
}

function renderMissing() {
  const box = $("missingBox");
  const list = P.missing || [];
  if (!list.length) { box.innerHTML = ""; return; }
  box.innerHTML = `<div class="hint"><b>Символов нет в шрифте.</b> Реши для каждого, иначе gcode не создаётся.</div>`;
  for (const m of list) {
    const choice = S.text_options.missing[m.char];
    const val = choice ? choice.action : "";
    const div = document.createElement("div");
    div.className = "missing" + (m.resolved ? " ok" : "");
    const pos = m.positions.slice(0, 6).map((p) => `стр. ${p.line}, слово ${p.word}, буква ${p.letter}`).join("; ");
    div.innerHTML = `
      <div class="row"><span class="ch">${esc(m.char === " " ? "␣" : m.char)}</span>
        <span class="kv">${m.code} · ${esc(m.name.toLowerCase())} · ${m.count} раз</span></div>
      <div class="pos">${esc(pos)}${m.positions.length > 6 ? "…" : ""}</div>
      <div class="row">
        <select><option value="">не решено</option><option value="skip">пропустить</option><option value="replace">заменить на</option></select>
        <input type="text" style="width:80px" placeholder="текст" value="${esc(choice?.replacement || "")}">
      </div>`;
    const sel = div.querySelector("select"), inp = div.querySelector("input");
    sel.value = val;
    inp.style.display = val === "replace" ? "" : "none";
    const apply = () => {
      if (!sel.value) delete S.text_options.missing[m.char];
      else S.text_options.missing[m.char] = { action: sel.value, replacement: inp.value };
      inp.style.display = sel.value === "replace" ? "" : "none";
      changed("full");
    };
    sel.addEventListener("change", apply);
    inp.addEventListener("change", apply);
    box.appendChild(div);
  }
}

let FONTS = [];
async function loadFonts() {
  const fonts = await api("GET", "/api/fonts");
  FONTS = fonts;
  const sel = $("fontSelect");
  sel.innerHTML = "";
  let found = false;
  for (const f of fonts) {
    const o = document.createElement("option");
    o.value = f.spec; o.textContent = f.label + (f.mode === "outlines" ? "  [контуры]" : "  [штрихи]");
    if (f.spec === S.font) found = true;
    sel.appendChild(o);
  }
  if (!found && S.font) {
    const o = document.createElement("option");
    o.value = S.font; o.textContent = S.font + " (не найден)";
    sel.appendChild(o);
  }
  sel.value = S.font;
}
$("fontSelect").addEventListener("change", (e) => {
  S.font = e.target.value;
  const f = FONTS.find((f) => f.spec === S.font);
  if (f && f.mode !== S.mode) { S.mode = f.mode; bindFields(); }
  changed("full");
});
$("btnUploadFont").onclick = () => $("fileFont").click();
$("btnUploadFolder").onclick = () => $("fileFolder").click();

async function uploadFiles(files, name) {
  const payload = { name, files: [] };
  for (const f of files) {
    if (/\.(ttf|otf)$/i.test(f.name)) {
      const bytes = new Uint8Array(await f.arrayBuffer());
      let bin = "";
      for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      payload.files.push({ filename: f.name, content_b64: btoa(bin) });
    } else if (/\.(svg|json)$/i.test(f.name)) {
      payload.files.push({ filename: f.name, content: await f.text() });
    }
  }
  try {
    $("busy").textContent = "загружаю шрифт…";
    const r = await api("POST", "/api/fonts/upload", payload);
    S.font = r.spec;
    S.mode = r.mode;
    bindFields();
    await loadFonts();
    changed("full");
    if (r.notes?.length) alert(r.notes.join("\n"));
  } catch (e) { $("busy").textContent = ""; alert("Шрифт не загружен: " + e.message); }
}
$("fileFont").addEventListener("change", (e) => {
  const f = e.target.files[0]; if (f) uploadFiles([f], f.name.replace(/\.(svg|ttf|otf)$/i, ""));
  e.target.value = "";
});
$("fileFolder").addEventListener("change", (e) => {
  const files = [...e.target.files];
  if (files.length) {
    const rel = files[0].webkitRelativePath || "font";
    uploadFiles(files, rel.split("/")[0]);
  }
  e.target.value = "";
});

$("btnLoadTxt").onclick = () => $("fileTxt").click();
$("fileTxt").addEventListener("change", async (e) => {
  const f = e.target.files[0]; if (!f) return;
  const buf = await f.arrayBuffer();
  let text;
  try { text = new TextDecoder("utf-8", { fatal: true }).decode(buf); }
  catch { alert("Файл не в UTF-8. Сохрани его в кодировке UTF-8 и загрузи снова."); return; }
  S.text = text.replace(/^﻿/, "");
  $("text").value = S.text;
  e.target.value = "";
  changed("full");
});

function renderProfiles() {
  const sel = $("profileSelect");
  sel.innerHTML = "";
  for (const name of Object.keys(S.profiles)) {
    const o = document.createElement("option"); o.value = o.textContent = name; sel.appendChild(o);
  }
  if (!S.profiles[S.active_profile]) {
    const o = document.createElement("option"); o.value = ""; o.textContent = "(свои параметры)"; sel.appendChild(o);
    sel.value = "";
  } else sel.value = S.active_profile;
}
function applyProfile(name) {
  const p = S.profiles[name]; if (!p) return;
  S.active_profile = name;
  S.sheet = JSON.parse(JSON.stringify(p.sheet));
  S.typography.size_mm = p.size_mm;
  S.typography.baseline_shift = p.baseline_shift;
  bindFields(); changed("full");
}
$("profileSelect").addEventListener("change", (e) => applyProfile(e.target.value));
function currentProfile() {
  return { sheet: JSON.parse(JSON.stringify(S.sheet)), size_mm: S.typography.size_mm, baseline_shift: S.typography.baseline_shift };
}
$("btnProfileSave").onclick = () => {
  const name = S.active_profile;
  if (!S.profiles[name]) { $("btnProfileNew").click(); return; }
  S.profiles[name] = currentProfile(); scheduleSave();
};
$("btnProfileNew").onclick = () => {
  const name = (prompt("Название профиля:") || "").trim(); if (!name) return;
  S.profiles[name] = currentProfile(); S.active_profile = name; renderProfiles(); scheduleSave();
};
$("btnProfileDel").onclick = () => {
  const name = S.active_profile;
  if (!S.profiles[name] || !confirm(`Удалить профиль «${name}»?`)) return;
  delete S.profiles[name]; S.active_profile = ""; renderProfiles(); scheduleSave();
};
$("btnProfileReset").onclick = async () => {
  if (!confirm("Вернуть стандартные профили (клетка, линейка, A4)? Свои профили останутся.")) return;
  const d = await api("GET", "/api/defaults");
  Object.assign(S.profiles, d.profiles); renderProfiles(); scheduleSave();
};

async function download(name, text) {
  if (window.showSaveFilePicker) {
    try {
      const h = await window.showSaveFilePicker({
        suggestedName: name, startIn: "downloads",
        types: [{ description: "Gcode", accept: { "text/plain": [".gcode"] } }],
      });
      const w = await h.createWritable();
      await w.write(text);
      await w.close();
      $("busy").textContent = `сохранено: ${h.name}`;
      setTimeout(() => ($("busy").textContent = ""), 4000);
      return;
    } catch (e) {
      if (e && e.name === "AbortError") return;
      console.warn("showSaveFilePicker:", e);
    }
  }
  const blob = new Blob([text], { type: "text/plain" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob); a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}
$("btnGcode").onclick = async () => {
  try {
    const r = await api("POST", "/api/gcode", S);
    await download(r.filename, r.gcode);
  } catch (e) { showMessages(e.data?.errors || [e.message], P?.warnings || []); }
};
$("btnTest").onclick = async () => {
  try {
    const r = await api("POST", "/api/testfile", S);
    await download(r.filename, r.gcode);
  } catch (e) { showMessages(e.data?.errors || [e.message], P?.warnings || []); }
};
async function loadTest() {
  const r = await api("POST", "/api/testfile/preview", S);
  TEST = r.strokes;
}
$("showTest").addEventListener("change", async (e) => {
  if (e.target.checked) await loadTest(); else TEST = null;
  draw();
});

const cv = $("cv"), ctx = cv.getContext("2d");

const view = { k: 3, ox: 40, oy: 600, auto: true };

function resize() {
  const r = cv.getBoundingClientRect(), d = window.devicePixelRatio || 1;
  cv.width = Math.round(r.width * d); cv.height = Math.round(r.height * d);
  if (view.auto && S) fit();
  draw();
}
function fit() {
  const r = cv.getBoundingClientRect();
  const W = S.sheet.width, H = S.sheet.height, pad = 24;
  view.k = Math.max(0.1, Math.min((r.width - 2 * pad) / W, (r.height - 2 * pad) / H));
  view.ox = (r.width - W * view.k) / 2;
  view.oy = (r.height + H * view.k) / 2;
}
const sx = (x) => view.ox + x * view.k;
const sy = (y) => view.oy - y * view.k;

function textTransform() {
  const t = S.typography, a = (t.rotation_deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  return (x, y) => [c * x - s * y + t.dx, s * x + c * y + t.dy];
}

function line(x1, y1, x2, y2) { ctx.beginPath(); ctx.moveTo(sx(x1), sy(y1)); ctx.lineTo(sx(x2), sy(y2)); ctx.stroke(); }

function draw() {
  if (!S) return;
  const d = window.devicePixelRatio || 1;
  const r = cv.getBoundingClientRect();

  if (cv.width !== Math.round(r.width * d) || cv.height !== Math.round(r.height * d)) {
    cv.width = Math.round(r.width * d); cv.height = Math.round(r.height * d);
    if (view.auto) fit();
  }
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  ctx.setTransform(d, 0, 0, d, 0, 0);
  const sh = S.sheet, W = sh.width, H = sh.height;

  ctx.save();
  ctx.shadowColor = "rgba(0,0,0,.18)"; ctx.shadowBlur = 12; ctx.shadowOffsetY = 2;
  ctx.fillStyle = cssVar("--paper"); ctx.fillRect(sx(0), sy(H), W * view.k, H * view.k);
  ctx.restore();

  const bases = P?.baselines?.map((b) => b - S.typography.baseline_shift) || [];
  if (S.preview.show_ruling) {
    ctx.save();
    ctx.beginPath(); ctx.rect(sx(0), sy(H), W * view.k, H * view.k); ctx.clip();
    ctx.lineWidth = 1; ctx.strokeStyle = "#b9cdf0";
    if (sh.ruling === "grid" && sh.grid_step > 0 && sh.grid_step * view.k > 3) {
      const y0 = H - sh.first_line_top;
      for (let y = y0 % sh.grid_step; y <= H; y += sh.grid_step) line(0, y, W, y);
      for (let x = sh.margin_left % sh.grid_step; x <= W; x += sh.grid_step) line(x, 0, x, H);
    } else if (sh.ruling === "lines") {
      for (const b of bases) line(0, b, W, b);
    }
    ctx.restore();
  }

  ctx.save();
  ctx.lineWidth = 1;
  ctx.strokeStyle = "#e39a9a";
  line(sh.margin_left, 0, sh.margin_left, H);
  line(W - sh.margin_right, 0, W - sh.margin_right, H);
  ctx.setLineDash([5, 4]); ctx.strokeStyle = "#9b9488";
  const top = H - sh.first_line_top;
  ctx.strokeRect(sx(sh.margin_left), sy(top), (W - sh.margin_left - sh.margin_right) * view.k, (top - sh.bottom_limit) * view.k);
  ctx.restore();
  if (S.preview.show_ruling && sh.ruling !== "lines") {
    ctx.save(); ctx.strokeStyle = "rgba(47,95,179,.25)"; ctx.lineWidth = 1;
    for (const b of P?.baselines || []) line(sh.margin_left, b, W - sh.margin_right, b);
    ctx.restore();
  }

  if (P && $("showTravelBox").checked) {
    const tb = P.travel_box, m = P.safety_margin;
    const fx = P.flip_x ? -1 : 1, fy = P.flip_y ? -1 : 1;
    const xs = [tb.x_min * fx, tb.x_max * fx], ys = [tb.y_min * fy, tb.y_max * fy];
    const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
    ctx.save();
    ctx.setLineDash([8, 5]); ctx.lineWidth = 1.2; ctx.strokeStyle = tb.measured ? "#c0392b" : "#d98c1f";
    ctx.strokeRect(sx(x0 + m), sy(y1 - m), (x1 - x0 - 2 * m) * view.k, (y1 - y0 - 2 * m) * view.k);
    ctx.fillStyle = ctx.strokeStyle; ctx.font = "12px system-ui";
    ctx.fillText(tb.measured ? "ход карандаша (с запасом)" : "ход не измерен", sx(x0 + m) + 4, sy(y1 - m) - 5);
    ctx.restore();
  }

  ctx.save(); ctx.fillStyle = "#2f5fb3";
  ctx.beginPath(); ctx.arc(sx(0), sy(0), 5, 0, Math.PI * 2); ctx.fill();
  ctx.font = "12px system-ui"; ctx.fillText("0,0", sx(0) + 7, sy(0) - 6);
  ctx.restore();

  if (!P) return;
  const lw = Math.max(1, 0.35 * view.k);

  if (S.preview.show_travel && P.travel.length) {
    ctx.save(); ctx.setLineDash([4, 4]); ctx.lineWidth = 0.8; ctx.strokeStyle = "rgba(226,120,30,.7)";
    ctx.beginPath();
    for (const [a, b] of P.travel) { ctx.moveTo(sx(a[0]), sy(a[1])); ctx.lineTo(sx(b[0]), sy(b[1])); }
    ctx.stroke(); ctx.restore();
  }

  for (const done of [true, false]) {
    ctx.save(); ctx.lineWidth = lw; ctx.lineCap = "round"; ctx.lineJoin = "round";
    ctx.strokeStyle = cssVar(done ? "--ink-done" : "--ink");
    ctx.beginPath();
    for (const st of P.strokes) {
      if (!!st.d !== done) continue;
      const p = st.p;
      ctx.moveTo(sx(p[0][0]), sy(p[0][1]));
      if (p.length === 1) ctx.lineTo(sx(p[0][0]) + 0.01, sy(p[0][1]));
      for (let i = 1; i < p.length; i++) ctx.lineTo(sx(p[i][0]), sy(p[i][1]));
    }
    ctx.stroke(); ctx.restore();
  }

  const T = textTransform(), xh = S.typography.size_mm;
  const box = (g) => {
    let pts;
    if (g.b) {
      const pad = Math.max(0.4, xh * 0.08), [x0, y0, x1, y1] = g.b;
      pts = [[x0 - pad, y0 - pad], [x1 + pad, y0 - pad], [x1 + pad, y1 + pad], [x0 - pad, y1 + pad]];
    } else {
      pts = [[g.x, g.y - xh * 0.5], [g.x + Math.max(g.adv, xh * 0.3), g.y - xh * 0.5],
             [g.x + Math.max(g.adv, xh * 0.3), g.y + xh * 1.5], [g.x, g.y + xh * 1.5]].map(([x, y]) => T(x, y));
    }
    ctx.beginPath(); pts.forEach(([x, y], i) => (i ? ctx.lineTo(sx(x), sy(y)) : ctx.moveTo(sx(x), sy(y)))); ctx.closePath();
  };
  ctx.save();
  for (const g of P.glyphs) {
    if (g.missing) { box(g); ctx.fillStyle = "rgba(179,38,30,.18)"; ctx.fill(); ctx.strokeStyle = "#b3261e"; ctx.lineWidth = 1; ctx.stroke(); }
  }
  const e = P.end;
  if (e && e.last_word) {
    const g = P.glyphs.find((g) => g.w === e.last_word && g.l === e.last_letter && !g.h);
    if (g) { box(g); ctx.fillStyle = "rgba(255,200,0,.35)"; ctx.fill(); }
  }
  const rw = S.text_options.resume_word, rl = S.text_options.resume_letter;
  if (rw) {
    for (const g of P.glyphs.filter((g) => g.w === rw && g.l === rl && !g.h)) {
      box(g); ctx.strokeStyle = "#2f5fb3"; ctx.lineWidth = 2; ctx.setLineDash([]); ctx.stroke();
      ctx.fillStyle = "rgba(47,95,179,.12)"; ctx.fill();
    }
  }
  ctx.restore();

  if (TEST) {
    ctx.save(); ctx.strokeStyle = "#2f5fb3"; ctx.lineWidth = Math.max(1, lw); ctx.lineCap = "round";
    ctx.beginPath();
    for (const p of TEST) { ctx.moveTo(sx(p[0][0]), sy(p[0][1])); for (let i = 1; i < p.length; i++) ctx.lineTo(sx(p[i][0]), sy(p[i][1])); }
    ctx.stroke(); ctx.restore();
  }
}

cv.addEventListener("wheel", (e) => {
  e.preventDefault();
  const r = cv.getBoundingClientRect(), mx = e.clientX - r.left, my = e.clientY - r.top;
  const f = Math.exp(-e.deltaY * 0.0015);
  const k = Math.min(200, Math.max(0.2, view.k * f));
  const wx = (mx - view.ox) / view.k, wy = (view.oy - my) / view.k;
  view.k = k; view.ox = mx - wx * k; view.oy = my + wy * k; view.auto = false;
  draw();
}, { passive: false });
let drag = null;
cv.addEventListener("mousedown", (e) => { drag = { x: e.clientX, y: e.clientY, ox: view.ox, oy: view.oy }; cv.classList.add("drag"); });
window.addEventListener("mouseup", (e) => {

  if (drag && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 4 && e.target === cv) pickLetter(e);
  drag = null; cv.classList.remove("drag");
});

function glyphAt(x, y) {
  if (!P || !P.glyphs) return null;
  let best = null, bestScore = Infinity;
  const xh = S.typography.size_mm;
  for (const g of P.glyphs) {
    if (g.h) continue;
    let cx, cy, inside = false, area = 0;
    if (g.b) {
      const [x0, y0, x1, y1] = g.b, pad = xh * 0.1;
      inside = x >= x0 - pad && x <= x1 + pad && y >= y0 - pad && y <= y1 + pad;
      cx = (x0 + x1) / 2; cy = (y0 + y1) / 2; area = (x1 - x0) * (y1 - y0);
    } else {
      cx = g.x + g.adv / 2; cy = g.y + xh / 2;
    }
    const d = Math.hypot(x - cx, y - cy);
    const score = inside ? area * 1e-3 + d * 1e-3 : 1e6 + d;
    if (score < bestScore && (inside || d < xh * 1.2)) { best = g; bestScore = score; }
  }
  return best;
}
function pickLetter(e) {
  const r = cv.getBoundingClientRect();
  const x = (e.clientX - r.left - view.ox) / view.k, y = (view.oy - (e.clientY - r.top)) / view.k;
  const g = glyphAt(x, y);
  if (!g) { $("picked").textContent = ""; return; }
  $("picked").innerHTML = `<b>слово ${g.w}, буква ${g.l}</b> «${esc(g.c)}» → продолжить с неё`;
  S.text_options.resume_word = g.w;
  S.text_options.resume_letter = g.l;
  bindFields();
  changed("full");
}
window.addEventListener("mousemove", (e) => {
  const r = cv.getBoundingClientRect();
  if (drag) { view.ox = drag.ox + e.clientX - drag.x; view.oy = drag.oy + e.clientY - drag.y; view.auto = false; draw(); }
  if (e.target === cv) {
    const x = (e.clientX - r.left - view.ox) / view.k, y = (view.oy - (e.clientY - r.top)) / view.k;
    $("cursorPos").textContent = `X ${x.toFixed(1)}  Y ${y.toFixed(1)} мм`;
  }
});
$("btnFit").onclick = () => { view.auto = true; fit(); draw(); };
$("btnResumeOff").onclick = () => {
  S.text_options.resume_word = 0; S.text_options.resume_letter = 1; $("picked").textContent = "";
  bindFields(); changed("full");
};
$("btnSeed").onclick = () => {
  S.randomness.seed = Math.floor(Math.random() * 1e6);
  bindFields(); changed("full");
};
$("btnRandomDefaults").onclick = () => {
  for (const p of RANDOM_PARAMS) S.randomness[p.key] = p.def;
  bindFields(); changed("full");
};
$("showTravelBox").addEventListener("change", draw);

new ResizeObserver(() => resize()).observe($("canvasWrap"));

window.addEventListener("focus", async () => {
  if (!S) return;
  try {
    const fresh = await api("GET", "/api/settings");
    S.drawing = fresh.drawing;
    if (!savePending && JSON.stringify(fresh.printer) !== JSON.stringify(S.printer)) {
      S.printer = fresh.printer; bindFields(); refreshPreview();
    }
    if (JSON.stringify(fresh.outline) !== JSON.stringify(S.outline) || fresh.font !== S.font || fresh.mode !== S.mode) {
      S.outline = fresh.outline; S.font = fresh.font; S.mode = fresh.mode;
      bindFields(); $("fontSelect").value = S.font; refreshPreview();
    }
  } catch {  }
});

(async function init() {
  S = await api("GET", "/api/settings");
  bindFields();
  renderProfiles();
  await loadFonts();
  resize();
  await refreshPreview();
})();
addEventListener("themechange", () => { if (typeof P !== "undefined" && P) draw(); });
