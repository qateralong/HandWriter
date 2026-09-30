from __future__ import annotations

import os
import re
import sys
import tempfile
import time
import traceback
import unicodedata
from pathlib import Path


def run(report: Path | None = None) -> int:
    from .paths import log_path
    report = report or log_path().with_name("selftest.txt")
    tmp = tempfile.mkdtemp(prefix="handwriter-selftest-")
    os.environ["HANDWRITER_HOME"] = tmp
    lines: list[str] = []
    ok_all = True

    def check(name: str, fn):
        nonlocal ok_all
        t = time.perf_counter()
        try:
            detail = fn()
            lines.append(f"[OK]   {name}" + (f" — {detail}" if detail else "") + f" ({time.perf_counter() - t:.1f} с)")
        except Exception as e:
            ok_all = False
            lines.append(f"[FAIL] {name} — {type(e).__name__}: {e}")
            lines.append("       " + traceback.format_exc().replace("\n", "\n       "))

    from . import __version__
    from .paths import FROZEN, STATIC_DIR, BUILTIN_FONTS_DIR
    from .i18n import LOCALE_DIR, lang
    lines.append(f"HandWriter {__version__} самопроверка, Python {sys.version.split()[0]}, "
                 f"{'собранный exe' if FROZEN else 'из исходников'}, язык интерфейса: {lang()}, {sys.executable}")

    from .pipeline import GenerationRefused, compose, make_gcode, make_test_gcode
    from .settings import MissingChoice, Settings, Travel

    def base(font="builtin:hershey_cyrillic.svg", mode="strokes", text="\tПроба пера: электрификация.\n\nВторой абзац."):
        s = Settings()
        s.font, s.mode, s.text = font, mode, text
        s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
        return s

    def outline(text="\tСъешь же ещё этих мягких французских булок, да выпей чаю.\n\n\tМама мыла раму."):
        return base("builtin:BadScript-Regular.ttf", "outlines", text)

    def data_files():
        need = [STATIC_DIR / "index.html", STATIC_DIR / "app.js", STATIC_DIR / "debug.html",
                STATIC_DIR / "keepalive.js", STATIC_DIR / "icon.png",
                STATIC_DIR / "drawing.html", STATIC_DIR / "drawing.js",
                BUILTIN_FONTS_DIR / "hershey_cyrillic.svg", BUILTIN_FONTS_DIR / "BadScript-Regular.ttf",
                LOCALE_DIR / "en.json"]
        missing = [str(p) for p in need if not p.exists()]
        assert not missing, "нет файлов: " + ", ".join(missing)
        return f"{len(need)} файлов на месте"
    check("Интерфейс и встроенные шрифты в сборке", data_files)

    def pyphen_ok():
        from .text import Word, break_positions
        w = Word(index=1, text="переносы", paragraph=1, text_line=1)
        assert set(break_positions(w, True)) == {2, 4, 6}
        return "ru_RU: пе-ре-но-сы"
    check("pyphen, словарь переносов ru_RU", pyphen_ok)

    def harfbuzz_ok():
        from .glyphs import load_provider
        p = load_provider("builtin:BadScript-Regular.ttf", "outlines")
        sh = p.shape("ffi")
        assert [(g.name, g.cluster) for g in sh] == [("f_f_i", 0)], sh
        return "лигатура ffi → f_f_i"
    check("uharfbuzz, шейпинг с лигатурами", harfbuzz_ok)

    def skeleton_ok():
        from .glyphs import load_provider
        p = load_provider("builtin:BadScript-Regular.ttf", "outlines")
        g = p.glyph(p.glyph_names_for_char("в")[0])
        assert g.strokes and sum(len(s) for s in g.strokes) > 10
        return f"«в»: {len(g.strokes)} штрих(а)"
    check("scikit-image и scipy, скелет глифа", skeleton_ok)

    def bounds_refuse():
        s = base()
        s.printer.travel = Travel(x_min=-2, x_max=60, y_min=-2, y_max=230)
        c = compose(s)
        assert any("вне хода" in e for e in c.errors), c.errors
        try:
            make_gcode(c)
        except GenerationRefused:
            return "генерация отказывается"
        raise AssertionError("gcode создан, хотя точки вне хода")
    check("Проверки: точка вне хода карандаша → отказ", bounds_refuse)

    def z_checks():
        s = base()
        s.printer.pen_down_z, s.printer.pen_up_z = -3.5, 1.5
        c = compose(s)
        assert any("pen_down_z" in w for w in c.warnings) and any("pen_up_z" in w for w in c.warnings)
        s.printer.pen_up_z = -4
        assert any("pen_up_z должен быть выше" in e for e in compose(s).errors)
        return "предупреждения и ошибка на месте"
    check("Проверки: значения Z", z_checks)

    def glyph_checks():
        s = base(text="Номер №5")
        c = compose(s)
        assert any("Нет в шрифте" in e for e in c.errors)
        s.text_options.missing["№"] = MissingChoice(action="skip")
        assert compose(s).errors == []
        return "без решения по «№» gcode не создаётся"
    check("Проверки: глифы для всех символов", glyph_checks)

    def sheet_checks():
        s = base()
        s.sheet.margin_left = s.sheet.margin_right = s.sheet.width
        assert any("поля" in e for e in compose(s).errors)
        return "несогласованные поля → ошибка"
    check("Проверки: поля и размеры листа", sheet_checks)

    def gcode_format():
        code = make_gcode(compose(outline()))
        body = [ln for ln in code.splitlines() if ln and not ln.startswith(";")]
        bad = [ln for ln in body if not re.match(r"^(G0|G1|G21|G90|G92 X0 Y0 Z0|M104 S0|M140 S0|M420 S0|M211 S0|M400)\b", ln)]
        assert not bad, bad[:3]
        assert "G28" not in code and "M109" not in code and "M190" not in code
        assert body[-3:][1] == "G0 X0.00 Y0.00 F3000" and body[-1] == "M400"
        test, _ = make_test_gcode(base())
        assert "G92 X0 Y0 Z0" in test
        return f"{len(body)} команд, только G0/G1, без нагрева и G28"
    check("Gcode: формат и тестовый файл", gcode_format)

    def determinism():
        from . import glyphs
        a = make_gcode(compose(outline()))
        glyphs._cache.clear()
        b = make_gcode(compose(outline()))
        assert a == b, "gcode отличается"
        s = outline()
        s.randomness.seed = 2
        assert make_gcode(compose(s)) != a
        return f"{len(a.encode())} байт, совпадает побайтно; другой seed даёт другой файл"
    check("Почерк: тот же seed → тот же gcode побайтно", determinism)

    def resume_tail():
        xy = lambda code: re.findall(r"X(-?\d+\.\d\d) Y(-?\d+\.\d\d)", code)[:-1]
        s = outline()
        s.connections.distance = 0.35
        full = make_gcode(compose(s))
        c = compose(s)
        target = None
        for st in c.layout.strokes:
            for k in range(1, len(st.tags)):
                if st.tags[k] != st.tags[k - 1]:
                    g = c.layout.glyphs[st.tags[k]]
                    target = (g.word, g.letter)
                    break
            if target:
                break
        results = []
        for w, l in ([target] if target else []) + [(6, 3)]:
            s2 = s.model_copy(deep=True)
            s2.text_options.resume_word, s2.text_options.resume_letter = w, l
            part = make_gcode(compose(s2))
            f, p = xy(full), xy(part)
            assert 0 < len(p) < len(f) and f[-len(p):] == p, f"слово {w}, буква {l}"
            results.append(f"слово {w} буква {l}")
        return "совпадает с хвостом: " + ", ".join(results) + (" (первое — от точки связки)" if target else "")
    check("Почерк: «продолжить с буквы» = хвост полного gcode", resume_tail)

    def nfc():
        from .text import normalize, process_text
        from .glyphs import load_provider
        p = load_provider("builtin:hershey_cyrillic.svg", "strokes")
        pt = process_text("йод ёж", p.has_char)
        assert [w.text for w in pt.words] == ["йод", "ёж"] and not pt.missing
        assert normalize("й") == unicodedata.normalize("NFC", "й")
        return "й и ё из двух кодов находятся в шрифте"
    check("Почерк: NFC-нормализация", nfc)

    def hyph():
        from .text import Word, break_positions
        for short in ("кот", "да", "ёж", "мир!"):
            assert break_positions(Word(1, short, 1, 1), True) == {}, short
        for long_ in ("электрификация", "подъёму", "переносы"):
            for b in break_positions(Word(1, long_, 1, 1), True):
                assert b >= 2 and len(long_) - b >= 2
        return "в коротких словах переносов нет, минимум 2 буквы с каждой стороны"
    check("Почерк: переносы", hyph)

    def mean_size():
        s = outline("х" * 12 + " " + "х" * 12)
        s.randomness.size = 10.0
        c = compose(s)
        f = [g.size for g in c.layout.glyphs]
        assert abs(sum(f) / len(f) - 1.0) < 1e-12 and max(f) - min(f) > 0.05
        return f"средний множитель {sum(f) / len(f):.12f} при разбросе ±10%"
    check("Почерк: средний размер букв равен заданному", mean_size)

    def drawing_formats():
        import io
        import ezdxf
        import pymupdf
        from PIL import Image, ImageDraw
        from .drawing.sources import clear_cache
        from .drawing.pipeline import compose_drawing
        from .paths import user_drawings_dir
        d = user_drawings_dir()
        doc = ezdxf.new("R2010", setup=True, units=4)
        msp = doc.modelspace()
        msp.add_lwpolyline([(0, 0), (100, 0), (100, 60), (0, 60)], close=True)
        msp.add_circle((50, 30), 20)
        msp.add_line((0, 30), (100, 30), dxfattribs={"linetype": "CENTER"})
        buf = io.StringIO()
        doc.write(buf)
        (d / "st.dxf").write_text(buf.getvalue(), encoding="utf-8")
        pdf = pymupdf.open()
        page = pdf.new_page(width=300, height=200)
        sh = page.new_shape()
        sh.draw_rect(pymupdf.Rect(20, 20, 280, 180))
        sh.finish(width=1, color=(0, 0, 0))
        sh.commit()
        (d / "st.pdf").write_bytes(pdf.tobytes())
        img = Image.new("L", (300, 200), 255)
        ImageDraw.Draw(img).line((20, 100, 280, 100), fill=0, width=7)
        png = io.BytesIO()
        img.save(png, format="PNG")
        (d / "st.png").write_bytes(png.getvalue())
        out = []
        for name in ("builtin:test", "st.dxf", "st.pdf", "st.png"):
            clear_cache()
            s = Settings()
            s.drawing.file = name
            c = compose_drawing(s)
            assert not c.errors and c.imp.paths, (name, c.errors)
            out.append(f"{c.imp.kind} {len(c.imp.paths)}")
        return "SVG, DXF (ezdxf), PDF (PyMuPDF), PNG (Pillow + скелет): " + ", ".join(out)
    check("Чертёж: форматы файлов", drawing_formats)

    def drawing_passes():
        from .drawing.passes import ROTATIONS, from_pass, to_pass
        from .drawing.pipeline import compose_drawing, make_all_files
        for r in ROTATIONS:
            for p in [(0, 0), (12.5, 200), (297, 210)]:
                assert from_pass(to_pass(p, r, 297, 210), r, 297, 210) == p
        s = Settings()
        s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215)
        s.drawing.weights.enabled = True
        s.drawing.split.marks = True
        c = compose_drawing(s)
        assert not c.errors and [p.rotation for p in c.parts] == [0, 180], c.errors
        st = c.cut_stats
        assert abs(st.extension - 2 * s.drawing.split.overlap * st.cuts) < 1e-6
        a = make_all_files(c, tests=True)
        b = make_all_files(compose_drawing(s), tests=True)
        assert [f["gcode"] for f in a] == [f["gcode"] for f in b]
        for f in a:
            assert "G92 X0 Y0 Z0" in f["gcode"] and "G28" not in f["gcode"] and "M109" not in f["gcode"]
        return (f"A4 альбом: проходы 0° и 180°, шов x = {c.split.seams()[0].s:.1f}, разрезов {st.cuts}, "
                f"{len(a)} файла, побайтно повторяются")
    check("Чертёж: разбиение на проходы и файлы", drawing_passes)

    def calibration():
        from .calibration import make_reach_check_gcode
        s = Settings()
        s.printer.travel = Travel(x_min=-3, x_max=220, y_min=-3, y_max=219)
        g = make_reach_check_gcode(s)
        assert g.count("G4 P1000") == 4 and "G92 X0 Y0 Z0" in g and "G28" not in g
        return "проверочный файл: 4 угла окна, касание и паузы"
    check("Чертёж: калибровка окна достижимости", calibration)

    def settings_io():
        from .settings import load_settings, save_settings
        from .paths import settings_path
        s = Settings()
        s.text = "проверка"
        save_settings(s)
        assert load_settings().text == "проверка" and settings_path().exists()
        return str(settings_path().parent)
    check("Настройки пишутся в папку пользователя", settings_io)

    lines.append("ИТОГ: " + ("всё в порядке" if ok_all else "ЕСТЬ ОШИБКИ"))
    from .i18n import tr
    text = "\n".join(tr(line) for line in lines) + "\n"
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(text, encoding="utf-8")
    try:
        print(text)
    except Exception:
        pass
    return 0 if ok_all else 1
