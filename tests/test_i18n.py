import base64
import io
import re

import ezdxf
import pytest
from fastapi.testclient import TestClient

from handwriter import i18n
from handwriter.drawing.sources import clear_cache
from handwriter.server import app
from handwriter.settings import Settings, Travel

CYR = re.compile(r"[Ѐ-ӿ]")
client = TestClient(app)


@pytest.fixture
def english():
    i18n.set_lang("en")
    clear_cache()
    yield
    i18n.set_lang("ru")
    clear_cache()


def cyrillic_messages(obj, key=None, found=None):
    found = [] if found is None else found
    if isinstance(obj, dict):
        for k, v in obj.items():
            cyrillic_messages(v, k, found)
    elif isinstance(obj, list):
        for v in obj:
            cyrillic_messages(v, key, found)
    elif isinstance(obj, str) and key in i18n.TR_KEYS and CYR.search(obj):
        found.append(obj)
    return found


def body(s: Settings) -> dict:
    return s.model_dump(mode="json")


@pytest.mark.parametrize("url", ["/", "/drawing", "/debug", "/static/app.js", "/static/drawing.js",
                                 "/static/outline_params.js", "/static/keepalive.js"])
def test_english_pages_have_no_russian(english, url):
    r = client.get(url)
    assert r.status_code == 200
    assert not CYR.search(r.text), CYR.search(r.text)
    if url in ("/", "/drawing", "/debug"):
        assert '<html lang="en">' in r.text


def test_russian_pages_unchanged():
    assert "Почерк" in client.get("/").text and "Чертёж" in client.get("/drawing").text
    assert "Путь карандаша" in client.get("/static/app.js").text


def test_english_defaults(english):
    s = Settings()
    assert not CYR.search(s.text) and s.active_profile == "Grid notebook"
    assert set(s.profiles) == {"Grid notebook", "Ruled notebook", "A4"}
    p = client.post("/api/preview", json=body(s)).json()
    assert p["errors"] == [] and p["strokes"]
    assert not cyrillic_messages(p)
    assert all(not CYR.search(f["label"]) for f in client.get("/api/fonts").json())


def test_english_messages_handwriting(english):
    s = Settings()
    s.text = "Hello №1"
    s.printer.travel = Travel(x_min=0, x_max=40, y_min=0, y_max=40)
    s.printer.pen_down_z = -4
    r = client.post("/api/gcode", json=body(s))
    assert r.status_code == 422
    errs = r.json()["errors"]
    assert errs and not cyrillic_messages(r.json())
    assert any("Missing from the font" in e for e in errs)
    bad = body(s)
    bad["sheet"]["width"] = "wide"
    r = client.post("/api/preview", json=bad)
    assert r.status_code == 422 and r.json()["detail"].startswith("Invalid settings. Sheet")
    assert client.get("/api/nope").json()["detail"].startswith("Not found")


def test_english_messages_drawing(english):
    s = Settings()
    s.printer.travel = Travel(x_min=-3, x_max=160, y_min=-3, y_max=120)
    s.drawing.frame.enabled = True
    s.drawing.split.marks = True
    p = client.post("/api/drawing/preview", json=body(s)).json()
    assert p["errors"] and not cyrillic_messages(p)
    assert all(not CYR.search(n) for n in p["passes"]["notes"])
    s.drawing.frame.enabled = False
    s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215)
    p = client.post("/api/drawing/preview", json=body(s)).json()
    assert p["errors"] == [] and len(p["parts"]) == 2
    assert [q["corner_name"] for q in p["parts"]] == ["bottom left", "top right"]
    assert not cyrillic_messages(p) and not CYR.search(p["import"]["name"])
    doc = ezdxf.new(units=0)
    doc.modelspace().add_line((0, 0), (80, 40))
    doc.modelspace().add_text("Деталь", dxfattribs={"insert": (5, 5), "height": 3})
    buf = io.StringIO()
    doc.write(buf)
    client.post("/api/drawing/upload", json={"filename": "part.dxf",
                                             "content_b64": base64.b64encode(buf.getvalue().encode()).decode()})
    s.drawing.file = "part.dxf"
    p = client.post("/api/drawing/preview", json=body(s)).json()
    leftovers = [m for m in cyrillic_messages(p) if "Деталь" not in m]
    assert not leftovers, leftovers
    assert any("Text not converted to curves" in w for w in p["warnings"])
    c = client.post("/api/calibration/info", json=body(s)).json()
    assert c["warnings"] and not cyrillic_messages(c)
    assert all(not CYR.search(f["label"]) for f in client.get("/api/drawing/files").json())


def test_translation_keeps_numbers_and_user_text(english):
    assert i18n.tr("Проход 2: поправка dx 0.5, dy 0 выводит линии за окно достижимости — уменьши поправку или масштаб") \
        == "Pass 2: correction dx 0.5, dy 0 moves lines outside the reach window — reduce the correction or the scale"
    assert i18n.tr("Нет в шрифте: «№». Выбери для них «пропустить» или «заменить»") \
        == "Missing from the font: «№». Choose “skip” or “replace” for them"
    assert i18n.tr("Листов") == "Листов"
