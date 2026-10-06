"use strict";

const getPath = (o, p) => p.split(".").reduce((a, k) => (a == null ? a : a[k]), o);
function setPath(o, p, v) {
  const ks = p.split(".");
  const last = ks.pop();
  ks.reduce((a, k) => a[k], o)[last] = v;
}

const num = (path, label, o = {}) => ({ type: "num", path, label, ...o });
const bool = (path, label, o = {}) => ({ type: "bool", path, label, ...o });
const sel = (path, label, options, o = {}) => ({ type: "sel", path, label, options, ...o });

const importKind = (P) => P?.import?.kind || "";
const travelOn = (S) => !!S.printer.travel;

const SHEET_MODE = {
  type: "sel",
  label: "Как ложится лист",
  options: [
    ["normal", "обычно"],
    ["a3", "A3 на столе A4 — 4 захода"],
    ["marked", "A4 по меткам на принтере — 2 захода"],
  ],
  get: (S) => (S.drawing.a3.enabled ? "a3" : S.drawing.marked.enabled ? "marked" : "normal"),
  set: (S, v) => {
    S.drawing.a3.enabled = v === "a3";
    S.drawing.marked.enabled = v === "marked";
    if (v === "a3") S.drawing.sheet.format = "A3";
    if (v === "marked") S.drawing.sheet.format = "A4";
  },
};

const QUICK = [
  {
    title: "Файл",
    fields: [
      { type: "file", label: "Чертёж" },
      sel("drawing.imp.units", "Единицы файла", [
        ["auto", "по файлу"], ["mm", "мм"], ["cm", "см"], ["m", "м"], ["in", "дюймы"], ["ft", "футы"],
        ["pt", "пункты (1/72″)"], ["px", "пиксели (96 на дюйм)"],
      ], { show: (S, P) => ["svg", "dxf", "test"].includes(importKind(P)) }),
      num("drawing.imp.pdf_page", "Страница PDF", { step: 1, min: 1, int: true, show: (S, P) => importKind(P) === "pdf" }),
    ],
  },
  {
    title: "Лист",
    fields: [
      sel("drawing.sheet.format", "Формат", [["A4", "A4 (210×297)"], ["A3", "A3 (297×420)"], ["custom", "свой размер"]],
        {
          disabled: (S) => S.drawing.a3.enabled || S.drawing.marked.enabled,
          get: (S) => (S.drawing.a3.enabled ? "A3" : S.drawing.marked.enabled ? "A4" : S.drawing.sheet.format),
        }),
      num("drawing.sheet.width", "Ширина", { unit: "мм", step: 1, show: (S) => S.drawing.sheet.format === "custom" }),
      num("drawing.sheet.height", "Высота", { unit: "мм", step: 1, show: (S) => S.drawing.sheet.format === "custom" }),
      sel("drawing.sheet.orientation", "Ориентация", [["auto", "авто (по чертежу)"], ["portrait", "книжная"], ["landscape", "альбомная"]]),
      SHEET_MODE,
      { type: "align", label: "Совместить проходы…", hint: "Если линии разных проходов не сходятся на стыке — тест с метками и поправка.",
        show: (S, P) => (P?.parts?.length || 0) > 1 },
    ],
  },
  {
    title: "Масштаб",
    fields: [
      sel("drawing.placement.scale_mode", "Масштаб", [
        ["fit", "вписать в лист"], ["fit_passes", "подобрать под проходы"], ["one_to_one", "1:1"], ["percent", "свой, %"],
      ], { onSet: (S, P) => { if (S.drawing.placement.scale_mode === "percent" && P?.scale) S.drawing.placement.percent = Number(P.scale.percent.toFixed(1)); } }),
      num("drawing.placement.percent", "Масштаб", { unit: "%", step: 1, min: 0.1, show: (S) => S.drawing.placement.scale_mode === "percent" }),
      sel("drawing.placement.anchor", "Где поставить", [["center", "по центру листа"], ["zero", "углом в точку нуля"]]),
    ],
  },
  {
    title: "Картинка",
    show: (S, P) => importKind(P) === "raster",
    fields: [
      sel("drawing.imp.raster_mode", "Как рисовать", [["centerlines", "по средним линиям"], ["fill", "штриховкой"]]),
      num("drawing.imp.fill_step", "Шаг штрихов", { unit: "мм", step: 0.05, min: 0.05, show: (S) => S.drawing.imp.raster_mode === "fill" }),
      sel("drawing.imp.fill_dir", "Направление штрихов", [["auto", "авто"], ["horizontal", "горизонтально"], ["vertical", "вертикально"]],
        { show: (S) => S.drawing.imp.raster_mode === "fill" }),
    ],
  },
];

const isOutlines = (S) => S.mode === "outlines";

function fontSummary(P) {
  const f = P?.font;
  if (!f) return "";
  const variants = Object.keys(f.variants || {}).length, ligs = (f.ligatures || []).length;
  let t = `${f.name}: ${f.glyph_count} глифов`;
  if (variants) t += `, варианты букв: ${variants}`;
  if (ligs) t += `, лигатуры: ${ligs}`;
  if (f.notes?.length) t += `. ${f.notes.join(" ")}`;
  return t;
}

const HAND = [
  {
    title: "Шрифт",
    fields: [
      { type: "font", label: "Шрифт" },
      { type: "info", label: "", text: (S, P) => fontSummary(P) },
      { type: "fontUpload", label: "Загрузить шрифт (TTF, OTF, SVG)" },
    ],
  },
  {
    title: "Тетрадь",
    fields: [
      { type: "profile", label: "Тетрадь" },
      num("typography.size_mm", "Высота строчной буквы", { unit: "мм", step: 0.1, min: 0.5 }),
      sel("sheet.ruling", "Линовка", [["grid", "клетка"], ["lines", "линейка"], ["none", "нет"]]),
      bool("typography.hyphenate", "Автоперенос слов"),
    ],
  },
  {
    title: "Живость почерка",
    fields: [
      bool("randomness.enabled", "Случайные отклонения"),
      bool("randomness.variants", "Разные варианты одной буквы", { show: (S) => S.randomness.enabled }),
      { type: "seed", label: "Другой случайный рисунок", show: (S) => S.randomness.enabled },
      bool("connections.enabled", "Соединять буквы"),
    ],
  },
  {
    title: "Проверка",
    fields: [{ type: "testFile", label: "Скачать тестовый файл (рамка поля письма и оси)" }],
  },
];

const randomField = (key, label, max, step, unit) =>
  num(`randomness.${key}`, label, { unit, step, min: 0, max, show: (S) => S.randomness.enabled });

const NOTEBOOK_TABS = [
  {
    id: "sheet",
    mode: "notebook",
    title: "Тетрадь",
    groups: [
      {
        title: "Лист",
        fields: [
          num("sheet.width", "Ширина", { unit: "мм", step: 0.5 }),
          num("sheet.height", "Высота", { unit: "мм", step: 0.5 }),
          num("sheet.margin_left", "Левое поле", { unit: "мм", step: 0.5 }),
          num("sheet.margin_right", "Правое поле", { unit: "мм", step: 0.5 }),
          num("sheet.first_line_top", "От верха до первой строки", { unit: "мм", step: 0.5 }),
          num("sheet.bottom_limit", "Нижний предел (от низа)", { unit: "мм", step: 0.5 }),
          num("sheet.line_pitch", "Шаг строк", { unit: "мм", step: 0.5 }),
          num("sheet.indent", "Красная строка", { unit: "мм", step: 0.5 }),
          num("sheet.grid_step", "Шаг клетки", { unit: "мм", step: 0.5, show: (S) => S.sheet.ruling === "grid" }),
        ],
      },
      {
        title: "Поправки",
        fields: [
          num("typography.baseline_shift", "Сдвиг букв над строкой", { unit: "мм", step: 0.1 }),
          num("typography.dx", "Сдвиг текста по X", { unit: "мм", step: 0.1 }),
          num("typography.dy", "Сдвиг текста по Y", { unit: "мм", step: 0.1 }),
          num("typography.rotation_deg", "Поворот текста", { unit: "°", step: 0.1,
            hint: "Против часовой стрелки — плюс, вокруг угла листа." }),
        ],
      },
      {
        title: "Вид листа",
        fields: [
          bool("preview.show_ruling", "Показывать линовку", { redraw: true }),
          bool("preview.show_travel", "Показывать холостые ходы", { redraw: true }),
        ],
      },
    ],
  },
  {
    id: "hand",
    mode: "notebook",
    title: "Почерк",
    groups: [
      {
        title: "Случайность",
        fields: [
          bool("randomness.enabled", "Случайные отклонения"),
          num("randomness.seed", "Номер случайного рисунка", { step: 1, int: true, show: (S) => S.randomness.enabled }),
          randomField("size", "Размер каждой буквы, ±", 30, 0.5, "%"),
          randomField("slant", "Наклон каждой буквы, ±", 20, 0.1, "°"),
          randomField("offset", "Смещение буквы по высоте, ±", 3, 0.05, "мм"),
          randomField("letter_spacing", "Межбуквенный интервал, ±", 50, 0.5, "%"),
          randomField("word_spacing", "Межсловный интервал, ±", 100, 1, "%"),
          randomField("drift", "Уплывание строки, ±", 5, 0.05, "мм"),
          randomField("line_start", "Начало строки, ±", 10, 0.1, "мм"),
          randomField("right_edge", "Правый край, ±", 8, 0.1, "мм"),
          randomField("jitter", "Дрожание линии", 0.5, 0.01, "мм"),
        ],
      },
      {
        title: "Связки",
        fields: [
          bool("connections.enabled", "Соединять буквы"),
          num("connections.distance", "Соединять, если ближе (доля высоты строчной)", { step: 0.01, min: 0, max: 0.6,
            show: (S) => S.connections.enabled }),
        ],
      },
      {
        title: "Контурные шрифты (TTF, OTF)",
        show: isOutlines,
        fields: [
          num("outline.prune", "Порог веточек", { step: 0.005, min: 0, max: 0.4, hint: "Больше — сильнее срезаются отростки скелета." }),
          num("outline.extend", "Достройка концов", { step: 0.05, min: 0, max: 2 }),
          num("outline.smooth", "Сглаживание", { step: 0.005, min: 0, max: 0.2 }),
          num("outline.simplify", "Упрощение", { step: 0.001, min: 0, max: 0.03 }),
          num("outline.junction_merge", "Склейка близких развилок", { step: 0.1, min: 0, max: 6 }),
          num("outline.px_per_em", "Разрешение растеризации", { unit: "px/em", step: 50, min: 400, max: 3000 }),
        ],
      },
    ],
  },
];

const TABS = [
  {
    id: "main",
    title: "Основные",
    groups: [
      {
        title: "Программа",
        fields: [
          { type: "theme", label: "Тема" },
          { type: "lang", label: "Язык", hint: "Интерфейс переключится сразу после выбора." },
        ],
      },
      {
        title: "Вид листа",
        mode: "drawing",
        fields: [bool("drawing.show_travel", "Показывать холостые ходы", { redraw: true })],
      },
    ],
  },
  {
    id: "print",
    title: "Настройки печати",
    groups: [
      {
        title: "Карандаш",
        fields: [
          num("printer.pen_up_z", "Высота подъёма (Z)", { unit: "мм", step: 0.1 }),
          num("printer.pen_down_z", "Высота касания (Z)", { unit: "мм", step: 0.1 }),
          num("printer.end_lift", "Подъём в конце печати", { unit: "мм", step: 1, min: 0, max: 100,
            hint: "В конце файла карандаш поднимается на столько выше обычного и остаётся на месте." }),
        ],
      },
      {
        title: "Скорости",
        fields: [
          num("printer.feed_draw", "Рисование", { unit: "мм/мин", step: 50 }),
          num("printer.feed_travel", "Перемещение", { unit: "мм/мин", step: 50 }),
          num("printer.feed_z", "Подъём и опускание", { unit: "мм/мин", step: 50 }),
          num("printer.simplify_tol", "Упрощение пути", { unit: "мм", step: 0.01, min: 0 }),
        ],
      },
      {
        title: "Рамка",
        mode: "drawing",
        fields: [
          bool("drawing.frame.enabled", "Рамка с полями"),
          num("drawing.frame.left", "Слева", { unit: "мм", step: 0.5, min: 0, show: (S) => S.drawing.frame.enabled }),
          num("drawing.frame.right", "Справа", { unit: "мм", step: 0.5, min: 0, show: (S) => S.drawing.frame.enabled }),
          num("drawing.frame.top", "Сверху", { unit: "мм", step: 0.5, min: 0, show: (S) => S.drawing.frame.enabled }),
          num("drawing.frame.bottom", "Снизу", { unit: "мм", step: 0.5, min: 0, show: (S) => S.drawing.frame.enabled }),
          bool("drawing.frame.title_block", "Основная надпись (только линии)", { show: (S) => S.drawing.frame.enabled }),
          num("drawing.frame.tb_width", "Ширина надписи", { unit: "мм", step: 1, min: 10,
            show: (S) => S.drawing.frame.enabled && S.drawing.frame.title_block }),
          num("drawing.frame.tb_height", "Высота надписи", { unit: "мм", step: 1, min: 5,
            show: (S) => S.drawing.frame.enabled && S.drawing.frame.title_block }),
        ],
      },
      {
        title: "Сдвиг чертежа",
        mode: "drawing",
        fields: [
          num("drawing.placement.dx", "Сдвиг по X", { unit: "мм", step: 0.5 }),
          num("drawing.placement.dy", "Сдвиг по Y", { unit: "мм", step: 0.5 }),
          num("drawing.placement.margin", "Поле при вписывании", { unit: "мм", step: 1, min: 0 }),
        ],
      },
      {
        title: "Толщина линий",
        mode: "drawing",
        fields: [
          bool("drawing.weights.enabled", "Толстые линии несколькими проходами"),
          num("drawing.weights.threshold", "Толстая, если толще", { unit: "мм", step: 0.05, min: 0, show: (S) => S.drawing.weights.enabled }),
          num("drawing.weights.passes", "Проходов", { step: 1, min: 1, max: 9, int: true, show: (S) => S.drawing.weights.enabled }),
          num("drawing.weights.step", "Шаг между проходами", { unit: "мм", step: 0.05, min: 0.01, show: (S) => S.drawing.weights.enabled }),
        ],
      },
      {
        title: "Проходы",
        mode: "drawing",
        fields: [
          sel("drawing.split.areas", "Рабочих областей", [[0, "авто (сколько нужно)"], [1, "1"], [2, "2 (лист 0° и 180°)"], [3, "3"], [4, "4 (все углы в упоры)"]], { int: true }),
          num("drawing.split.overlap", "Нахлёст у шва", { unit: "мм", step: 0.1, min: 0 }),
          num("drawing.split.slack", "Запас под сдвиг нуля", { unit: "мм", step: 0.5, min: 0 }),
          bool("drawing.split.marks", "Контрольные крестики вдоль швов"),
          num("drawing.split.mark_size", "Размах крестика", { unit: "мм", step: 0.5, min: 1, show: (S) => S.drawing.split.marks }),
          num("drawing.split.mark_count", "Крестиков на шов", { step: 1, min: 1, max: 10, int: true, show: (S) => S.drawing.split.marks }),
          bool("drawing.test_files", "Класть в архив тестовые файлы проходов"),
        ],
      },
    ],
  },
  {
    id: "printer",
    title: "Настройки принтера",
    groups: [
      {
        title: "Стол и доступная область",
        fields: [
          { type: "calib", label: "Калибровка: где достаёт карандаш…", hint: "Пошагово: ноль в углу листа и края хода в четыре стороны. Можно по кабелю или с SD-карты." },
          { type: "travel", label: "Доступная область измерена" },
          num("printer.travel.x_min", "X от", { unit: "мм", step: 0.5, show: travelOn }),
          num("printer.travel.x_max", "X до", { unit: "мм", step: 0.5, show: travelOn }),
          num("printer.travel.y_min", "Y от", { unit: "мм", step: 0.5, show: travelOn }),
          num("printer.travel.y_max", "Y до", { unit: "мм", step: 0.5, show: travelOn }),
          num("printer.work_w", "Рабочая зона по X", { unit: "мм", step: 1, min: 1, show: (S) => !travelOn(S) }),
          num("printer.work_h", "Рабочая зона по Y", { unit: "мм", step: 1, min: 1, show: (S) => !travelOn(S) }),
          num("printer.table.table_x", "Стол от упора по +X", { unit: "мм", step: 1, min: 0, nullable: true, placeholder: "авто" }),
          num("printer.table.table_y", "Стол от упора по +Y", { unit: "мм", step: 1, min: 0, nullable: true, placeholder: "авто" }),
          bool("printer.table.overhang_x", "Лист может выступать за стол в сторону +X"),
          bool("printer.table.overhang_y", "Лист может выступать за стол в сторону +Y"),
        ],
      },
      {
        title: "Оси и запас",
        fields: [
          bool("printer.flip_x", "Ось X принтера смотрит влево по листу"),
          bool("printer.flip_y", "Ось Y принтера смотрит вниз по листу"),
          num("printer.safety_margin", "Запас до границ", { unit: "мм", step: 0.5, min: 0 }),
          num("printer.test_mark_offset", "Стрелки тестового файла от нуля", { unit: "мм", step: 0.5, min: 0 }),
        ],
      },
    ],
  },
  ...NOTEBOOK_TABS,
  {
    id: "extra",
    mode: "drawing",
    title: "Дополнительные",
    groups: [
      {
        title: "Картинки (PNG, JPG)",
        fields: [
          bool("drawing.imp.threshold_auto", "Порог автоматически"),
          num("drawing.imp.threshold", "Порог (темнее — линия)", { step: 1, min: 1, max: 254, int: true, disabled: (S) => S.drawing.imp.threshold_auto }),
          num("drawing.imp.raster_dpi", "DPI (0 — из файла)", { step: 1, min: 0 }),
          bool("drawing.imp.invert", "Светлые линии на тёмном фоне"),
        ],
      },
      {
        title: "Заливки в чертежах",
        fields: [
          bool("drawing.imp.fill_centerlines", "Мелкие закрашенные фигуры → средние линии"),
          num("drawing.imp.fill_centerline_max", "…если меньшая сторона не больше", { unit: "мм", step: 0.5, min: 0.1,
            show: (S) => S.drawing.imp.fill_centerlines }),
        ],
      },
      {
        title: "Линии чертежа",
        fields: [
          num("drawing.paths.curve_tol", "Точность кривых", { unit: "мм", step: 0.01, min: 0.005 }),
          num("drawing.paths.join_tol", "Склейка концов", { unit: "мм", step: 0.01, min: 0 }),
          num("drawing.paths.long_path", "Длинные пути от", { unit: "мм", step: 5, min: 0 }),
        ],
      },
      {
        title: "A3 на столе A4 — окно",
        fields: [
          num("drawing.a3.x_min", "X от", { unit: "мм", step: 0.5 }),
          num("drawing.a3.x_max", "X до", { unit: "мм", step: 0.5 }),
          num("drawing.a3.y_min", "Y от", { unit: "мм", step: 0.5 }),
          num("drawing.a3.y_max", "Y до", { unit: "мм", step: 0.5 }),
        ],
      },
      {
        title: "Лист по меткам",
        fields: [
          num("drawing.marked.x_min", "X от", { unit: "мм", step: 0.5 }),
          num("drawing.marked.x_max", "X до", { unit: "мм", step: 0.5 }),
          num("drawing.marked.y_min", "Y от", { unit: "мм", step: 0.5 }),
          num("drawing.marked.y_max", "Y до", { unit: "мм", step: 0.5 }),
          num("drawing.marked.length", "Длина листа", { unit: "мм", step: 1, min: 1 }),
        ],
      },
    ],
  },
];

function fieldValue(f, S) {
  if (f.get) return f.get(S);
  return getPath(S, f.path);
}

function buildForm(container, groups, hooks) {
  const items = [];
  container.innerHTML = "";
  for (const g of groups) {
    const box = document.createElement("section");
    box.className = "fgroup";
    if (g.title) {
      const h = document.createElement("h3");
      h.textContent = g.title;
      box.appendChild(h);
    }
    for (const f of g.fields) {
      const row = document.createElement("label");
      row.className = f.type === "bool" || f.type === "travel" ? "frow check" : "frow";
      let input;
      if (f.type === "info") {
        row.className = "frow info";
        input = document.createElement("span");
        row.appendChild(input);
      } else if (f.type === "seed" || f.type === "fontUpload" || f.type === "testFile" || f.type === "calib" || f.type === "align") {
        row.className = "frow action";
        input = document.createElement("button");
        input.type = "button";
        input.className = f.type === "calib" ? "btn primary" : "btn small";
        input.textContent = f.label;
        row.appendChild(input);
      } else if (f.type === "bool" || f.type === "travel") {
        input = document.createElement("input");
        input.type = "checkbox";
        row.append(input, document.createTextNode(" " + f.label));
      } else {
        const span = document.createElement("span");
        span.textContent = f.label;
        const ctl = document.createElement("span");
        ctl.className = "ctl";
        if (f.type === "num") {
          input = document.createElement("input");
          input.type = "number";
          if (f.step != null) input.step = f.step;
          if (f.min != null) input.min = f.min;
          if (f.max != null) input.max = f.max;
          if (f.placeholder) input.placeholder = f.placeholder;
        } else {
          input = document.createElement("select");
          for (const [v, t] of f.options || []) {
            const o = document.createElement("option");
            o.value = v;
            o.textContent = t;
            input.appendChild(o);
          }
        }
        ctl.appendChild(input);
        if (f.type === "num") {
          const u = document.createElement("i");
          u.textContent = f.unit || "";
          ctl.appendChild(u);
        }
        row.append(span, ctl);
      }
      box.appendChild(row);
      if (f.hint) {
        const p = document.createElement("p");
        p.className = "fhint";
        p.textContent = f.hint;
        box.appendChild(p);
      }
      const item = { f, row, input, group: box, g };
      items.push(item);
      const commit = () => hooks.change(item);
      if (f.type === "info") {
      } else if (input.tagName === "BUTTON") {
        input.addEventListener("click", commit);
      } else if (f.type === "num") {
        let t = 0;
        input.addEventListener("input", () => { clearTimeout(t); t = setTimeout(commit, 450); });
        input.addEventListener("change", () => { clearTimeout(t); commit(); });
      } else {
        input.addEventListener("change", commit);
      }
    }
    container.appendChild(box);
  }
  return items;
}

function readInput(item) {
  const { f, input } = item;
  if (f.type === "bool" || f.type === "travel") return { ok: true, v: input.checked };
  if (f.type === "num") {
    if (input.value === "") return f.nullable ? { ok: true, v: null } : { ok: false };
    const v = Number(input.value);
    if (!Number.isFinite(v)) return { ok: false };
    return { ok: true, v: f.int ? Math.round(v) : v };
  }
  return { ok: true, v: f.int ? Number(input.value) : input.value };
}
