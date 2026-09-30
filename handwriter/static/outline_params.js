"use strict";

const OUTLINE_PARAMS = [
  { key: "prune", label: "Порог веточек (доля высоты строчной, за краем штриха)", min: 0, max: 0.4, step: 0.005,
    hint: "Больше: сильнее срезаются отростки. Слишком много: пропадают короткие хвостики." },
  { key: "extend", label: "Достройка концов (× полутолщины штриха)", min: 0, max: 2, step: 0.05,
    hint: "1 = конец линии доходит до края буквы. 0 = концы остаются укороченными." },
  { key: "smooth", label: "Сглаживание (окно, доля высоты строчной)", min: 0, max: 0.2, step: 0.005,
    hint: "Больше: плавнее, но острые углы скругляются." },
  { key: "simplify", label: "Упрощение (допуск, доля высоты строчной)", min: 0, max: 0.03, step: 0.001,
    hint: "Больше: меньше точек в gcode, но линия грубее." },
  { key: "junction_merge", label: "Склейка близких развилок (× полутолщины)", min: 0, max: 6, step: 0.1,
    hint: "Пересечение двух линий скелет даёт двумя развилками. Больше: такие развилки считаются одной." },
  { key: "px_per_em", label: "Разрешение растеризации (px на em)", min: 400, max: 3000, step: 50,
    hint: "Больше: точнее и медленнее. Около 1500 обычно хватает." },
];

const RANDOM_PARAMS = [
  { key: "size", label: "Размер каждой буквы, ±%", min: 0, max: 15, step: 0.5, def: 3,
    hint: "Средний размер остаётся равным заданной высоте строчной." },
  { key: "slant", label: "Наклон каждой буквы, ±°", min: 0, max: 10, step: 0.1, def: 2 },
  { key: "offset", label: "Смещение буквы по высоте, ±мм", min: 0, max: 1.5, step: 0.05, def: 0.3 },
  { key: "letter_spacing", label: "Межбуквенный интервал, ±%", min: 0, max: 25, step: 0.5, def: 5 },
  { key: "word_spacing", label: "Межсловный интервал, ±%", min: 0, max: 50, step: 1, def: 10 },
  { key: "drift", label: "Уплывание базовой линии, ±мм", min: 0, max: 2, step: 0.05, def: 0.5,
    hint: "Плавная волна вдоль строки." },
  { key: "line_start", label: "Начало строки, ±мм", min: 0, max: 5, step: 0.1, def: 1 },
  { key: "right_edge", label: "Правый край, ±мм", min: 0, max: 8, step: 0.1, def: 1.5 },
  { key: "jitter", label: "Дрожание линии, мм", min: 0, max: 0.5, step: 0.01, def: 0.1,
    hint: "Гладкий шум по длине пути, не независимый в каждой точке." },
];

const CONNECTION_PARAMS = [
  { key: "distance", label: "Связка, если конец и начало ближе (доля высоты строчной)", min: 0, max: 0.6, step: 0.01,
    def: 0.15, hint: "Ищется до случайных сдвигов. После сдвигов точки соединяются плавной кривой." },
];

function buildSliders(container, defs, getObj, onChange) {
  container.innerHTML = "";
  for (const p of defs) {
    const div = document.createElement("div");
    div.className = "slider";
    if (p.hint) div.title = p.hint;
    div.innerHTML = `<label>${p.label}</label>
      <input type="range" min="${p.min}" max="${p.max}" step="${p.step}">
      <input type="number" min="${p.min}" max="${p.max}" step="${p.step}">`;
    const [range, num] = div.querySelectorAll("input");
    const v = getObj()[p.key];
    range.value = v; num.value = v;
    range.addEventListener("input", () => { num.value = range.value; getObj()[p.key] = Number(range.value); onChange(); });
    num.addEventListener("input", () => {
      if (num.value === "" || isNaN(Number(num.value))) return;
      range.value = num.value; getObj()[p.key] = Number(num.value); onChange();
    });
    container.appendChild(div);
  }
}

function buildOutlineSliders(container, getOpts, onChange) {
  buildSliders(container, OUTLINE_PARAMS, getOpts, onChange);
}
