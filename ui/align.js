"use strict";

const AL = {
  open: false,
  step: 0,
  maxStep: 0,
  info: null,
  read: {},
  result: null,
  printed: new Set(),
};

const AL_STEPS = [
  { id: "intro", dot: "Зачем" },
  { id: "print", dot: "Печать теста" },
  { id: "howto", dot: "Как читать метку" },
  { id: "read", dot: "Замеры" },
  { id: "result", dot: "Поправка" },
];

function alOffsetsSet() {
  const o = state.settings?.drawing.split.offsets || {};
  return Object.values(o).some(([x, y]) => Math.abs(x) > 1e-9 || Math.abs(y) > 1e-9);
}

async function openAlign(open) {
  AL.open = open;
  if (open) {
    closePopovers();
    AL.step = 0;
    AL.maxStep = 0;
    AL.result = null;
    AL.printed = new Set();
    AL.info = null;
    reveal($("alignPanel"), true);
    alRender();
    await flushSave();
    try {
      AL.info = await api("POST", "/api/drawing/align", state.settings);
    } catch (e) {
      AL.info = { errors: e.data?.errors || [e.message], marks: [], parts: [] };
    }
    const keep = {};
    for (const m of AL.info.marks || []) keep[m.id] = AL.read[m.id] || { x: 0, y: 0 };
    AL.read = keep;
    alRender();
    return;
  }
  reveal($("alignPanel"), false);
}

function alGo(i) {
  AL.step = Math.max(0, Math.min(AL_STEPS.length - 1, i));
  AL.maxStep = Math.max(AL.maxStep, AL.step);
  alRender();
}

function alSheetSvg(highlight) {
  const I = AL.info;
  if (!I?.sheet?.width) return "";
  const W = I.sheet.width, H = I.sheet.height;
  const VW = 400, VH = 440, pad = 24;
  const k = Math.min((VW - 2 * pad) / W, (VH - 2 * pad - 30) / H);
  const ox = (VW - W * k) / 2, oy = (VH + H * k) / 2 + 10;
  const px = (x) => +(ox + x * k).toFixed(1), py = (y) => +(oy - y * k).toFixed(1);
  const colors = ["#c98a3c", "#3c7fc9", "#3ca06a", "#b5487a", "#7a5cc9", "#c9b13c"];
  let s = `<svg viewBox="0 0 ${VW} ${VH}" xmlns="http://www.w3.org/2000/svg">`;
  s += `<text x="${VW / 2}" y="24" text-anchor="middle" style="fill:var(--muted);font-size:12px">лист как для прохода 1, вид сверху</text>`;
  s += `<rect x="${px(0)}" y="${py(H)}" width="${(W * k).toFixed(1)}" height="${(H * k).toFixed(1)}" style="fill:var(--paper);stroke:var(--line)" filter="url(#alShadow)"/>`;
  s += `<defs><filter id="alShadow" x="-10%" y="-10%" width="120%" height="120%"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-opacity=".18"/></filter></defs>`;
  for (const p of I.parts || []) {
    const [x0, y0, x1, y1] = p.region;
    const c = colors[(p.index - 1) % colors.length];
    s += `<rect x="${px(x0)}" y="${py(y1)}" width="${((x1 - x0) * k).toFixed(1)}" height="${((y1 - y0) * k).toFixed(1)}" style="fill:${c};fill-opacity:.13;stroke:${c};stroke-opacity:.5;stroke-dasharray:4 3"/>`;
    s += `<text x="${px((x0 + x1) / 2)}" y="${py((y0 + y1) / 2) + 6}" text-anchor="middle" style="fill:${c};font-size:18px;font-weight:700;opacity:.8">${p.index}</text>`;
  }
  for (const m of I.marks || []) {
    const on = highlight == null || highlight === m.id;
    s += `<g class="${highlight === m.id ? "cal-pulse-g" : ""}" style="opacity:${on ? 1 : 0.35}">
      <circle cx="${px(m.x)}" cy="${py(m.y)}" r="11" style="fill:var(--accent)"/>
      <text x="${px(m.x)}" y="${py(m.y) + 4}" text-anchor="middle" style="fill:var(--accent-ink);font-size:12px;font-weight:700">${m.id}</text></g>`;
  }
  s += `<style>@keyframes alPulse { 50% { transform: scale(1.18); } } .cal-pulse-g { animation: alPulse 1.4s ease-in-out infinite; transform-box: fill-box; transform-origin: center; }</style>`;
  return s + "</svg>";
}

function alMarkSvg(dx, dy, ticks = 4, big = true) {
  const size = 14, h = size / 2, band = h - 2.8;
  const VW = big ? 360 : 200;
  const k = VW / 19;
  const c = VW / 2;
  const px = (x) => +(c + x * k).toFixed(1), py = (y) => +(c - y * k).toFixed(1);
  const tick = (i) => (i === 0 ? 2.8 : Math.abs(i) === ticks ? 2.0 : 1.2);
  const line = (x0, y0, x1, y1, st) => `<line x1="${px(x0)}" y1="${py(y0)}" x2="${px(x1)}" y2="${py(y1)}" style="${st}"/>`;
  const ink = "stroke:var(--text);stroke-width:2.2;stroke-linecap:round";
  const ptr = "stroke:var(--accent);stroke-width:2.6;stroke-linecap:round";
  let s = `<svg viewBox="0 0 ${VW} ${VW}" xmlns="http://www.w3.org/2000/svg">`;
  s += `<rect x="2" y="2" width="${VW - 4}" height="${VW - 4}" rx="12" style="fill:var(--paper);stroke:var(--line)"/>`;
  for (let i = -ticks; i <= ticks; i++) s += line(i, band, i, band + tick(i), ink);
  for (let j = -ticks; j <= ticks; j++) s += line(band, j, band + tick(j), j, ink);
  s += `<circle cx="${px(ticks)}" cy="${py(band + tick(ticks) + 0.9)}" r="${(0.45 * k).toFixed(1)}" style="fill:none;stroke:var(--text);stroke-width:2"/>`;
  s += `<circle cx="${px(band + tick(ticks) + 0.9)}" cy="${py(ticks)}" r="${(0.45 * k).toFixed(1)}" style="fill:none;stroke:var(--text);stroke-width:2"/>`;
  s += line(-h + 0.5, -h + 0.3, -h + 0.5, -h + 1.3, ink);
  s += line(dx, dy - h + 2, dx, dy + h + 0.5, ptr) + line(dx - h + 2, dy, dx + h + 0.5, dy, ptr);
  if (big) {
    s += `<text x="${px(0)}" y="${py(band + 2.8) - 8}" text-anchor="middle" style="fill:var(--muted);font-size:12px">0</text>`;
    s += `<text x="${px(band + 2.8) + 8}" y="${py(0) + 4}" style="fill:var(--muted);font-size:12px">0</text>`;
    s += `<text x="${px(ticks)}" y="${py(band + tick(ticks) + 0.9) - 14}" text-anchor="middle" style="fill:var(--muted);font-size:12px">+</text>`;
    s += `<text x="${px(band + tick(ticks) + 0.9) + 14}" y="${py(ticks) + 4}" style="fill:var(--muted);font-size:12px">+</text>`;
    s += `<text x="${px(-h + 0.5) + 10}" y="${py(-h + 0.6)}" style="fill:var(--muted);font-size:12px">номер метки = число палочек</text>`;
    s += `<text x="${px(dx - h + 2)}" y="${py(dy) - 8}" style="fill:var(--accent);font-size:12px;font-weight:600">крест другого прохода</text>`;
  }
  return s + "</svg>";
}

function alStepIntro() {
  const I = AL.info;
  const out = [h("h3", { text: "Совмещение проходов" }),
    h("p", { text: "Если на стыке линии разных проходов не сходятся, виноваты не руки: реальный лист на 1–2 мм отличается от номинала, а ноль карандаша стоит чуть мимо угла. При повороте листа эти ошибки удваиваются." }),
    h("p", { text: "Сделаем тест: на чистом листе каждый проход нарисует метки на стыках, ты посчитаешь деления, и программа сдвинет проходы так, чтобы они сошлись." })];
  if (!I) out.push(h("p", { class: "muted", text: "Считаю метки…" }));
  else if (I.errors?.length) out.push(h("div", { class: "cal-box err" }, I.errors.map((e) => h("p", { text: e }))));
  else out.push(h("div", { class: "cal-box" }, h("p", { text: `Проходов: ${I.parts.length}, меток для замера: ${I.marks.length}. Метки — кружки с номерами на схеме.` })));
  if (alOffsetsSet()) {
    const o = state.settings.drawing.split.offsets;
    out.push(h("div", { class: "cal-box ok" },
      h("p", { text: "Поправка уже задана: " + Object.entries(o).map(([r, [x, y]]) => `поворот ${r}°: ${n2(x)}, ${n2(y)} мм`).join("; ") + ". Новый тест уточнит её." }),
      h("div", { class: "cal-row" }, h("button", { class: "btn small", onclick: alReset }, "Сбросить поправку"))));
  }
  return out;
}

function alReset() {
  state.settings.drawing.split.offsets = {};
  saveSettings();
  refreshPreview();
  toast("Поправка совмещения сброшена");
  openAlign(true);
}

function alStepPrint() {
  const I = AL.info;
  const out = [h("h3", { text: "Печать теста" }),
    h("p", { text: "Возьми чистый лист того же формата. Клади его для каждого прохода так же, как для чертежа, и ставь ноль так же. Карандаш нарисует только маленькие метки." })];
  const cable = calConnected();
  const list = h("div", { class: "cal-box" });
  for (const p of I?.parts || []) {
    const row = h("div", { class: "cal-row" },
      h("span", { style: "flex:1" }, h("b", { text: `Проход ${p.index}` }), ` — поворот ${p.rotation}°, в упоры угол ${p.corner} (${p.corner_name})`));
    if (cable) {
      row.append(h("button", { class: "btn small needs-idle" + (AL.printed.has(p.index) ? "" : " primary"), onclick: () => alPrint(p.index) },
        AL.printed.has(p.index) ? "Ещё раз" : "Печать"));
    }
    list.append(row);
  }
  out.push(list);
  if (cable) {
    out.push(h("p", { class: "muted", id: "calProgress" }));
  } else {
    out.push(h("div", { class: "cal-row" }, h("button", { class: "btn primary", onclick: alDownload }, "Скачать файлы теста")),
      h("p", { class: "muted", text: "Принтер по кабелю не подключён — файлы для SD-карты. Запускай их по порядку номеров." }));
  }
  return out;
}

async function alPrint(index) {
  await flushSave();
  try {
    await api("POST", `/api/printer/print?kind=align&part=${index}`, state.settings);
    AL.printed.add(index);
    schedulePoll(100);
    alRender();
  } catch (e) {
    toast(e.data?.errors?.join("; ") || e.message, 8000);
  }
}

async function alDownload() {
  await flushSave();
  busy(true, "готовлю архив…");
  try {
    if (IN_APP) {
      const r = await api("POST", "/api/drawing/zip/save?align=1", state.settings);
      toast(`Сохранено: ${r.path}`, 8000);
      return;
    }
    const r = await fetch("/api/drawing/zip?align=1", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(state.settings),
    });
    if (!r.ok) {
      const d = await r.json().catch(() => ({}));
      throw Object.assign(new Error(r.statusText), { data: d });
    }
    const a = document.createElement("a");
    a.href = URL.createObjectURL(await r.blob());
    a.download = fileNameFrom(r, "align.zip");
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  } catch (e) {
    toast(e.data?.errors?.join("; ") || e.message, 8000);
  } finally {
    busy(false);
  }
}

function alStepHowto() {
  return [
    h("h3", { text: "Как читать метку" }),
    h("p", { text: "В каждой метке две шкалы с делениями через 1 мм — их рисует один проход. Крест рисует соседний проход." }),
    h("ol", {},
      h("li", { text: "Самый длинный штрих шкалы — ноль. Кружок у конца шкалы показывает сторону «плюс»." }),
      h("li", { text: "Посмотри, где линия креста пересекает шкалу, и посчитай деления от нуля: к кружку — плюс, от кружка — минус. Между делениями — половинка, 0.5." }),
      h("li", { text: "Так же для второй шкалы. Номер метки — число коротких палочек в её углу." })),
    h("div", { class: "cal-box" }, h("p", { text: "На примере справа: по верхней шкале крест на +1.5, по правой — на −2." })),
  ];
}

function alStepRead() {
  const I = AL.info;
  const out = [h("h3", { text: "Замеры" }), h("p", { text: "Держи лист как на схеме — как для прохода 1. Для каждой метки введи, на сколько делений крест ушёл от нуля. Совпал с нулём — оставь 0." })];
  for (const m of I?.marks || []) {
    const r = AL.read[m.id] || (AL.read[m.id] = { x: 0, y: 0 });
    const inp = (key, label) => {
      const i = h("input", { type: "number", step: "0.5", value: String(r[key]) });
      i.oninput = () => {
        const v = Number(i.value);
        r[key] = i.value === "" || !Number.isFinite(v) ? 0 : v;
        AL.result = null;
        const art = $("alMini" + m.id);
        if (art) art.innerHTML = alMarkSvg(r.x, r.y, m.ticks, false);
      };
      i.onfocus = () => { $("alArt").innerHTML = alSheetSvg(m.id); };
      return h("label", {}, h("span", { text: label }), i);
    };
    out.push(h("div", { class: "cal-box al-mark" },
      h("div", { class: "al-mini", id: "alMini" + m.id }),
      h("div", { class: "al-fields" },
        h("b", { text: `Метка ${m.id}` }),
        h("span", { class: "muted", text: `палочек: ${m.id} · шкалы — проход ${m.scale_pass}, крест — проход ${m.pointer_pass}` }),
        h("div", { class: "cal-row" }, inp("x", "верхняя шкала"), inp("y", "правая шкала")))));
  }
  setTimeout(() => {
    for (const m of AL.info?.marks || []) {
      const el = $("alMini" + m.id);
      if (el) el.innerHTML = alMarkSvg(AL.read[m.id].x, AL.read[m.id].y, m.ticks, false);
    }
  });
  return out;
}

async function alCompute() {
  const readings = (AL.info?.marks || []).map((m) => ({ mark: m.id, x: AL.read[m.id]?.x || 0, y: AL.read[m.id]?.y || 0 }));
  try {
    AL.result = await api("POST", "/api/drawing/align/apply", { settings: state.settings, readings });
  } catch (e) {
    AL.result = { errors: e.data?.errors || [e.message] };
  }
}

function alStepResult() {
  const R = AL.result;
  const out = [h("h3", { text: "Поправка" })];
  if (!R) return [...out, h("p", { class: "muted", text: "Считаю…" })];
  if (R.errors) return [...out, h("div", { class: "cal-box err" }, R.errors.map((e) => h("p", { text: e })))];
  const moved = R.parts.filter((p) => p.shift > 0.01);
  if (!moved.length) {
    out.push(h("div", { class: "cal-box ok" }, h("p", { text: "Все кресты на нуле — проходы уже совпадают. Ничего менять не нужно." })));
    return out;
  }
  out.push(h("p", { text: "Программа сдвинет проходы относительно прохода 1:" }),
    h("div", { class: "cal-table" }, R.parts.flatMap((p) => [
      h("span", { text: `Проход ${p.index} (поворот ${p.rotation}°)` }),
      h("b", { text: p.shift > 0.01 ? `сдвиг ${n2(p.shift)} мм` : "без изменений" }),
    ])),
    h("p", { class: "muted", text: "Поправка запишется в настройки и попадёт во все файлы этого режима. Чтобы проверить, напечатай тест ещё раз: кресты должны встать на ноль." }));
  return out;
}

function alRender() {
  const id = AL_STEPS[AL.step].id;
  const dots = $("alSteps");
  dots.innerHTML = "";
  AL_STEPS.forEach((s, i) => {
    const li = h("li", { title: s.dot, text: String(i + 1) });
    if (i === AL.step) li.className = "on";
    else if (i <= AL.maxStep) {
      li.className = i < AL.step ? "done" : "reach";
      li.onclick = () => alGo(i);
    }
    dots.appendChild(li);
  });
  const parts = id === "intro" ? alStepIntro() : id === "print" ? alStepPrint() : id === "howto" ? alStepHowto() : id === "read" ? alStepRead() : alStepResult();
  $("alContent").innerHTML = "";
  $("alContent").append(...parts);
  $("alArt").innerHTML = id === "howto" ? alMarkSvg(1.5, -2) : alSheetSvg(null);
  $("alCount").textContent = `Шаг ${AL.step + 1} из ${AL_STEPS.length} · ${AL_STEPS[AL.step].dot}`;
  $("alBack").disabled = AL.step === 0;
  const blocked = !AL.info || AL.info.errors?.length;
  const R = AL.result;
  const nothing = R && !R.errors && !R.parts.some((p) => p.shift > 0.01);
  $("alNext").textContent = id === "result" ? (nothing || R?.errors ? "Закрыть" : "Применить") : id === "read" ? "Рассчитать →" : "Далее →";
  $("alNext").disabled = !!blocked && id !== "result";
  alLive();
}

function alLive() {
  if (!AL.open) return;
  const busy = !calIdle();
  for (const b of document.querySelectorAll("#alContent .needs-idle")) b.disabled = busy;
  const prog = document.querySelector("#alContent #calProgress");
  const job = PR.st?.job;
  if (prog) prog.textContent = job ? `${job.label}: ${job.percent}%` : "";
}

$("alClose").onclick = () => openAlign(false);
$("alignPanel").addEventListener("mousedown", (e) => { if (e.target === $("alignPanel")) openAlign(false); });
$("alBack").onclick = () => alGo(AL.step - 1);
$("alNext").onclick = async () => {
  const id = AL_STEPS[AL.step].id;
  if (id === "read") {
    alGo(AL.step + 1);
    await alCompute();
    alRender();
    return;
  }
  if (id === "result") {
    const R = AL.result;
    if (R && !R.errors && R.parts.some((p) => p.shift > 0.01)) {
      state.settings.drawing.split.offsets = R.offsets;
      AL.read = {};
      saveSettings();
      await flushSave();
      refreshPreview();
      toast("Поправка совмещения записана");
    }
    openAlign(false);
    return;
  }
  alGo(AL.step + 1);
};
addEventListener("themechange", () => AL.open && alRender());
if (new URLSearchParams(location.search).get("open") === "align") setTimeout(() => openAlign(true), 800);
