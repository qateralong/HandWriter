"use strict";

const CAL = {
  open: false,
  step: 0,
  maxStep: 0,
  mode: localGet("hw-cal-mode") || "",
  r: {},
  zeroed: false,
  stepMm: 1,
  ports: [],
  info: null,
  checkStarted: false,
};

const CAL_DIRS = {
  px: { axis: "X", sign: 1, name: "+X", where: "вдоль листа от угла по +X", jog: "X+" },
  py: { axis: "Y", sign: 1, name: "+Y", where: "вдоль листа от угла по +Y", jog: "Y+" },
  mx: { axis: "X", sign: -1, name: "−X", where: "за угол листа по −X, над упором", jog: "X-" },
  my: { axis: "Y", sign: -1, name: "−Y", where: "за угол листа по −Y, над упором", jog: "Y-" },
};

const CAL_STEPS = [
  { id: "mode", dot: "Способ управления" },
  { id: "prep", dot: "Подготовка" },
  { id: "zero", dot: "Ноль в углу листа" },
  { id: "px", dot: "Край по +X" },
  { id: "py", dot: "Край по +Y" },
  { id: "mx", dot: "Край по −X" },
  { id: "my", dot: "Край по −Y" },
  { id: "result", dot: "Итог" },
  { id: "check", dot: "Проверка" },
];

const CAL_SHEET = [210, 297];

function h(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e[k] = v;
    else if (k === "text") e.textContent = v;
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? "" : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
  return e;
}

const calNum = (v) => (Math.round(v * 10) / 10).toFixed(1);
const calCable = () => CAL.mode === "cable";
const calConnected = () => !!PR.st?.connected && PR.st.status !== "halted" && PR.st.status !== "connecting";
const calIdle = () => PR.st?.status === "idle";

function calWindow(r) {
  if (["px", "py", "mx", "my"].some((k) => typeof r[k] !== "number")) return null;
  const x0 = r.x0 ?? 0, y0 = r.y0 ?? 0;
  const round = (v) => Math.round(v * 100) / 100;
  return { x_min: round(r.mx - x0), x_max: round(r.px - x0), y_min: round(r.my - y0), y_max: round(r.py - y0) };
}

function calProblems(w) {
  if (!w) return ["Не хватает замеров: пройди все четыре края"];
  const e = [];
  if (!(w.x_min <= 0 && 0 <= w.x_max)) e.push("Угол листа (ноль) вне окна по X: проверь замеры по +X и −X — у −X число должно быть отрицательным или нулём");
  if (!(w.y_min <= 0 && 0 <= w.y_max)) e.push("Угол листа (ноль) вне окна по Y: проверь замеры по +Y и −Y — у −Y число должно быть отрицательным или нулём");
  const m = state.settings.printer.safety_margin;
  if (w.x_max - w.x_min <= 2 * m || w.y_max - w.y_min <= 2 * m) e.push(`Окно меньше двух запасов (${2 * m} мм) по одной из осей`);
  return e;
}

function openCalib(open) {
  CAL.open = open;
  if (open) {
    const S = state.settings;
    CAL.r = { ...(S.printer.table.readings || {}) };
    if (!Object.keys(CAL.r).length && S.printer.travel) {
      const t = S.printer.travel;
      CAL.r = { x0: 0, y0: 0, px: t.x_max, py: t.y_max, mx: t.x_min, my: t.y_min };
    }
    CAL.zeroed = false;
    CAL.checkStarted = false;
    CAL.info = null;
    CAL.step = 0;
    CAL.maxStep = calWindow(CAL.r) ? CAL_STEPS.length - 2 : 0;
    closePopovers();
    if (!PR.ports.length) loadPorts().then(() => CAL.open && calRender());
    calRender();
    schedulePoll(0);
  }
  reveal($("calibPanel"), open);
}

function calGo(i) {
  CAL.step = Math.max(0, Math.min(CAL_STEPS.length - 1, i));
  CAL.maxStep = Math.max(CAL.maxStep, CAL.step);
  calRender();
}

function calCanNext() {
  const id = CAL_STEPS[CAL.step].id;
  if (id === "mode") return CAL.mode === "sd" || (CAL.mode === "cable" && calConnected());
  if (id === "zero") return !calCable() || CAL.zeroed || typeof CAL.r.x0 === "number";
  if (CAL_DIRS[id]) return typeof CAL.r[id] === "number";
  if (id === "result") return calProblems(calWindow(CAL.r)).length === 0;
  return true;
}

function calSend(lines) {
  return prCall("/api/printer/command", { cmd: lines.join("\n") });
}

function calJog(axis) {
  const P = state.settings.printer;
  const a = axis[0];
  const down = axis[1] === "-";
  const step = a === "Z" ? Math.min(CAL.stepMm, down ? 1 : 10) : CAL.stepMm;
  const feed = a === "Z" ? P.feed_z : P.feed_travel;
  calSend(["G91", `G0 ${a}${n2(down ? -step : step)} F${Math.round(feed)}`, "G90"]);
}

function calPenUp() {
  const P = state.settings.printer;
  calSend(["G90", `G0 Z${n2(P.pen_up_z)} F${Math.round(P.feed_z)}`]);
}

function calPad(hint) {
  const btn = (jogKey, label, title) =>
    h("button", { class: "icon" + (jogKey === hint ? " hint" : ""), title, "data-caljog": jogKey, onclick: () => calJog(jogKey) }, label);
  const steps = h("div", { class: "seg" },
    [0.1, 1, 10, 50].map((v) =>
      h("button", { class: CAL.stepMm === v ? "on" : "", onclick: () => { CAL.stepMm = v; calRender(); } }, String(v))));
  return h("div", { class: "cal-pad" },
    h("div", { class: "pad" },
      h("span"), btn("Y+", "▲", "Y+"), h("span"),
      btn("X-", "◀", "X−"), h("button", { class: "icon", title: "Поднять карандаш", onclick: calPenUp }, "⤒"), btn("X+", "▶", "X+"),
      h("span"), btn("Y-", "▼", "Y−"), h("span")),
    h("div", { class: "zcol" }, btn("Z+", "▲", "Z+ (карандаш выше)"), h("span", { text: "Z" }), btn("Z-", "▼", "Z− (карандаш ниже, не больше 1 мм за шаг)")),
    h("div", { class: "steps" }, h("span", { class: "muted", text: "Шаг, мм" }), steps));
}

function calLiveBox(axis) {
  return h("div", { class: "cal-live", id: "calLive", "data-axis": axis || "" });
}

function calLive() {
  if (!CAL.open) return;
  const box = $("calLive");
  if (box) {
    const p = PR.st?.position;
    box.innerHTML = "";
    if (!p || !calConnected()) {
      box.append(h("span", { text: "Принтер не подключён" }));
    } else {
      for (const k of ["X", "Y", "Z"]) {
        box.append(h("b", { class: box.dataset.axis === k ? "hot" : "" }, h("span", { text: k + " " }), n2(p[k.toLowerCase()])));
      }
    }
  }
  const rec = $("calRecord");
  if (rec && PR.st?.position) {
    const d = CAL_DIRS[CAL_STEPS[CAL.step].id];
    rec.textContent = `Записать ${d.axis} = ${calNum(PR.st.position[d.axis.toLowerCase()])} мм`;
  }
  const busy = !calIdle();
  for (const b of document.querySelectorAll("#calContent [data-caljog], #calContent .needs-idle")) b.disabled = busy;
  const job = PR.st?.job;
  const prog = $("calProgress");
  if (prog) prog.textContent = job ? `${job.label}: ${job.percent}%` : PR.st?.finished && CAL.checkStarted ? "Проверка закончена." : "";
  $("calNext").disabled = !calCanNext();
}

function calNotConnected() {
  return h("div", { class: "cal-box warn" },
    h("p", { text: "Принтер не подключён. Подключи его на первом шаге или переключись на SD-карту." }),
    h("div", { class: "cal-row" }, h("button", { class: "btn small", onclick: () => calGo(0) }, "К подключению")));
}

function calStepMode() {
  const card = (mode, icon, title, text) =>
    h("button", { class: "cal-card" + (CAL.mode === mode ? " on" : ""), onclick: () => { CAL.mode = mode; localPut("hw-cal-mode", mode); calRender(); } },
      h("span", { class: "big", text: icon }), h("b", { text: title }), h("span", { text }));
  const out = [
    h("h3", { text: "Как управляешь принтером?" }),
    h("p", { text: "Мастер найдёт, куда карандаш может дотянуться от угла листа. Это нужно, чтобы чертёж и текст не упирались в раму." }),
    h("div", { class: "cal-cards" },
      card("cable", "🔌", "По кабелю USB", "Двигаешь карандаш прямо отсюда, координаты записываются кнопкой."),
      card("sd", "💾", "С SD-карты", "Двигаешь карандаш с экрана принтера, числа с экрана вводишь сюда.")),
  ];
  if (CAL.mode === "cable") {
    if (calConnected()) {
      out.push(h("div", { class: "cal-box ok" }, h("p", { text: `Принтер подключён (${PR.st.port}).` })));
    } else {
      const sel = h("select", { id: "calPort" });
      for (const p of PR.ports) sel.add(new Option(p.label, p.path));
      if (!PR.ports.length) sel.add(new Option("Портов не найдено — подключи кабель", ""));
      const last = localGet("hw-printer-port");
      if (last && PR.ports.some((p) => p.path === last)) sel.value = last;
      out.push(h("div", { class: "cal-box" },
        h("p", { text: PR.connErr || (PR.st?.status === "connecting" ? "Подключаюсь…" : "Выбери порт принтера и подключись.") }),
        h("div", { class: "cal-row" }, sel,
          h("button", { class: "icon", title: "Обновить список портов", onclick: () => loadPorts().then(calRender) }, "↻"),
          h("button", { class: "btn primary", onclick: calConnect, disabled: !PR.ports.length }, "Подключить"))));
    }
  }
  return out;
}

async function calConnect() {
  const port = $("calPort")?.value;
  if (!port) return;
  localPut("hw-printer-port", port);
  PR.connErr = "";
  try {
    await api("POST", "/api/printer/connect", { port, baud: Number(localGet("hw-printer-baud") || 115200) });
  } catch (e) {
    PR.connErr = e.message;
  }
  schedulePoll(100);
  calRender();
  setTimeout(() => CAL.open && calRender(), 3000);
}

function calStepPrep() {
  return [
    h("h3", { text: "Подготовка" }),
    h("ol", {},
      h("li", { text: "Закрепи карандаш в держателе и подними его над столом." }),
      h("li", { text: "Положи лист (лучше A4) в угол упоров: угол листа — точно в угол, края вдоль упоров." }),
      h("li", { text: "Дальше карандаш дойдёт до края хода в каждую из четырёх сторон. Двигай его поднятым, чтобы не чертить по листу." })),
    h("p", { class: "muted", text: "На схеме: упоры — тёмный уголок, ноль — угол листа у упоров. Оси подписаны так, как их показывает экран принтера." }),
  ];
}

function calStepZero() {
  const out = [h("h3", { text: "Ноль в углу листа" })];
  if (calCable()) {
    if (!calConnected()) return [...out, calNotConnected()];
    out.push(
      h("p", { text: "Подведи карандаш к углу листа у упоров и опусти до лёгкого касания бумаги: у самой бумаги — шагом 0.1 мм." }),
      calLiveBox(""),
      calPad(""),
      h("div", { class: "cal-row" },
        h("button", { class: "btn primary cal-big needs-idle", onclick: calZeroHere }, "Карандаш в углу — поставить ноль")),
      h("p", { class: "muted", text: "Ноль ставится командой G92 X0 Y0 Z0, затем карандаш поднимается. Высота касания станет Z0 — от неё считаются подъём и нажим карандаша." }));
    if (CAL.zeroed) out.push(h("div", { class: "cal-box ok" }, h("p", { text: "Ноль поставлен, карандаш поднят. Жми «Далее»." })));
  } else {
    out.push(
      h("ol", {},
        h("li", { text: "С экрана принтера (меню «Перемещение») подведи карандаш к углу листа у упоров и опусти до касания." }),
        h("li", {}, "Скачай ", h("b", { text: "файл нуля" }), ", запиши на карту и запусти: на экране станет X0 Y0, карандаш поднимется.")),
      h("div", { class: "cal-row" }, h("button", { class: "btn primary", onclick: () => calDownload("/api/calibration/zero_gcode") }, "Скачать файл нуля")),
      h("details", { class: "cal-more" }, h("summary", { text: "Без файла нуля" }),
        h("p", { class: "muted", text: "Перепиши координаты угла с экрана — мастер отсчитает от них." }),
        h("div", { class: "cal-row" }, calInput("x0", "Угол: X"), calInput("y0", "Угол: Y"))));
  }
  return out;
}

async function calZeroHere() {
  try {
    const r = await api("POST", "/api/calibration/zero_gcode", state.settings);
    const lines = r.gcode.split("\n").map((l) => l.split(";")[0].trim()).filter(Boolean);
    if (await calSend(lines)) {
      CAL.r.x0 = 0;
      CAL.r.y0 = 0;
      CAL.zeroed = true;
      calRender();
    }
  } catch (e) {
    toast(e.message, 7000);
  }
}

function calInput(key, label) {
  const inp = h("input", { type: "number", step: "0.1", value: typeof CAL.r[key] === "number" ? String(CAL.r[key]) : "" });
  if (key === "x0" || key === "y0") inp.placeholder = "0";
  inp.oninput = () => {
    const v = inp.value === "" ? null : Number(inp.value);
    if (v == null || !Number.isFinite(v)) delete CAL.r[key];
    else CAL.r[key] = v;
    $("calNext").disabled = !calCanNext();
    calArt();
  };
  return h("label", {}, h("span", { text: label }), inp, h("i", { class: "muted", text: "мм" }));
}

function calStepDir(key) {
  const d = CAL_DIRS[key];
  const out = [
    h("h3", { text: `Край по ${d.name}` }),
    h("p", {}, "Карандаш поднят. Веди его ", h("b", { text: d.where }), ", пока он не упрётся в раму или край хода. У края — шагом 0.1 мм, чтобы не стучать."),
  ];
  const have = typeof CAL.r[key] === "number";
  if (calCable()) {
    if (!calConnected()) return [...out, calNotConnected()];
    out.push(calLiveBox(d.axis), calPad(d.jog),
      h("div", { class: "cal-row" },
        h("button", { class: "btn primary cal-big needs-idle", id: "calRecord", onclick: () => calRecord(key) }, `Записать ${d.axis}`)));
    if (have) {
      out.push(h("div", { class: "cal-box ok" },
        h("p", { text: `Записано: ${d.axis} = ${calNum(CAL.r[key])} мм.` }),
        h("div", { class: "cal-row" },
          h("button", { class: "btn small needs-idle", onclick: calReturn }, "Вернуть карандаш в угол"))));
    }
  } else {
    out.push(h("p", { text: `Когда карандаш упрётся, перепиши с экрана координату ${d.axis}:` }),
      h("div", { class: "cal-row" }, calInput(key, `${d.axis} на экране`)));
    if (d.sign < 0) out.push(h("p", { class: "muted", text: "За углом листа число обычно отрицательное — это нормально." }));
  }
  return out;
}

function calRecord(key) {
  const d = CAL_DIRS[key];
  const p = PR.st?.position;
  if (!p) return;
  CAL.r[key] = Math.round(p[d.axis.toLowerCase()] * 100) / 100;
  if (typeof CAL.r.x0 !== "number") CAL.r.x0 = 0;
  if (typeof CAL.r.y0 !== "number") CAL.r.y0 = 0;
  calRender();
}

function calReturn() {
  const P = state.settings.printer;
  calSend(["G90", `G0 Z${n2(P.pen_up_z)} F${Math.round(P.feed_z)}`, `G0 X${n2(CAL.r.x0 ?? 0)} Y${n2(CAL.r.y0 ?? 0)} F${Math.round(P.feed_travel)}`]);
}

function calStepResult() {
  const w = calWindow(CAL.r);
  const probs = calProblems(w);
  const S = state.settings;
  const m = S.printer.safety_margin;
  const out = [h("h3", { text: "Итог" })];
  if (w) {
    out.push(h("p", { text: "Куда достаёт карандаш, если считать от угла листа:" }),
      h("div", { class: "cal-table" },
        h("span", { text: "По X" }), h("b", { text: `${calNum(w.x_min)} … ${calNum(w.x_max)} мм` }),
        h("span", { text: "По Y" }), h("b", { text: `${calNum(w.y_min)} … ${calNum(w.y_max)} мм` }),
        h("span", { text: "С запасом" }), h("span", { text: `${m} мм с каждой стороны` })));
  }
  if (probs.length) out.push(h("div", { class: "cal-box err" }, probs.map((p) => h("p", { text: p }))));
  else out.push(h("div", { class: "cal-box ok" }, h("p", { text: "Замеры в порядке. «Сохранить» запишет их в настройки, потом проверим углы." })));
  const tb = S.printer.table;
  const tnum = (key, label) => {
    const inp = h("input", { type: "number", step: "1", min: "0", placeholder: "авто", value: tb[key] ?? "" });
    inp.onchange = () => { tb[key] = inp.value === "" ? null : Math.max(0, Number(inp.value)); calArt(); };
    return h("label", {}, h("span", { text: label }), inp, h("i", { class: "muted", text: "мм" }));
  };
  const tbool = (key, label) => {
    const inp = h("input", { type: "checkbox" });
    inp.checked = !!tb[key];
    inp.onchange = () => { tb[key] = inp.checked; calArt(); };
    return h("label", {}, inp, h("span", { text: label }));
  };
  out.push(h("details", { class: "cal-more" }, h("summary", { text: "Стол (необязательно)" }),
    h("p", { class: "muted", text: "Измерь линейкой стол от упоров и отметь, куда лист может свисать за край, если корпус не мешает." }),
    h("div", { class: "cal-row" }, tnum("table_x", "по +X"), tnum("table_y", "по +Y")),
    h("div", { class: "cal-row" }, tbool("overhang_x", "лист может свисать по +X")),
    h("div", { class: "cal-row" }, tbool("overhang_y", "лист может свисать по +Y"))));
  return out;
}

async function calSave() {
  const S = state.settings;
  const w = calWindow(CAL.r);
  if (!w || calProblems(w).length) return false;
  S.printer.table.readings = { ...CAL.r };
  S.printer.travel = w;
  lastTravel = { ...w };
  saveSettings();
  await flushSave();
  syncForms();
  refreshPreview();
  try {
    CAL.info = await api("POST", "/api/calibration/info", S);
  } catch (e) {
    CAL.info = { errors: e.data?.errors || [e.message], warnings: [] };
  }
  return true;
}

function calStepCheck() {
  const m = state.settings.printer.safety_margin;
  const out = [h("h3", { text: "Проверка по углам" }),
    h("p", { text: `Карандаш по очереди придёт в четыре угла окна (с запасом ${m} мм), в каждом коснётся бумаги и поднимется. Следи, чтобы он нигде не упирался.` })];
  const corners = (CAL.info?.corners || []).map(([x, y], i) => (x < 0 || y < 0 ? i + 1 : 0)).filter(Boolean);
  for (const w of CAL.info?.warnings || []) {
    if (!/^\S+ \d+ \(X-?[\d.]+ Y-?[\d.]+\)/.test(w)) out.push(h("div", { class: "cal-box warn" }, h("p", { text: w })));
  }
  if (corners.length) {
    out.push(h("div", { class: "cal-box warn" }, h("p", {
      text: `${corners.length > 1 ? "Углы" : "Угол"} ${corners.join(", ")} — за краем листа, со стороны упоров: там карандаш коснётся стола или упора. Подложи туда бумагу или убедись, что там можно касаться.`,
    })));
  }
  for (const e of CAL.info?.errors || []) out.push(h("div", { class: "cal-box err" }, h("p", { text: e })));
  if (calCable()) {
    if (!calConnected()) return [...out, calNotConnected()];
    out.push(h("p", { class: "muted", text: "Перед запуском: лист в упорах, ноль на месте (если двигал карандаш руками — поставь ноль заново на шаге 3)." }),
      h("div", { class: "cal-row" },
        h("button", { class: "btn primary cal-big needs-idle", onclick: calRunCheck }, "Проверить углы по кабелю"),
        h("span", { class: "muted", id: "calProgress" })));
  } else {
    out.push(h("ol", {},
      h("li", { text: "Скачай проверочный файл и запиши на карту." }),
      h("li", { text: "Поставь лист в упоры, карандаш — в угол до касания, и запусти файл." })),
    h("div", { class: "cal-row" }, h("button", { class: "btn primary", onclick: () => calDownload("/api/calibration/check_gcode") }, "Скачать проверочный файл")));
  }
  out.push(h("p", { class: "muted", text: "Упёрся в каком-то углу — вернись к шагу этого края и уменьши число на 1–2 мм:" }),
    h("div", { class: "cal-row" }, Object.entries(CAL_DIRS).map(([k, d]) =>
      h("button", { class: "btn small", onclick: () => calGo(CAL_STEPS.findIndex((s) => s.id === k)) }, `Край ${d.name}`))));
  return out;
}

async function calRunCheck() {
  await flushSave();
  try {
    await api("POST", "/api/printer/print?kind=reach", state.settings);
    CAL.checkStarted = true;
    schedulePoll(100);
  } catch (e) {
    toast(e.message, 7000);
  }
}

async function calDownload(url) {
  await flushSave();
  try {
    const r = await api("POST", url, state.settings);
    await saveText(r.filename, r.gcode);
  } catch (e) {
    toast(e.data?.errors?.join("; ") || e.message, 8000);
  }
}

function calRender() {
  const id = CAL_STEPS[CAL.step].id;
  const dots = $("calSteps");
  dots.innerHTML = "";
  CAL_STEPS.forEach((s, i) => {
    const li = h("li", { title: s.dot, text: String(i + 1) });
    if (i === CAL.step) li.className = "on";
    else if (i <= CAL.maxStep) {
      li.className = i < CAL.step || calStepDone(s.id) ? "done" : "reach";
      li.onclick = () => calGo(i);
    }
    dots.appendChild(li);
  });
  const content = $("calContent");
  content.innerHTML = "";
  const parts =
    id === "mode" ? calStepMode()
    : id === "prep" ? calStepPrep()
    : id === "zero" ? calStepZero()
    : CAL_DIRS[id] ? calStepDir(id)
    : id === "result" ? calStepResult()
    : calStepCheck();
  content.append(...parts);
  $("calCount").textContent = `Шаг ${CAL.step + 1} из ${CAL_STEPS.length} · ${CAL_STEPS[CAL.step].dot}`;
  $("calBack").disabled = CAL.step === 0;
  $("calNext").textContent = id === "result" ? "Сохранить →" : id === "check" ? "Готово" : "Далее →";
  calArt();
  calLive();
}

function calStepDone(id) {
  if (CAL_DIRS[id]) return typeof CAL.r[id] === "number";
  if (id === "zero") return CAL.zeroed || typeof CAL.r.x0 === "number";
  return false;
}

function calArt() {
  const id = CAL_STEPS[CAL.step].id;
  const r = CAL.r;
  const S = state.settings;
  const [W, H] = CAL_SHEET;
  const guess = { px: W + 22, py: H + 18, mx: -18, my: -16 };
  const val = (k) => (typeof r[k] === "number" ? r[k] - (k[1] === "x" ? r.x0 ?? 0 : r.y0 ?? 0) : null);
  const edge = (k) => val(k) ?? guess[k];
  const xs = [edge("mx"), edge("px"), 0, W], ys = [edge("my"), edge("py"), 0, H];
  const X0 = Math.min(...xs) - 26, X1 = Math.max(...xs) + 44, Y0 = Math.min(...ys) - 26, Y1 = Math.max(...ys) + 26;
  const VW = 400, VH = 440;
  const k = Math.min((VW - 20) / (X1 - X0), (VH - 20) / (Y1 - Y0));
  const ox = (VW - (X1 - X0) * k) / 2 - X0 * k, oy = VH - (VH - (Y1 - Y0) * k) / 2 + Y0 * k;
  const px = (x) => +(ox + x * k).toFixed(1), py = (y) => +(oy - y * k).toFixed(1);
  const rect = (x0, y0, x1, y1, style, extra = "") =>
    `<rect x="${px(x0)}" y="${py(y1)}" width="${((x1 - x0) * k).toFixed(1)}" height="${((y1 - y0) * k).toFixed(1)}" style="${style}" ${extra}/>`;
  const text = (x, y, t, style = "fill:var(--muted)", anchor = "start") =>
    `<text x="${x}" y="${y}" text-anchor="${anchor}" style="${style}">${t}</text>`;
  let s = `<svg viewBox="0 0 ${VW} ${VH}" xmlns="http://www.w3.org/2000/svg">`;
  s += `<defs><marker id="calArrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" style="fill:var(--accent)"/></marker>
    <filter id="calShadow" x="-10%" y="-10%" width="120%" height="120%"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-opacity=".18"/></filter></defs>`;
  s += rect(X0 + 6, Y0 + 6, X1 - 6, Y1 - 6, "fill:var(--panel);stroke:var(--line)", 'rx="14"');
  s += text(px((X0 + X1) / 2), py(Y1 - 6) + 15, "стол принтера", "fill:var(--muted)", "middle");
  const sheetMove = id === "prep" ? 'class="cal-sheet-in"' : "";
  s += `<g ${sheetMove}>${rect(0, 0, W, H, "fill:var(--paper);stroke:var(--line)", 'filter="url(#calShadow)"')}`;
  s += text(px(W / 2), py(H / 2), "лист A4", "fill:var(--muted);font-size:13px", "middle") + "</g>";
  s += `<path d="M${px(-3)} ${py(95)} V${py(-3)} H${px(95)}" style="fill:none;stroke:var(--text);stroke-width:6;stroke-linecap:round"/>`;
  s += text(px(-3) + 8, py(-3) + 18, "упоры", "fill:var(--text);font-weight:600");
  const zeroCls = id === "zero" ? 'class="cal-pulse"' : "";
  s += `<circle cx="${px(0)}" cy="${py(0)}" r="5" style="fill:var(--accent)" ${zeroCls}/>`;
  s += text(px(0) + 9, py(0) - 8, "0, 0", "fill:var(--accent);font-weight:600");
  const axisLabel = (x1, y1, label) =>
    `<line x1="${px(0)}" y1="${py(0)}" x2="${x1}" y2="${y1}" style="stroke:var(--muted);stroke-width:1;opacity:.5" marker-end="url(#calArrow)"/>` +
    text(x1 + 4, y1 + 4, label, "fill:var(--muted)");
  if (id === "mode" || id === "prep") {
    s += axisLabel(px(60), py(0), "+X") + axisLabel(px(0), py(60), "+Y");
  }
  for (const key of ["px", "py", "mx", "my"]) {
    const v = val(key);
    if (v == null) continue;
    const on = key === id;
    const st = `stroke:var(--danger);stroke-width:${on ? 2.2 : 1.4};stroke-dasharray:6 4`;
    if (key[1] === "x") {
      s += `<line x1="${px(v)}" y1="${py(edge("my") - 8)}" x2="${px(v)}" y2="${py(edge("py") + 8)}" style="${st}"/>`;
      s += text(px(v), py(edge("my") - 8) + 13, calNum(v), "fill:var(--danger);font-weight:600", "middle");
    } else {
      s += `<line x1="${px(edge("mx") - 8)}" y1="${py(v)}" x2="${px(edge("px") + 8)}" y2="${py(v)}" style="${st}"/>`;
      s += text(px(edge("px") + 8) + 3, py(v) + 4, calNum(v), "fill:var(--danger);font-weight:600", "start");
    }
  }
  const w = calWindow(r);
  const m = S.printer.safety_margin;
  let anim = "";
  let pen = null;
  if ((id === "result" || id === "check") && w) {
    s += rect(w.x_min, w.y_min, w.x_max, w.y_max, "fill:color-mix(in srgb, var(--ok) 10%, transparent);stroke:var(--ok);stroke-width:1.5;stroke-dasharray:7 4", 'class="cal-fade"');
    s += rect(w.x_min + m, w.y_min + m, w.x_max - m, w.y_max - m, "fill:none;stroke:var(--ok);stroke-width:1");
    s += text(px((w.x_min + w.x_max) / 2), py(w.y_max) + 18, "сюда достаёт карандаш", "fill:var(--ok);font-weight:600", "middle");
    const cs = [[w.x_min + m, w.y_min + m], [w.x_max - m, w.y_min + m], [w.x_max - m, w.y_max - m], [w.x_min + m, w.y_max - m]];
    for (const [x, y] of cs) s += `<circle cx="${px(x)}" cy="${py(y)}" r="4" style="fill:var(--ok)"/>`;
    if (id === "check") {
      pen = cs[0];
      const frames = cs.concat([cs[0]]).map(([x, y], i) => `${i * 25}% { transform: translate(${px(x)}px, ${py(y)}px); }`).join(" ");
      anim = `@keyframes calPen { ${frames} } .cal-pen { animation: calPen 6s var(--ease) infinite; }`;
    }
  }
  if (id === "zero") {
    pen = [W * 0.62, H * 0.72];
    anim = `@keyframes calPen { 0%, 10% { transform: translate(${px(W * 0.62)}px, ${py(H * 0.72)}px); } 55%, 100% { transform: translate(${px(0)}px, ${py(0)}px); } }
      .cal-pen { animation: calPen 3.4s var(--ease) infinite; }
      @keyframes calDip { 0%, 60% { transform: scale(1); } 72% { transform: scale(.55); } 84%, 100% { transform: scale(1); } }
      .cal-pen .tip { animation: calDip 3.4s ease-in-out infinite; transform-box: fill-box; transform-origin: center; }`;
  }
  if (CAL_DIRS[id]) {
    const d = CAL_DIRS[id];
    const target = edge(id);
    const to = d.axis === "X" ? [target, 0] : [0, target];
    s += `<line x1="${px(0)}" y1="${py(0)}" x2="${px(to[0])}" y2="${py(to[1])}" style="stroke:var(--accent);stroke-width:2.5;stroke-dasharray:2 6;stroke-linecap:round" marker-end="url(#calArrow)"/>`;
    const lx = px(to[0]) + (d.axis === "X" ? (d.sign > 0 ? -4 : 4) : 10);
    const ly = py(to[1]) + (d.axis === "Y" ? (d.sign > 0 ? 14 : -8) : -12);
    s += text(lx, ly, d.name, "fill:var(--accent);font-weight:700;font-size:14px", d.axis === "X" ? (d.sign > 0 ? "end" : "start") : "start");
    if (val(id) == null) {
      const bar = d.axis === "X"
        ? `<line x1="${px(target)}" y1="${py(-14)}" x2="${px(target)}" y2="${py(14)}"`
        : `<line x1="${px(-14)}" y1="${py(target)}" x2="${px(14)}" y2="${py(target)}"`;
      s += `${bar} style="stroke:var(--danger);stroke-width:4;stroke-linecap:round" class="cal-blink"/>`;
      s += text(d.axis === "X" ? px(target) : px(16), d.axis === "X" ? py(-14) + 14 : py(target) - 6, "упрётся здесь?", "fill:var(--danger)", d.axis === "X" ? "middle" : "start");
    }
    pen = [0, 0];
    anim = `@keyframes calPen { 0%, 12% { transform: translate(${px(0)}px, ${py(0)}px); } 62%, 82% { transform: translate(${px(to[0])}px, ${py(to[1])}px); } 100% { transform: translate(${px(0)}px, ${py(0)}px); } }
      .cal-pen { animation: calPen 3.6s var(--ease) infinite; }`;
  }
  if (id === "prep" || id === "mode") pen = [W * 0.62, H * 0.72];
  if (pen) {
    const style = anim ? "" : `transform: translate(${px(pen[0])}px, ${py(pen[1])}px)`;
    s += `<g class="cal-pen" style="${style}"><circle r="13" style="fill:var(--accent);opacity:.14"/><g class="tip"><circle r="5" style="fill:var(--accent)"/></g>
      <line x1="-18" y1="0" x2="-8" y2="0" style="stroke:var(--accent);stroke-width:1.5"/><line x1="8" y1="0" x2="18" y2="0" style="stroke:var(--accent);stroke-width:1.5"/>
      <line x1="0" y1="-18" x2="0" y2="-8" style="stroke:var(--accent);stroke-width:1.5"/><line x1="0" y1="8" x2="0" y2="18" style="stroke:var(--accent);stroke-width:1.5"/>
      <text x="16" y="-12" style="fill:var(--accent);font-size:11px">карандаш</text></g>`;
  }
  const sheetIn = `@keyframes calSheet { 0% { transform: translate(${(40 * k).toFixed(1)}px, ${(-40 * k).toFixed(1)}px) rotate(4deg); opacity: 0; }
      35% { opacity: 1; } 60%, 100% { transform: none; } } .cal-sheet-in { animation: calSheet 2.2s var(--ease) both; transform-box: fill-box; }`;
  s += `<style>${anim} ${sheetIn}
    @keyframes calPulse { 50% { r: 9; opacity: .55; } } .cal-pulse { animation: calPulse 1.4s ease-in-out infinite; }
    @keyframes calBlink { 50% { opacity: .25; } } .cal-blink { animation: calBlink 1.1s ease-in-out infinite; }
    .cal-fade { animation: fade-in .5s ease-out; }</style>`;
  s += "</svg>";
  $("calArt").innerHTML = s;
}

$("calClose").onclick = () => openCalib(false);
$("calibPanel").addEventListener("mousedown", (e) => { if (e.target === $("calibPanel")) openCalib(false); });
$("calBack").onclick = () => calGo(CAL.step - 1);
$("calNext").onclick = async () => {
  const id = CAL_STEPS[CAL.step].id;
  if (!calCanNext()) return;
  if (id === "result") {
    if (!(await calSave())) return;
    toast("Калибровка записана в настройки");
  }
  if (id === "check") {
    openCalib(false);
    return;
  }
  calGo(CAL.step + 1);
};
addEventListener("themechange", () => CAL.open && calArt());
if (new URLSearchParams(location.search).get("open") === "calib") setTimeout(() => openCalib(true), 600);
