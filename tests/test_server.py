from fastapi.testclient import TestClient

from handwriter.server import app
from handwriter.settings import Settings, Travel

client = TestClient(app)


def body(**changes):
    s = Settings()
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    d = s.model_dump(mode="json")
    d.update(changes)
    return d


def test_index_and_static():
    assert "<title>HandWriter" in client.get("/").text
    assert client.get("/static/app.js").status_code == 200


def test_settings_roundtrip_and_profiles():
    s = client.get("/api/settings").json()
    assert set(s["profiles"]) == {"Тетрадь в клетку", "Тетрадь в линейку", "A4"}
    s["typography"]["size_mm"] = 3.7
    s["profiles"]["Мой"] = {"sheet": s["sheet"], "size_mm": 2.0, "baseline_shift": 0}
    assert client.put("/api/settings", json=s).json() == {"ok": True}
    s2 = client.get("/api/settings").json()
    assert s2["typography"]["size_mm"] == 3.7 and "Мой" in s2["profiles"]


def test_preview_payload():
    r = client.post("/api/preview", json=body(text="\tПривет, мир!")).json()
    assert r["errors"] == []
    assert r["strokes"] and {"p", "w", "l", "n", "h"} <= set(r["strokes"][0])
    assert r["stats"]["strokes"] == len(r["strokes"])
    assert r["end"]["last_word"] == 2 and r["end"]["next_word"] is None


def test_gcode_refused_with_missing_char():
    r = client.post("/api/gcode", json=body(text="№1"))
    assert r.status_code == 422 and r.json()["errors"]


def test_gcode_and_testfile():
    r = client.post("/api/gcode", json=body(text="Проба"))
    assert r.status_code == 200 and r.json()["gcode"].startswith("; HandWriter")
    t = client.post("/api/testfile", json=body())
    assert t.status_code == 200 and "G92 X0 Y0 Z0" in t.json()["gcode"]


def test_upload_ttf_and_debug_page():
    import base64
    from pathlib import Path
    raw = (Path(__file__).parent / "fonts" / "MarckScript-Regular.ttf").read_bytes()
    r = client.post("/api/fonts/upload", json={"name": "Marck", "files": [
        {"filename": "MarckScript-Regular.ttf", "content_b64": base64.b64encode(raw).decode()}]})
    assert r.status_code == 200, r.text
    assert r.json()["mode"] == "outlines"
    fonts = {f["spec"]: f["mode"] for f in client.get("/api/fonts").json()}
    assert fonts["MarckScript-Regular.ttf"] == "outlines"
    assert "Отладка глифов" in client.get("/debug").text
    s = body(font="MarckScript-Regular.ttf", mode="outlines")
    d = client.post("/api/debug/glyphs", json={"settings": s, "text": "Пр"}).json()
    assert [g["char"] for g in d["glyphs"]] == ["П", "р"]
    g = d["glyphs"][1]
    assert g["outline"] and g["strokes"] and g["raw"] and g["x"] > 0
    one = client.post("/api/debug/glyphs", json={"settings": s, "glyph": "#5"}).json()
    assert len(one["glyphs"]) == 1
    assert client.post("/api/debug/glyphs", json={"settings": s, "glyph": "нет_такого"}).status_code == 400
    names = client.post("/api/debug/names", json=s).json()["names"]
    assert "uni0440" in names
    p = client.post("/api/preview", json=dict(s, text="Привет")).json()
    assert p["errors"] == [] and p["font"]["mode"] == "outlines" and p["strokes"]
    p = client.post("/api/preview", json=dict(s, mode="strokes", text="Привет")).json()
    assert any("Контуры" in e for e in p["errors"])


def test_upload_folder_font():
    svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M10 80 L10 30"/></svg>'
    r = client.post("/api/fonts/upload", json={"name": "моя рука", "files": [
        {"filename": "а.svg", "content": svg}, {"filename": "?.svg", "content": svg}]})
    assert r.status_code == 200, r.text
    assert r.json()["glyph_count"] == 2
    fonts = client.get("/api/fonts").json()
    assert any(f["spec"] == "моя рука" for f in fonts)
