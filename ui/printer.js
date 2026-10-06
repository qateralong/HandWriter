"use strict";

const PR = {
  open: localGet("hw-printer-open") === "1",
  st: null,
  gen: -1,
  segs: [],
  owner: null,
  logSeq: 0,
  ports: [],
  step: Number(localGet("hw-jog-step")) || 10,
  timer: 0,
  inFlight: false,
};

const PR_STATUS = {
  disconnected: "не подключён",
  connecting: "подключение…",
  idle: "готов",
  printing: "печатает",
  paused: "пауза",
  halted: "остановлен",
};

const n2 = (v) => (Math.round(v * 100) / 100).toFixed(2);

function prBusy() {
  const s = PR.st?.status;
  return s === "printing" || s === "paused";
}

function prReady() {
  return PR.st?.status === "idle";
}

function openPrinter(open) {
  PR.open = open;
  localPut("hw-printer-open", open ? "1" : "0");
  $("printerPanel").classList.toggle("open", open);
  $("printerBtn").setAttribute("aria-expanded", String(open));
  if (open) {
    closePopovers();
    if (!PR.ports.length) loadPorts();
    renderPrinter();
  }
  schedulePoll(0);
}

async function loadPorts() {
  try {
    PR.ports = await api("GET", "/api/printer/ports");
  } catch {
    PR.ports = [];
  }
  const sel = $("ppPort");
  const last = localGet("hw-printer-port");
  sel.innerHTML = "";
  for (const p of PR.ports) sel.add(new Option(p.label, p.path));
  sel.add(new Option(PR.ports.length ? "Другой порт…" : "Портов не найдено — указать вручную…", "*"));
  if (last && PR.ports.some((p) => p.path === last)) sel.value = last;
  else if (last && !PR.ports.length) {
    sel.value = "*";
    $("ppPath").value = last;
  }
  $("ppPath").hidden = sel.value !== "*";
  renderPrinter();
}

function portPath() {
  return $("ppPort").value === "*" ? $("ppPath").value.trim() : $("ppPort").value;
}

async function prCall(url, body) {
  try {
    await api("POST", url, body);
    schedulePoll(150);
    return true;
  } catch (e) {
    toast(e.message, 7000);
    return false;
  }
}

async function prConnect() {
  if (PR.st?.connected && PR.st.status !== "halted") {
    await prCall("/api/printer/disconnect");
    return;
  }
  const port = portPath();
  if (!port) {
    toast("Выбери порт принтера");
    return;
  }
  localPut("hw-printer-port", port);
  localPut("hw-printer-baud", $("ppBaud").value);
  $("ppConnect").disabled = true;
  PR.connErr = "";
  try {
    await api("POST", "/api/printer/connect", { port, baud: Number($("ppBaud").value) });
  } catch (e) {
    PR.connErr = e.message;
  }
  $("ppConnect").disabled = false;
  schedulePoll(150);
  renderPrinter();
}

function prSend(lines) {
  return prCall("/api/printer/command", { cmd: lines.join("\n") });
}

function jog(axis) {
  const P = state.settings.printer;
  const a = axis[0];
  const step = a === "Z" ? Math.min(PR.step, axis[1] === "-" ? 1 : 10) : PR.step;
  const d = axis[1] === "-" ? -step : step;
  const feed = a === "Z" ? P.feed_z : P.feed_travel;
  prSend(["G91", `G0 ${a}${n2(d)} F${Math.round(feed)}`, "G90"]);
}

async function prZero() {
  if (!confirm("Карандаш касается бумаги в углу листа? Эта точка станет нулём (X0 Y0 Z0), затем карандаш поднимется.")) return;
  try {
    const r = await api("POST", "/api/calibration/zero_gcode", state.settings);
    const lines = r.gcode.split("\n").map((l) => l.split(";")[0].trim()).filter(Boolean);
    await prSend(lines);
  } catch (e) {
    toast(e.message, 7000);
  }
}

function prJobs() {
  if (state.mode === "drawing") {
    const parts = state.preview?.parts || [];
    if (!parts.length) return [];
    return parts.map((p) => ({
      kind: "drawing",
      part: p.index,
      title: parts.length > 1 ? `Проход ${p.index} из ${parts.length}` : "Чертёж",
      sub: `${rotationText(p)} · ${fmtTime(p.stats?.time_s)}`,
      rotation: p,
    }));
  }
  if (!state.text?.strokes?.length) return [];
  return [{ kind: "text", part: 0, title: "Этот лист", sub: fmtTime(state.text.stats?.time_s) }];
}

function prErrors() {
  const p = state.mode === "drawing" ? state.preview : state.text;
  return (p?.errors || []).length > 0;
}

async function prPrint(job, test) {
  const what = test ? "тест рамки" : job.title.toLowerCase();
  if (!test && !confirm(`Начать печать: ${what}?\nПроверь, что лист закреплён и карандаш над нулём.`)) return;
  await flushSave();
  const q = new URLSearchParams({ kind: job.kind, part: String(job.part || 1), test: test ? "1" : "0" });
  busy(true, "готовлю gcode…");
  try {
    await api("POST", "/api/printer/print?" + q, state.settings);
    schedulePoll(100);
  } catch (e) {
    showMessages(e.data?.errors || [e.message], []);
    toast(e.message, 7000);
  } finally {
    busy(false);
  }
}

function renderJobs() {
  const box = $("ppJobs");
  const jobs = prJobs();
  const st = PR.st;
  const active = st?.job;
  const fin = st?.finished;
  box.innerHTML = "";
  if (!jobs.length) {
    const p = document.createElement("p");
    p.className = "fhint";
    p.textContent = state.mode === "drawing" ? "Загрузи чертёж — здесь появятся проходы." : "Напиши текст — здесь появится кнопка печати.";
    box.appendChild(p);
    return;
  }
  const can = prReady() && !prErrors();
  for (const j of jobs) {
    const row = document.createElement("div");
    row.className = "pp-job";
    const done = PR.done.has(`${j.kind}:${j.part}`);
    if (done) row.classList.add("done");
    if (active && active.kind === j.kind && (j.kind !== "drawing" || active.part === j.part)) row.classList.add("active");
    const what = document.createElement("div");
    what.className = "what";
    what.textContent = j.title;
    const small = document.createElement("small");
    small.textContent = j.sub;
    what.appendChild(small);
    const t = document.createElement("button");
    t.className = "btn small";
    t.textContent = "Тест";
    t.title = "Обвести рамку чертежа, не опуская карандаш на бумагу";
    t.disabled = !can;
    t.onclick = () => prPrint(j, true);
    const go = document.createElement("button");
    go.className = "btn small primary";
    go.textContent = "Печать";
    go.disabled = !can;
    go.onclick = () => prPrint(j, false);
    row.append(what, t, go);
    box.appendChild(row);
  }
  if (state.mode === "drawing" && jobs.length > 1) {
    box.appendChild(h("div", { class: "pp-row pp-align" },
      h("span", { class: "muted", text: "Линии на стыках не сходятся?" }),
      h("button", { class: "btn small", onclick: () => openAlign(true) }, "Совместить проходы…")));
  }
  if (fin && fin.kind === "drawing" && state.mode === "drawing") {
    const next = jobs.find((j) => j.part === fin.part + 1);
    if (next && !active) {
      const hint = document.createElement("div");
      hint.className = "pp-next";
      hint.textContent = `Проход ${fin.part} готов. Переложи лист: ${rotationText(next.rotation)} — и запускай проход ${next.part}.`;
      box.appendChild(hint);
    }
  }
}

function renderPrinter() {
  const st = PR.st;
  const status = st?.status || "disconnected";
  const dot = $("printerBtn").querySelector(".dot");
  dot.className = "dot " + status;
  setTip($("printerBtn"), `Принтер по кабелю: ${PR_STATUS[status]}`);
  if (!PR.open) return;
  const pill = $("ppStatus");
  pill.className = "pill " + status;
  pill.textContent = PR_STATUS[status];
  const msg = $("ppMsg");
  const connErr = !st?.connected && PR.connErr;
  msg.textContent = connErr || st?.message || "";
  msg.className = "pp-msg" + (connErr || st?.alert ? " err" : "");
  const connected = !!st?.connected && status !== "halted";
  $("ppPort").disabled = $("ppPath").disabled = $("ppBaud").disabled = $("ppRefresh").disabled = connected;
  $("ppConnect").textContent = connected ? "Отключить" : "Подключить";
  $("ppConnect").classList.toggle("primary", !connected);
  $("ppConnect").disabled = connected && prBusy();
  $("ppMove").hidden = $("ppPrint").hidden = $("ppConsole").hidden = !st?.connected;
  $("ppEmergency").hidden = !st?.connected;
  if (!st?.connected) return;

  const p = st.position, r = st.reported;
  const pen = st.pen_down == null ? "" : st.pen_down ? " · карандаш внизу" : " · карандаш поднят";
  $("ppPos").innerHTML = "";
  for (const [k, v] of [["X", p.x], ["Y", p.y], ["Z", p.z]]) {
    const b = document.createElement("b");
    b.textContent = k + " ";
    $("ppPos").append(b, n2(v) + "  ");
  }
  $("ppPos").append(pen);
  $("ppPos").title = r ? `Принтер сообщает: X${n2(r.x)} Y${n2(r.y)} Z${n2(r.z)}` : "";
  const canMove = status === "idle" || status === "paused";
  for (const b of $("ppMove").querySelectorAll("button")) b.disabled = !canMove;
  $("ppCalib").disabled = false;
  for (const b of $("ppSteps").children) b.classList?.toggle("on", Number(b.dataset.step) === PR.step);
  $("ppZero").disabled = status !== "idle";
  $("ppCmd").disabled = !canMove;
  $("ppCmdForm").querySelector("button").disabled = !canMove;

  renderJobs();
  const j = st.job;
  $("ppProgress").hidden = !j;
  if (j) {
    $("ppBar").style.width = `${j.percent}%`;
    const rem = j.remaining_s != null ? ` · осталось ≈ ${fmtTime(j.remaining_s)}` : "";
    $("ppPText").textContent = `${j.label}: ${j.percent}% (${j.done} из ${j.total} строк) · идёт ${fmtTime(j.elapsed_s)}${rem}`;
    $("ppPause").hidden = status === "paused";
    $("ppResume").hidden = status !== "paused";
  }
  $("ppEmergency").hidden = status === "halted";
}

function appendLog(items) {
  if (!items.length) return;
  const box = $("ppLog");
  const stick = box.scrollTop + box.clientHeight >= box.scrollHeight - 8;
  for (const [seq, text] of items) {
    PR.logSeq = Math.max(PR.logSeq, seq);
    const d = document.createElement("div");
    d.textContent = text;
    if (text.startsWith("<")) d.className = /error|halted|kill/i.test(text) ? "bad" : "in";
    else if (text.startsWith("—")) d.className = "sys";
    box.appendChild(d);
  }
  while (box.childElementCount > 400) box.firstElementChild.remove();
  if (stick) box.scrollTop = box.scrollHeight;
}

PR.done = new Set();

async function poll() {
  if (PR.inFlight) return;
  PR.inFlight = true;
  try {
    const q = new URLSearchParams({ gen: String(Math.max(0, PR.gen)), from: String(PR.segs.length), log: String(PR.logSeq) });
    const st = await api("GET", "/api/printer/status?" + q);
    const prevFinished = JSON.stringify(PR.st?.finished);
    PR.st = st;
    const s = st.segs;
    if (s.reset || s.gen !== PR.gen) {
      PR.segs = [];
      PR.gen = s.gen;
    }
    if (s.from === PR.segs.length) PR.segs.push(...s.items);
    PR.owner = s.kind ? { kind: s.kind, part: s.part, test: s.test } : null;
    if (st.finished && JSON.stringify(st.finished) !== prevFinished && PR.owner && !PR.owner.test) {
      PR.done.add(`${st.finished.kind}:${st.finished.part}`);
    }
    appendLog(st.log);
    renderPrinter();
    if (typeof calLive === "function") calLive();
    if (typeof alLive === "function") alLive();
    if (st.connected || s.items.length) draw();
  } catch {
    PR.st = null;
    renderPrinter();
  } finally {
    PR.inFlight = false;
  }
  const s = PR.st?.status;
  const calib = (typeof CAL !== "undefined" && CAL.open) || (typeof AL !== "undefined" && AL.open);
  schedulePoll(s === "printing" || (calib && PR.st?.connected) ? 400 : PR.open || calib || s === "paused" || s === "connecting" ? 1000 : 3000);
}

function schedulePoll(ms) {
  clearTimeout(PR.timer);
  PR.timer = setTimeout(poll, ms);
}

function printerOverlay(T, k, kind, part) {
  const o = PR.owner;
  if (!o || o.kind !== kind || (kind === "drawing" && o.part !== part)) return;
  ctx.save();
  if (PR.segs.length) {
    ctx.lineCap = "round";
    ctx.strokeStyle = css("--printed");
    ctx.lineWidth = Math.max(1.4, 0.45 * k);
    ctx.beginPath();
    for (const [x0, y0, x1, y1] of PR.segs) {
      const a = T([x0, y0]), b = T([x1, y1]);
      ctx.moveTo(a[0], a[1]);
      ctx.lineTo(b[0], b[1]);
    }
    ctx.stroke();
  }
  const sp = PR.st?.connected && PR.st.sheet_position;
  if (sp) {
    const [x, y] = T(sp);
    ctx.beginPath();
    ctx.arc(x, y, 6, 0, Math.PI * 2);
    ctx.lineWidth = 2;
    ctx.strokeStyle = css("--danger");
    ctx.fillStyle = PR.st.pen_down ? css("--danger") : "rgba(0,0,0,0)";
    ctx.fill();
    ctx.stroke();
  }
  ctx.restore();
}

$("printerBtn").onclick = () => openPrinter(!PR.open);
$("ppClose").onclick = () => openPrinter(false);
$("ppRefresh").onclick = loadPorts;
$("ppPort").onchange = () => {
  $("ppPath").hidden = $("ppPort").value !== "*";
  if (!$("ppPath").hidden) $("ppPath").focus();
};
$("ppBaud").value = localGet("hw-printer-baud") || "115200";
$("ppConnect").onclick = prConnect;
$("ppSteps").onclick = (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  PR.step = Number(b.dataset.step);
  localPut("hw-jog-step", String(PR.step));
  renderPrinter();
};
for (const b of document.querySelectorAll("[data-jog]")) b.onclick = () => jog(b.dataset.jog);
$("ppHome").onclick = () => prSend(["G28 X Y"]);
$("ppPenUp").onclick = () => prSend(["G90", `G0 Z${n2(state.settings.printer.pen_up_z)} F${Math.round(state.settings.printer.feed_z)}`]);
$("ppPenDown").onclick = () => prSend(["G90", `G1 Z${n2(state.settings.printer.pen_down_z)} F${Math.round(state.settings.printer.feed_z)}`]);
$("ppZero").onclick = prZero;
$("ppCalib").onclick = () => openCalib(true);
$("ppPause").onclick = () => prCall("/api/printer/pause");
$("ppResume").onclick = () => prCall("/api/printer/resume");
$("ppStop").onclick = () => {
  if (confirm("Остановить печать? Карандаш поднимется, продолжить с этого места будет нельзя.")) prCall("/api/printer/stop");
};
$("ppEmergency").onclick = () => prCall("/api/printer/emergency");
$("ppCmdForm").onsubmit = (e) => {
  e.preventDefault();
  const v = $("ppCmd").value.trim();
  if (!v) return;
  prSend([v]).then((ok) => ok && ($("ppCmd").value = ""));
};

if (PR.open || new URLSearchParams(location.search).get("open") === "printer") openPrinter(true);
else schedulePoll(0);
