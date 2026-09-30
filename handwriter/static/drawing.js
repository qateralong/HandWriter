"use strict";

let S = null;
let P = null;
let lastTravel = { x_min: 0, x_max: 200, y_min: 0, y_max: 200 };
const $ = (id) => document.getElementById(id);

function getPath(obj, path) { return path.split(".").reduce((o, k) => (o == null ? o : o[k]), obj); }
function setPath(obj, path, v) {
  const ks = path.split("."); let o = obj;
  for (let i = 0; i < ks.length - 1; i++) o = o[ks[i]];
  o[ks[ks.length - 1]] = v;
}
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }

function bindFields() {
  document.querySelectorAll("[data-path]").forEach((el) => {
    if (el === document.activeElement && el.type === "number") return;
    const v = getPath(S, el.dataset.path);
    if (el.type === "checkbox") el.checked = !!v; else el.value = v ?? "";
  });
  const t = S.printer.travel;
  $("travelMeasured").checked = !!t;
  if (t) lastTravel = { ...t };
  document.querySelectorAll("[data-travel]").forEach((el) => { el.value = lastTravel[el.dataset.travel]; el.disabled = !t; });
  document.querySelectorAll("[data-table]").forEach((el) => {
    if (el === document.activeElement) return;
    const v = S.printer.table[el.dataset.table]; el.value = v == null ? "" : v;
  });
  document.querySelectorAll("input[name=scaleMode]").forEach((r) => (r.checked = r.value === S.drawing.placement.scale_mode));
  const custom = S.drawing.sheet.format === "custom";
  document.querySelectorAll("label.custom input").forEach((el) => (el.disabled = !custom));
  $("fileSelect").value = S.drawing.file;
  document.querySelector('[data-path="drawing.placement.percent"]').disabled = S.drawing.placement.scale_mode !== "percent";
  document.querySelector('[data-path="drawing.imp.threshold"]').disabled = S.drawing.imp.threshold_auto;
  const w = S.drawing.weights;
  $("passHint").textContent = w.enabled
    ? `Толстая линия = ${w.passes} прохода со смещениями ${passOffsets(w).map((d) => (d > 0 ? "+" : "") + d.toFixed(2)).join(", ")} мм.`
    : "Выключено: каждая линия рисуется одним проходом.";
}
function passOffsets(w) { return Array.from({ length: w.passes }, (_, k) => (k - (w.passes - 1) / 2) * w.step); }

function onFieldChange(el) {
  const path = el.dataset.path;
  let v;
  if (el.type === "checkbox") v = el.checked;
  else if (el.type === "number") {
    if (el.value === "" || isNaN(Number(el.value))) return;
    v = Number(el.value);
    if (/pdf_page|passes|threshold$/.test(path)) v = Math.round(v);
  } else v = el.value;
  setPath(S, path, v);
  bindFields();
  changed(path === "drawing.show_travel" ? "redraw" : "full");
}
document.addEventListener("input", (e) => {
  const el = e.target;
  if (el.dataset && el.dataset.path && el.tagName !== "SELECT") onFieldChange(el);
  if (el.dataset && el.dataset.travel) {
    if (el.value === "" || isNaN(Number(el.value))) return;
    lastTravel[el.dataset.travel] = Number(el.value);
    if (S.printer.travel) S.printer.travel = { ...lastTravel };
    changed("full");
  }
  if (el.dataset && el.dataset.table) {
    if (el.value !== "" && isNaN(Number(el.value))) return;
    S.printer.table[el.dataset.table] = el.value === "" ? null : Number(el.value);
    changed("full");
  }
  if (el.dataset && el.dataset.reading) wizardUpdate();
});
document.addEventListener("change", (e) => {
  const el = e.target;
  if (el.tagName === "SELECT" && el.dataset.path) onFieldChange(el);
});
$("travelMeasured").addEventListener("change", (e) => {
  S.printer.travel = e.target.checked ? { ...lastTravel } : null;
  bindFields(); changed("full");
});
document.querySelectorAll("input[name=scaleMode]").forEach((r) => r.addEventListener("change", () => {
  if (!r.checked) return;
  S.drawing.placement.scale_mode = r.value;
  if (r.value === "percent" && P?.scale) S.drawing.placement.percent = Number(P.scale.percent.toFixed(1));
  bindFields(); changed("full");
}));
$("btnReach").onclick = () => {
  S.drawing.placement.scale_mode = "fit_reach";
  S.drawing.placement.dx = 0; S.drawing.placement.dy = 0;
  bindFields(); changed("full");
};
$("btnPasses").onclick = () => {
  S.drawing.placement.scale_mode = "fit_passes";
  bindFields(); changed("full");
};
$("btnFrameDefaults").onclick = () => {
  Object.assign(S.drawing.frame, { left: 20, right: 5, top: 5, bottom: 5, tb_width: 185, tb_height: 55 });
  bindFields(); changed("full");
};

let previewTimer = null, saveTimer = null, previewSeq = 0, savePending = false;
function changed(kind) {
  scheduleSave();
  if (kind === "redraw") { draw(); return; }
  clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 250);
}
function scheduleSave() {
  savePending = true;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(flushSave, 700);
}
async function flushSave() {
  clearTimeout(saveTimer);
  if (!savePending || !S) return;
  savePending = false;
  try { await api("PUT", "/api/settings", S); } catch {  }
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
    const data = await api("POST", "/api/drawing/preview", S);
    if (seq !== previewSeq) return;
    P = data;
    renderStatus(); renderFileInfo(); renderLayers(); renderPassInfo(); renderParts();
    if (!$("wizard").hidden) wizardUpdate();
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
function showMessages(errors, warnings) {
  $("messages").innerHTML =
    errors.map((m) => `<div class="msg err">⛔ ${esc(m)}</div>`).join("") +
    warnings.map((m) => `<div class="msg warn">⚠ ${esc(m)}</div>`).join("");
}
function renderStatus() {
  const st = P.stats;
  $("stats").innerHTML =
    `<span>Путь карандаша: <b>${st.draw_mm} мм</b></span>` +
    `<span>Холостые: <b>${st.travel_mm} мм</b></span>` +
    `<span>Штрихов: <b>${st.strokes}</b></span>` +
    `<span>Подъёмов: <b>${st.lifts}</b></span>` +
    `<span>Время ≈ <b>${fmtTime(st.time_s)}</b> <span class="kv">(без разгонов)</span></span>`;
  showMessages(P.errors, P.warnings);
  $("btnGcode").title = P.errors.length ? "Сначала исправь ошибки внизу" : "";
  const sc = P.scale;
  if (sc) {
    let h = `Масштаб <b>${esc(sc.label)}</b> (${sc.percent}%)`;
    if (sc.paper_size) h += `, на бумаге ${sc.paper_size[0]}×${sc.paper_size[1]} мм`;
    if (P.sheet) h += `<br><span class="kv">Лист ${P.sheet.width}×${P.sheet.height} мм, ${P.sheet.orientation === "portrait" ? "книжная" : "альбомная"}.`;
    if (sc.reach_percent) h += ` В один проход влезает до ${sc.reach_percent}%.`;
    if (sc.passes_percent) h += ` Проходы покроют чертёж до ${sc.passes_percent}%.`;
    h += `</span>`;
    $("scaleInfo").innerHTML = h;
  } else $("scaleInfo").textContent = "";
}

const PASS_COLORS = { 0: [47, 95, 179], 90: [39, 150, 90], 180: [214, 120, 20], 270: [140, 70, 180] };
const rgba = (r, a) => `rgba(${PASS_COLORS[r].join(",")},${a})`;
const CORNER_NAME = { A: "левый нижний", B: "правый нижний", C: "правый верхний", D: "левый верхний" };

function renderPassInfo() {
  const ps = P.passes;
  if (!ps) { $("passInfo").innerHTML = ""; return; }
  const sw = (r) => `<span class="sw" style="background:${rgba(r, .55)}"></span>`;
  const desc = (r) => `${sw(r)}${r}°, в упоре угол ${ps.corner_of[r]} (${CORNER_NAME[ps.corner_of[r]]})`;
  let h = "";
  if (ps.rotation != null && ps.rotation !== 0)
    h += `<div class="turn">Чертёж рисуется за один проход с поворотом листа ${esc(ps.rotation_text)}. Поставь этот угол в упор, ноль — в нём.</div>`;
  const sp = ps.sheet_plan;
  if (sp.rotations) {
    h += `<div><b>Рабочее поле листа:</b> проходов ${sp.rotations.length} — ${sp.rotations.map(desc).join("; ")}.</div>`;
  } else {
    h += `<div class="msg warn">⚠ ${esc(sp.message)}</div>`;
  }
  if (ps.drawing) {
    h += `<div><b>Чертёж:</b> ${ps.drawing.length === 1 ? "один проход" : "проходов " + ps.drawing.length} — ${ps.drawing.map(desc).join("; ")}.</div>`;
  }
  if (ps.notes.length) h += `<div class="kv">Недопустимые повороты: ${ps.notes.map(esc).join("; ")}.</div>`;
  $("passInfo").innerHTML = h;
}

function renderFileInfo() {
  const im = P.import;
  const kind = im?.kind || "";
  $("pdfBox").style.display = kind === "pdf" ? "" : "none";
  $("rasterBox").style.display = kind === "raster" ? "" : "none";
  $("unitsBox").style.display = kind === "svg" || kind === "dxf" ? "" : "none";
  $("fillBox").style.display = kind === "raster" ? "none" : "";
  if (!im) { $("fileInfo").textContent = ""; return; }
  const names = { svg: "SVG", dxf: "DXF", pdf: "PDF", raster: "растровая картинка", test: "встроенный SVG" };
  let h = `${esc(im.name)}: ${names[im.kind] || im.kind}`;
  if (im.size) h += `, ${im.size[0]}×${im.size[1]} мм при 1:1`;
  h += `, путей ${im.paths}.`;
  if (im.units_note) h += `<br>Единицы: ${esc(im.units_note)}.`;
  if (im.kind === "raster" && im.info) h += `<br>${im.info.width_px}×${im.info.height_px} px, порог ${im.info.threshold}.`;
  $("fileInfo").innerHTML = h;
  $("pdfPages").textContent = kind === "pdf" ? `из ${im.pages}` : "";
}
function renderLayers() {
  const box = $("layersBox");
  const layers = P.import?.layers || [];
  if (!layers.length) {
    box.innerHTML = P.import?.has_widths ? `<div class="hint">Толщина берётся из stroke-width файла.</div>`
      : `<div class="hint">В файле нет толщин линий: всё рисуется одним проходом, если слоям не задано иначе.</div>`;
    return;
  }
  const opt = (v, cur, t) => `<option value="${v}"${v === cur ? " selected" : ""}>${t}</option>`;
  box.innerHTML = `<div class="hint">Слои: толщина из файла или вручную.</div><table class="layers">` +
    layers.map((l) => {
      const cur = S.drawing.weights.layers[l.name] || "auto";
      return `<tr><td>${esc(l.name)}</td><td class="kv">${l.count} шт.${l.width != null ? ", " + l.width + " мм" : ""}</td>` +
        `<td><select data-layer="${esc(l.name)}">${opt("auto", cur, "по толщине")}${opt("thin", cur, "тонкая")}${opt("thick", cur, "толстая")}</select></td></tr>`;
    }).join("") + `</table>`;
  box.querySelectorAll("select[data-layer]").forEach((sel) => sel.addEventListener("change", () => {
    if (sel.value === "auto") delete S.drawing.weights.layers[sel.dataset.layer];
    else S.drawing.weights.layers[sel.dataset.layer] = sel.value;
    changed("full");
  }));
}

let FILES = [];
async function loadFiles() {
  FILES = await api("GET", "/api/drawing/files");
  const sel = $("fileSelect");
  sel.innerHTML = "";
  for (const f of FILES) {
    const o = document.createElement("option");
    o.value = f.spec; o.textContent = f.label; sel.appendChild(o);
  }
  if (!FILES.some((f) => f.spec === S.drawing.file)) {
    const o = document.createElement("option");
    o.value = S.drawing.file; o.textContent = S.drawing.file + " (не найден)"; sel.appendChild(o);
  }
  sel.value = S.drawing.file;
}
$("fileSelect").addEventListener("change", (e) => {
  S.drawing.file = e.target.value;
  S.drawing.imp.pdf_page = 1;
  S.drawing.weights.layers = {};
  view.auto = true;
  changed("full");
});
$("btnUpload").onclick = () => $("fileInput").click();
$("fileInput").addEventListener("change", async (e) => {
  const f = e.target.files[0]; e.target.value = "";
  if (!f) return;
  const bytes = new Uint8Array(await f.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  try {
    $("busy").textContent = "загружаю…";
    const r = await api("POST", "/api/drawing/upload", { filename: f.name, content_b64: btoa(bin) });
    S.drawing.file = r.spec;
    S.drawing.imp.pdf_page = 1;
    S.drawing.weights.layers = {};
    await loadFiles();
    view.auto = true;
    bindFields(); changed("full");
  } catch (err) { $("busy").textContent = ""; alert("Файл не загружен: " + err.message); }
});

async function download(name, text) {
  if (window.showSaveFilePicker) {
    try {
      const h = await window.showSaveFilePicker({
        suggestedName: name, startIn: "downloads",
        types: [{ description: "Gcode", accept: { "text/plain": [".gcode"] } }],
      });
      const w = await h.createWritable();
      await w.write(text); await w.close();
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
function plainDownload(name, text) {
  const blob = new Blob([text], { type: "text/plain" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob); a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

async function saveFiles(files) {
  if (files.length === 1) return download(files[0].filename, files[0].gcode);
  if (window.showDirectoryPicker) {
    try {
      const dir = await window.showDirectoryPicker({ mode: "readwrite", startIn: "downloads" });
      for (const f of files) {
        const h = await dir.getFileHandle(f.filename, { create: true });
        const w = await h.createWritable(); await w.write(f.gcode); await w.close();
      }
      $("busy").textContent = `сохранено файлов: ${files.length} в папку «${dir.name}»`;
      setTimeout(() => ($("busy").textContent = ""), 6000);
      return;
    } catch (e) {
      if (e && e.name === "AbortError") return;
      console.warn("showDirectoryPicker:", e);
    }
  }
  for (const f of files) { plainDownload(f.filename, f.gcode); await new Promise((r) => setTimeout(r, 400)); }
}
async function getFiles(query) {
  try {
    const r = await api("POST", "/api/drawing/gcode" + query, S);
    await saveFiles(r.files);
  } catch (e) { showMessages(e.data?.errors || [e.message], P?.warnings || []); }
}
$("btnGcode").onclick = () => getFiles("");
$("btnTests").onclick = () => getFiles("?test=true");

function rotText(r) { return r === 0 ? "без поворота" : `поворот ${r}° против часовой`; }
function cardSvg(part) {

  const [W, H] = sheetWH(), r = part.rotation, [Wp, Hp] = part.pass_size;
  const k = Math.min(110 / Wp, 80 / Hp), px = (x) => 12 + x * k, py = (y) => 92 - y * k;
  const a = toPass([part.region[0], part.region[1]], r, W, H), b = toPass([part.region[2], part.region[3]], r, W, H);
  const cx0 = Math.max(0, Math.min(a[0], b[0])), cx1 = Math.min(Wp, Math.max(a[0], b[0]));
  const cy0 = Math.max(0, Math.min(a[1], b[1])), cy1 = Math.min(Hp, Math.max(a[1], b[1]));
  let s = `<rect x="${px(0)}" y="${py(Hp)}" width="${Wp * k}" height="${Hp * k}" fill="#fff" stroke="#888"/>`;
  s += `<rect x="${px(cx0)}" y="${py(cy1)}" width="${(cx1 - cx0) * k}" height="${(cy1 - cy0) * k}" fill="${rgba(r, .35)}"/>`;
  const win = P.passes.windows[r].safe;
  s += `<rect x="${px(Math.max(win[0], 0))}" y="${py(Math.min(win[3], Hp))}" width="${(Math.min(win[2], Wp) - Math.max(win[0], 0)) * k}" height="${(Math.min(win[3], Hp) - Math.max(win[1], 0)) * k}" fill="none" stroke="#c0392b" stroke-dasharray="3 2"/>`;
  s += `<path d="M${px(0) - 3} ${py(Hp * 0.4)} V${py(0) + 3} H${px(Wp * 0.4)}" fill="none" stroke="#333" stroke-width="3"/>`;
  s += `<circle cx="${px(0)}" cy="${py(0)}" r="3" fill="#2f5fb3"/>`;
  for (const [name, c] of Object.entries(P.passes.corners)) {
    const [x, y] = toPass(c, r, W, H);
    s += `<text x="${px(x) + (x > 0 ? 2 : -9)}" y="${py(y) + (y > 0 ? -2 : 10)}" font-size="9" font-weight="bold" fill="${name === part.corner ? rgba(r, 1) : "#666"}">${name}</text>`;
  }
  return `<svg viewBox="0 0 128 104">${s}</svg>`;
}
function renderParts() {
  const box = $("partCards"), parts = P.parts || [];
  const sel = $("viewMode"), cur = sel.value;
  sel.innerHTML = `<option value="0">лист, все проходы</option>` +
    parts.map((q) => `<option value="${q.index}">проход ${q.index} на столе (${q.rotation}°)</option>`).join("");
  sel.value = parts.some((q) => String(q.index) === cur) ? cur : "0";
  $("btnGcode").textContent = parts.length > 1 ? `Скачать ${parts.length} файла проходов` : "Скачать gcode";
  $("btnTests").textContent = parts.length > 1 ? "Тестовые файлы проходов" : "Тестовый файл прохода";
  $("btnTests").disabled = $("btnGcode").disabled = P.errors.length > 0 || !parts.length;
  if (!parts.length) { box.innerHTML = `<div class="hint">Проходов нет: сначала исправь ошибки внизу.</div>`; return; }
  box.innerHTML = parts.length > 1
    ? `<div class="hint">Запускай файлы по порядку. После каждого прохода поверни лист, поставь указанный угол в упоры и опусти карандаш в этот угол до касания (ноль).</div>` : "";
  parts.forEach((q, i) => {
    const prev = i ? parts[i - 1].rotation : 0;
    const turn = i === 0 ? rotText(q.rotation) + " относительно раскладки" : `от прохода ${i}: поверни ещё на ${((q.rotation - prev) % 360 + 360) % 360}° против часовой`;
    const st = q.stats, off = S.drawing.split.offsets[String(q.rotation)] || [0, 0];
    const div = document.createElement("div");
    div.className = "card"; div.style.borderLeftColor = rgba(q.rotation, .9);
    div.innerHTML = `
      <div class="top"><b>Проход ${q.index} из ${parts.length} · ${q.rotation}°</b><span class="kv">файл ${i + 1} по порядку</span></div>
      <div class="cbody">
        ${cardSvg(q)}
        <div>
          <div>Под карандаш (в упоры, ноль): <b>угол ${q.corner}</b> — ${q.corner_name}.</div>
          <div class="kv">Лист: ${esc(turn)}.</div>
          <div>≈ <b>${fmtTime(st.time_s)}</b>, путь ${st.draw_mm} мм, холостые ${st.travel_mm} мм, штрихов ${st.strokes}${q.marks ? `, крестиков ${q.marks}` : ""}.</div>
          <div class="kv" style="word-break:break-all">${esc(q.filename)}</div>
        </div>
      </div>
      <div class="row">
        <label class="c" title="Поправка нуля этого прохода по оси X принтера, мм">dx <input type="number" step="0.1" data-off="0" value="${off[0]}"></label>
        <label class="c" title="Поправка нуля этого прохода по оси Y принтера, мм">dy <input type="number" step="0.1" data-off="1" value="${off[1]}"></label>
        <button data-get="gcode">gcode</button><button data-get="test">тест</button>
      </div>
      ${q.errors.map((e) => `<div class="err">⛔ ${esc(e)}</div>`).join("")}`;
    div.querySelectorAll("input[data-off]").forEach((inp) => inp.addEventListener("change", () => {
      const v = [...div.querySelectorAll("input[data-off]")].map((x) => Number(x.value) || 0);
      if (v[0] || v[1]) S.drawing.split.offsets[String(q.rotation)] = v; else delete S.drawing.split.offsets[String(q.rotation)];
      changed("full");
    }));
    div.querySelector('[data-get="gcode"]').onclick = () => getFiles(`?part=${q.index}`);
    div.querySelector('[data-get="test"]').onclick = () => getFiles(`?part=${q.index}&test=true`);
    box.appendChild(div);
  });
}
$("viewMode").addEventListener("change", () => { view.auto = true; fit(); draw(); });

const cv = $("cv"), ctx = cv.getContext("2d");
const view = { k: 2, ox: 40, oy: 600, auto: true };
function sheetWH() { return P?.sheet ? [P.sheet.width, P.sheet.height] : [210, 297]; }
function viewBounds() {

  const part = tablePart();
  if (!part) { const [W, H] = sheetWH(); return [0, 0, W, H]; }
  const [Wp, Hp] = part.pass_size, raw = P.passes.windows[part.rotation].raw;
  return [Math.min(0, raw[0]) - 12, Math.min(0, raw[1]) - 12, Math.max(Wp, raw[2]) + 5, Math.max(Hp, raw[3]) + 5];
}
function fit() {
  const r = cv.getBoundingClientRect();
  const [x0, y0, x1, y1] = viewBounds(), pad = 24, W = x1 - x0, H = y1 - y0;
  view.k = Math.max(0.1, Math.min((r.width - 2 * pad) / W, (r.height - 2 * pad) / H));
  view.ox = (r.width - W * view.k) / 2 - x0 * view.k;
  view.oy = (r.height + H * view.k) / 2 + y0 * view.k;
}
function tablePart() {
  const k = Number($("viewMode").value || 0);
  return k > 0 && P?.parts?.[k - 1] ? P.parts[k - 1] : null;
}

function toPass([u, v], r, W, H) {
  if (r === 90) return [H - v, u];
  if (r === 180) return [W - u, H - v];
  if (r === 270) return [v, W - u];
  return [u, v];
}
function partColor(k, a) {
  const part = P?.parts?.[k - 1];
  return part ? rgba(part.rotation, a) : `rgba(29,29,31,${a})`;
}
function drawStrokes(T, only) {

  const showPasses = $("showPasses").checked;
  const groups = new Map();
  for (const st of P.strokes) {
    const key = `${st.k}|${st.t ? 1 : 0}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(st.p);
  }
  const keys = [...groups.keys()].sort((a, b) => (only && a.startsWith(only + "|") ? 1 : 0) - (only && b.startsWith(only + "|") ? 1 : 0));
  for (const key of keys) {
    const [k, t] = key.split("|").map(Number);
    ctx.save(); ctx.lineCap = "round"; ctx.lineJoin = "round";
    const lw = showPasses ? Math.max(0.8, 0.3 * view.k) : Math.max(1, 0.35 * view.k);
    ctx.lineWidth = t && !showPasses ? lw * 1.6 : lw;
    ctx.strokeStyle = only && k !== only ? "rgba(150,150,150,.35)" : partColor(k, k ? (t ? 1 : .9) : 1);
    ctx.beginPath();
    for (const p of groups.get(key)) poly(p.map(T));
    ctx.stroke(); ctx.restore();
  }

  ctx.save(); ctx.strokeStyle = "rgba(0,0,0,.55)"; ctx.lineWidth = 1;
  for (const m of P.marks || []) {
    const [x, y] = T([m.x, m.y]);
    ctx.beginPath(); ctx.arc(sx(x), sy(y), Math.max(6, S.drawing.split.mark_size * view.k), 0, Math.PI * 2); ctx.stroke();
  }
  ctx.restore();
}
function drawSeams(T) {
  if (!P.seams?.length) return;
  ctx.save();
  for (const sm of P.seams) {
    const ov = P.overlap;
    const a = sm.axis === 0 ? [[sm.s - ov, sm.span[0]], [sm.s + ov, sm.span[1]]] : [[sm.span[0], sm.s - ov], [sm.span[1], sm.s + ov]];
    const p0 = T(a[0]), p1 = T(a[1]);
    ctx.fillStyle = "rgba(0,0,0,.06)";
    ctx.fillRect(Math.min(sx(p0[0]), sx(p1[0])), Math.min(sy(p0[1]), sy(p1[1])), Math.abs(sx(p1[0]) - sx(p0[0])), Math.abs(sy(p1[1]) - sy(p0[1])));
    const b = sm.axis === 0 ? [[sm.s, sm.span[0]], [sm.s, sm.span[1]]] : [[sm.span[0], sm.s], [sm.span[1], sm.s]];
    const q0 = T(b[0]), q1 = T(b[1]);
    ctx.setLineDash([10, 4, 2, 4]); ctx.strokeStyle = "rgba(0,0,0,.55)"; ctx.lineWidth = 1.2;
    ctx.beginPath(); ctx.moveTo(sx(q0[0]), sy(q0[1])); ctx.lineTo(sx(q1[0]), sy(q1[1])); ctx.stroke();
    ctx.setLineDash([]); ctx.fillStyle = "rgba(0,0,0,.65)"; ctx.font = "11px system-ui";
    ctx.fillText(`шов ${sm.axis === 0 ? "x" : "y"} = ${sm.s.toFixed(1)}`, sx(q1[0]) + 4, sy(q1[1]) + 12);
  }
  ctx.restore();
}
function drawTable(part) {

  const [W, H] = sheetWH(), r = part.rotation, [Wp, Hp] = part.pass_size;
  const T = (p) => { const q = toPass(p, r, W, H); return [q[0] + part.dx, q[1] + part.dy]; };
  const win = P.passes.windows[r], tb = P.passes.table;
  const rect = ([x0, y0, x1, y1], fill, stroke, dash) => {
    ctx.save(); if (dash) ctx.setLineDash(dash);
    if (fill) { ctx.fillStyle = fill; ctx.fillRect(sx(x0), sy(y1), (x1 - x0) * view.k, (y1 - y0) * view.k); }
    if (stroke) { ctx.strokeStyle = stroke; ctx.lineWidth = 1.2; ctx.strokeRect(sx(x0), sy(y1), (x1 - x0) * view.k, (y1 - y0) * view.k); }
    ctx.restore();
  };
  rect([0, 0, tb.table_x, tb.table_y], "#e9ebee", tb.given ? "#9aa3ad" : null, null);
  ctx.save(); ctx.shadowColor = "rgba(0,0,0,.18)"; ctx.shadowBlur = 10;
  ctx.fillStyle = "#fff"; ctx.fillRect(sx(0), sy(Hp), Wp * view.k, Hp * view.k); ctx.restore();

  const [c0, c1] = [T([part.region[0], part.region[1]]), T([part.region[2], part.region[3]])];
  rect([Math.min(c0[0], c1[0]), Math.min(c0[1], c1[1]), Math.max(c0[0], c1[0]), Math.max(c0[1], c1[1])], rgba(r, .08), null, null);
  rect(win.raw, null, "#c0392b", [7, 4]);
  rect(win.safe, null, "rgba(192,57,43,.6)", null);
  drawSeams(T);
  if (S.drawing.show_travel) {
    ctx.save(); ctx.setLineDash([4, 4]); ctx.lineWidth = 0.8; ctx.strokeStyle = "rgba(226,120,30,.7)"; ctx.beginPath();
    for (const [a, b, k] of P.travel) if (k === part.index) { const p = T(a), q = T(b); ctx.moveTo(sx(p[0]), sy(p[1])); ctx.lineTo(sx(q[0]), sy(q[1])); }
    ctx.stroke(); ctx.restore();
  }
  drawStrokes(T, part.index);

  ctx.save(); ctx.strokeStyle = "#333"; ctx.lineWidth = 5;
  ctx.beginPath(); ctx.moveTo(sx(0) - 4, sy(Math.min(60, Hp))); ctx.lineTo(sx(0) - 4, sy(0) + 4); ctx.lineTo(sx(Math.min(60, Wp)), sy(0) + 4); ctx.stroke();
  ctx.fillStyle = "#2f5fb3"; ctx.beginPath(); ctx.arc(sx(0), sy(0), 5, 0, Math.PI * 2); ctx.fill();
  ctx.font = "bold 13px system-ui"; ctx.fillStyle = "#555";
  for (const [name, c] of Object.entries(P.passes.corners)) {
    const [x, y] = toPass(c, r, W, H);
    ctx.fillText(name, sx(x) + (x > 0 ? 4 : -14), sy(y) + (y > 0 ? -4 : 16));
  }
  ctx.fillStyle = rgba(r, 1); ctx.font = "bold 13px system-ui";
  ctx.fillText(`Проход ${part.index}: так лист лежит на столе — поворот ${r}°, в упоре угол ${part.corner} (${part.corner_name}), ноль в нём`, 10, 20);
  ctx.restore();
}
const sx = (x) => view.ox + x * view.k;
const sy = (y) => view.oy - y * view.k;
function poly(p) {
  ctx.moveTo(sx(p[0][0]), sy(p[0][1]));
  if (p.length === 1) ctx.lineTo(sx(p[0][0]) + 0.01, sy(p[0][1]));
  for (let i = 1; i < p.length; i++) ctx.lineTo(sx(p[i][0]), sy(p[i][1]));
}

function hatch(x0, y0, x1, y1, color) {

  if (x1 <= x0 || y1 <= y0) return;
  ctx.save();
  ctx.beginPath(); ctx.rect(sx(x0), sy(y1), (x1 - x0) * view.k, (y1 - y0) * view.k); ctx.clip();
  ctx.fillStyle = "rgba(192,57,43,.06)"; ctx.fill();
  ctx.strokeStyle = color; ctx.lineWidth = 1; ctx.beginPath();
  const step = 8;
  const X0 = sx(x0), Y0 = sy(y1), w = (x1 - x0) * view.k, h = (y1 - y0) * view.k;
  for (let t = -h; t < w; t += step) { ctx.moveTo(X0 + t, Y0 + h); ctx.lineTo(X0 + t + h, Y0); }
  ctx.stroke(); ctx.restore();
}

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
  const [W, H] = sheetWH();
  const part = tablePart();
  if (part) { drawTable(part); return; }

  ctx.save();
  ctx.shadowColor = "rgba(0,0,0,.18)"; ctx.shadowBlur = 12; ctx.shadowOffsetY = 2;
  ctx.fillStyle = "#fff"; ctx.fillRect(sx(0), sy(H), W * view.k, H * view.k);
  ctx.restore();

  const ps = P?.passes;
  if (ps && $("showReach").checked) {
    const bad = P.reach.measured ? "rgba(192,57,43,.35)" : "rgba(217,140,31,.35)";
    for (const [x0, y0, x1, y1] of ps.map_uncovered) hatch(x0, y0, x1, y1, bad);
    ps.map.forEach((r, i) => {
      const q = ps.rects[r]; if (!q) return;
      const [x0, y0, x1, y1] = q;
      ctx.save();
      ctx.fillStyle = rgba(r, .07); ctx.fillRect(sx(x0), sy(y1), (x1 - x0) * view.k, (y1 - y0) * view.k);
      ctx.setLineDash([7, 4]); ctx.lineWidth = 1.3; ctx.strokeStyle = rgba(r, .85);
      ctx.strokeRect(sx(x0) + i, sy(y1) + i, (x1 - x0) * view.k - 2 * i, (y1 - y0) * view.k - 2 * i);

      const [cx, cy] = ps.corners[ps.corner_of[r]];
      ctx.fillStyle = rgba(r, 1); ctx.font = "bold 12px system-ui";
      const tx = sx(Math.min(Math.max(cx, x0), x1)) + (cx > (x0 + x1) / 2 ? -118 : 6);
      const ty = sy(Math.min(Math.max(cy, y0), y1)) + (cy > (y0 + y1) / 2 ? 34 : -24);
      ctx.fillText(`проход ${i + 1}: ${r}° (угол ${ps.corner_of[r]})`, tx, ty);
      ctx.restore();
    });
    if (!P.reach.measured) {
      ctx.save(); ctx.fillStyle = "#d98c1f"; ctx.font = "12px system-ui";
      ctx.fillText("ход не измерен: окно = лист", sx(0) + 6, sy(H) + 30); ctx.restore();
    }
  }

  if (ps) {
    ctx.save(); ctx.font = "bold 13px system-ui"; ctx.fillStyle = "#555";
    for (const [name, [cx, cy]] of Object.entries(ps.corners)) {
      ctx.fillText(name, sx(cx) + (cx > 0 ? 4 : -14), sy(cy) + (cy > 0 ? -4 : 14));
    }
    const rot = ps.rotation;
    if (rot != null) {
      const [cx, cy] = ps.corners[ps.corner_of[rot]];
      const dx = cx > 0 ? -1 : 1, dy = cy > 0 ? -1 : 1, L = 14;
      ctx.fillStyle = rgba(rot, .9);
      ctx.beginPath(); ctx.moveTo(sx(cx), sy(cy)); ctx.lineTo(sx(cx) + dx * L, sy(cy)); ctx.lineTo(sx(cx), sy(cy) - dy * L); ctx.closePath(); ctx.fill();
    }
    ctx.restore();
  }

  if (P?.scale?.area) {
    const [x0, y0, x1, y1] = P.scale.area;
    ctx.save(); ctx.setLineDash([3, 4]); ctx.strokeStyle = "rgba(47,95,179,.35)"; ctx.lineWidth = 1;
    ctx.strokeRect(sx(x0), sy(y1), (x1 - x0) * view.k, (y1 - y0) * view.k);
    ctx.restore();
  }

  const zeros = P?.parts?.length ? P.parts.map((q) => [q.rotation, q.index]) : [[P?.passes?.rotation ?? 0, 0]];
  for (const [zr, idx] of zeros) {
    const [zx, zy] = P?.passes ? P.passes.corners[P.passes.corner_of[zr]] : [0, 0];
    ctx.save(); ctx.fillStyle = idx ? rgba(zr, 1) : "#2f5fb3";
    ctx.beginPath(); ctx.arc(sx(zx), sy(zy), 5, 0, Math.PI * 2); ctx.fill();
    ctx.font = "12px system-ui";
    ctx.fillText(zeros.length > 1 ? `ноль ${idx}` : "ноль", sx(zx) + (zx > 0 ? -44 : 7), sy(zy) + (zy > 0 ? 18 : -6));
    ctx.restore();
  }
  if (!P) return;

  if (S.drawing.show_travel && P.travel.length) {
    ctx.save(); ctx.setLineDash([4, 4]); ctx.lineWidth = 0.8; ctx.strokeStyle = "rgba(226,120,30,.7)";
    ctx.beginPath();
    for (const [a, b] of P.travel) { ctx.moveTo(sx(a[0]), sy(a[1])); ctx.lineTo(sx(b[0]), sy(b[1])); }
    ctx.stroke(); ctx.restore();
  }

  drawSeams((p) => p);

  drawStrokes((p) => p, 0);

  if (P.unreachable.length) {
    ctx.save(); ctx.lineCap = "round"; ctx.strokeStyle = "#e0241b"; ctx.lineWidth = Math.max(2.5, 0.8 * view.k);
    ctx.beginPath(); for (const p of P.unreachable) poly(p); ctx.stroke(); ctx.restore();
  }

  ctx.save(); ctx.font = "11px system-ui";
  for (const t of P.texts) {
    const x = sx(t.x), y = sy(t.y);
    ctx.fillStyle = t.kind === "image" ? "rgba(120,60,180,.8)" : "rgba(200,110,0,.9)";
    ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(x + 6, y - 10); ctx.lineTo(x - 6, y - 10); ctx.closePath(); ctx.fill();
    ctx.fillText((t.kind === "image" ? "🖼 " : "T ") + (t.text.length > 24 ? t.text.slice(0, 23) + "…" : t.text), x + 8, y - 4);
  }
  ctx.restore();
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
window.addEventListener("mouseup", () => { drag = null; cv.classList.remove("drag"); });
window.addEventListener("mousemove", (e) => {
  const r = cv.getBoundingClientRect();
  if (drag) { view.ox = drag.ox + e.clientX - drag.x; view.oy = drag.oy + e.clientY - drag.y; view.auto = false; draw(); }
  if (e.target === cv) {
    const x = (e.clientX - r.left - view.ox) / view.k, y = (view.oy - (e.clientY - r.top)) / view.k;
    $("cursorPos").textContent = `X ${x.toFixed(1)}  Y ${y.toFixed(1)} мм`;
  }
});
$("btnFit").onclick = () => { view.auto = true; fit(); draw(); };
$("showReach").addEventListener("change", draw);
$("showPasses").addEventListener("change", draw);
new ResizeObserver(() => { if (view.auto && S) fit(); draw(); }).observe($("canvasWrap"));

window.addEventListener("focus", async () => {
  if (!S || savePending) return;
  try {
    const fresh = await api("GET", "/api/settings");
    if (JSON.stringify(fresh.printer) !== JSON.stringify(S.printer)) {
      S.printer = fresh.printer; bindFields(); refreshPreview();
    }
  } catch {  }
});

function readings() {
  const r = {};
  document.querySelectorAll("[data-reading]").forEach((el) => {
    if (el.value !== "" && !isNaN(Number(el.value))) r[el.dataset.reading] = Number(el.value);
  });
  return r;
}
function wizardWindow(r) {

  if (["px", "py", "mx", "my"].some((k) => r[k] == null)) return null;
  const x0 = r.x0 ?? 0, y0 = r.y0 ?? 0;
  return { x_min: +(r.mx - x0).toFixed(2), x_max: +(r.px - x0).toFixed(2),
           y_min: +(r.my - y0).toFixed(2), y_max: +(r.py - y0).toFixed(2) };
}
function wizardProblems(w) {
  const e = [];
  if (!w) return ["Введи все четыре показания у упоров"];
  if (!(w.x_min <= 0 && 0 <= w.x_max)) e.push("Ноль (угол листа) вне окна по X: проверь показания угла и упоров по ±X");
  if (!(w.y_min <= 0 && 0 <= w.y_max)) e.push("Ноль (угол листа) вне окна по Y: проверь показания угла и упоров по ±Y");
  const m = S.printer.safety_margin;
  if (w.x_max - w.x_min <= 2 * m || w.y_max - w.y_min <= 2 * m) e.push("Окно меньше двух запасов по одной из осей");
  return e;
}
function wizardUpdate() {
  const r = readings();
  const w = wizardWindow(r);
  const probs = wizardProblems(w);
  const m = S.printer.safety_margin;
  $("wzResult").innerHTML = w
    ? `Окно относительно нуля: X ${w.x_min}…${w.x_max}, Y ${w.y_min}…${w.y_max} мм; ` +
      `с запасом ${m} мм: X ${(w.x_min + m).toFixed(1)}…${(w.x_max - m).toFixed(1)}, Y ${(w.y_min + m).toFixed(1)}…${(w.y_max - m).toFixed(1)}.`
    : "";
  $("wzMsg").innerHTML = probs.map((p) => `<div class="msg err">⛔ ${esc(p)}</div>`).join("");
  $("wzSave").disabled = probs.length > 0;
  wizardSvg(w || S.printer.travel);
}
function wizardSvg(w) {

  const svg = $("wzSvg"), tb = S.printer.table, m = S.printer.safety_margin;
  const sheet = P?.sheet ? [P.sheet.width, P.sheet.height] : [210, 297];
  const win = w || { x_min: 0, x_max: sheet[0], y_min: 0, y_max: sheet[1] };
  const tX = tb.table_x ?? win.x_max, tY = tb.table_y ?? win.y_max;
  const xs = [win.x_min, 0, win.x_max, tX, sheet[0]], ys = [win.y_min, 0, win.y_max, tY, sheet[1]];
  const X0 = Math.min(...xs) - 15, X1 = Math.max(...xs) + 25, Y0 = Math.min(...ys) - 15, Y1 = Math.max(...ys) + 25;
  const k = Math.min(340 / (X1 - X0), 280 / (Y1 - Y0));
  const px = (x) => 10 + (x - X0) * k, py = (y) => 290 - (y - Y0) * k;
  const rect = (x0, y0, x1, y1, st) => `<rect x="${px(x0)}" y="${py(y1)}" width="${(x1 - x0) * k}" height="${(y1 - y0) * k}" ${st}/>`;
  const txt = (x, y, t, st = "") => `<text x="${x}" y="${y}" font-size="11" ${st}>${esc(t)}</text>`;
  let s = `<defs><pattern id="hb" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="6" stroke="#999" stroke-width="2"/></pattern></defs>`;
  s += rect(0, 0, tX, tY, `fill="#eef0f3" stroke="#9aa3ad" ${tb.table_x == null ? 'stroke-dasharray="4 3"' : ""}`);
  s += txt(px(tX) - 4, py(tY) + 13, tb.table_x == null ? "стол (= окно)" : "стол", 'text-anchor="end" fill="#6d757d"');
  s += rect(0, 0, sheet[0], sheet[1], `fill="rgba(47,95,179,.06)" stroke="#2f5fb3" stroke-width="1"`);
  s += txt(px(sheet[0]) - 4, py(0) - 5, `лист ${sheet[0]}×${sheet[1]}, поворот 0°`, 'text-anchor="end" fill="#2f5fb3"');
  s += rect(win.x_min, win.y_min, win.x_max, win.y_max, `fill="none" stroke="#c0392b" stroke-dasharray="6 4" stroke-width="1.3"`);
  s += rect(win.x_min + m, win.y_min + m, win.x_max - m, win.y_max - m, `fill="none" stroke="#c0392b" stroke-width=".8"`);
  s += txt(px(win.x_min) + 3, py(win.y_max) + 13, "окно достижимости", 'fill="#c0392b"');
  for (const [x, y] of [[win.x_min + m, win.y_min + m], [win.x_max - m, win.y_min + m], [win.x_max - m, win.y_max - m], [win.x_min + m, win.y_max - m]])
    s += `<circle cx="${px(x)}" cy="${py(y)}" r="3" fill="#c0392b"/>`;

  s += `<path d="M${px(0) - 5} ${py(60)} V${py(0) + 5} H${px(60)}" fill="none" stroke="#333" stroke-width="5"/>`;
  s += txt(px(0) + 6, py(0) + 16, "упоры", 'fill="#333"');
  s += `<circle cx="${px(0)}" cy="${py(0)}" r="4" fill="#2f5fb3"/>` + txt(px(0) + 6, py(0) - 6, "0,0", 'fill="#2f5fb3"');

  const side = (ok, x, y, t) => txt(x, y, (ok ? "→ " : "✕ ") + t, `fill="${ok ? "#27965a" : "#b3261e"}"`);
  s += side(tb.overhang_x, px(tX) + 3, py(tY / 2), "+X");
  s += side(tb.overhang_y, px(tX / 2), py(tY) - 5, "+Y");
  if (!tb.overhang_x) s += `<rect x="${px(tX)}" y="${py(tY)}" width="6" height="${tY * k}" fill="url(#hb)"/>`;
  if (!tb.overhang_y) s += `<rect x="${px(0)}" y="${py(tY) - 6}" width="${tX * k}" height="6" fill="url(#hb)"/>`;
  svg.innerHTML = s;
}
$("btnWizard").onclick = () => {
  const r = S.printer.table.readings || {};
  document.querySelectorAll("[data-reading]").forEach((el) => {
    el.value = r[el.dataset.reading] ?? "";
  });
  if (!Object.keys(r).length && S.printer.travel) {
    const t = S.printer.travel;
    Object.entries({ px: t.x_max, py: t.y_max, mx: t.x_min, my: t.y_min })
      .forEach(([k, v]) => (document.querySelector(`[data-reading="${k}"]`).value = v));
  }
  $("wizard").hidden = false; wizardUpdate();
};
$("wzClose").onclick = () => ($("wizard").hidden = true);
$("wizard").addEventListener("mousedown", (e) => { if (e.target === $("wizard")) $("wizard").hidden = true; });
$("wzSave").onclick = () => {
  const r = readings(), w = wizardWindow(r);
  if (!w || wizardProblems(w).length) return;
  S.printer.table.readings = r;
  S.printer.travel = w; lastTravel = { ...w };
  bindFields(); changed("full");
  $("wzMsg").innerHTML = `<div class="msg warn">Записано. Теперь проверь окно проверочным файлом.</div>`;
};
async function calibFile(url) {
  try {
    const r = await api("POST", url, S);
    await download(r.filename, r.gcode);
    const info = await api("POST", "/api/calibration/info", S);
    $("wzMsg").innerHTML = info.warnings.map((m) => `<div class="msg warn">⚠ ${esc(m)}</div>`).join("");
  } catch (e) {
    $("wzMsg").innerHTML = (e.data?.errors || [e.message]).map((m) => `<div class="msg err">⛔ ${esc(m)}</div>`).join("");
  }
}
$("wzCheck").onclick = async () => {

  const w = wizardWindow(readings());
  if (w && !wizardProblems(w).length && JSON.stringify(w) !== JSON.stringify(S.printer.travel)) $("wzSave").onclick();
  await flushSave();
  calibFile("/api/calibration/check_gcode");
};
$("wzZero").onclick = (e) => { e.preventDefault(); calibFile("/api/calibration/zero_gcode"); };

(async function init() {
  S = await api("GET", "/api/settings");
  await loadFiles();
  bindFields();
  fit();
  await refreshPreview();
})();
