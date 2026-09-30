from __future__ import annotations

import base64
import logging
import re
import secrets
import time
from functools import lru_cache
from pathlib import Path

from fastapi import FastAPI, HTTPException, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel
from starlette.exceptions import HTTPException as StarletteHTTPException

from . import __version__
from .glyphs import load_provider, mode_for_path
from .i18n import lang, tr_json, tr_static
from .paths import BUILTIN_FONTS_DIR, BUILTIN_PREFIX, STATIC_DIR, log_path, resolve_font_path, user_fonts_dir
from .pipeline import GenerationRefused, compose, make_gcode, make_test_gcode, preview_payload
from .settings import Settings, default_profiles, load_settings, save_settings

log = logging.getLogger("handwriter.server")


class TrJSONResponse(JSONResponse):
    def render(self, content) -> bytes:
        return super().render(tr_json(content))


app = FastAPI(title="HandWriter", default_response_class=TrJSONResponse)


class Activity:
    started = time.monotonic()
    last_ping: float | None = None
    token = secrets.token_hex(8)

    @classmethod
    def ping(cls):
        cls.last_ping = time.monotonic()


_FIELD_NAMES = {
    "sheet": "Лист", "typography": "Размер и подгонка", "printer": "Принтер", "randomness": "Случайность",
    "connections": "Слитное письмо", "outline": "Контуры → линии", "text_options": "Текст",
    "drawing": "Чертёж",
}


@app.exception_handler(RequestValidationError)
async def _validation_error(request: Request, exc: RequestValidationError):
    msgs = []
    for e in exc.errors()[:5]:
        loc = [str(x) for x in e.get("loc", []) if x != "body"]
        where = " → ".join([_FIELD_NAMES.get(loc[0], loc[0])] + loc[1:]) if loc else "запрос"
        kind = e.get("type", "")
        if "greater_than" in kind or "less_than" in kind:
            what = "значение вне допустимых пределов"
        elif "float" in kind or "int" in kind or "number" in kind:
            what = "нужно число"
        elif kind == "missing":
            what = "не заполнено"
        else:
            what = "неверное значение"
        msgs.append(f"{where}: {what}")
    log.warning("Неверные данные %s: %s", request.url.path, exc.errors()[:5])
    return TrJSONResponse(status_code=422, content={"detail": "Неверные настройки. " + "; ".join(msgs),
                                                  "errors": ["Неверные настройки. " + "; ".join(msgs)]})


@app.exception_handler(StarletteHTTPException)
async def _http_error(request: Request, exc: StarletteHTTPException):
    detail = exc.detail if isinstance(exc.detail, str) else "Ошибка запроса"
    if exc.status_code == 404:
        detail = "Не найдено: " + request.url.path
    return TrJSONResponse(status_code=exc.status_code, content={"detail": detail})


@app.exception_handler(Exception)
async def _unexpected_error(request: Request, exc: Exception):
    log.exception("Ошибка при обработке %s", request.url.path)
    msg = (f"Внутренняя ошибка программы ({type(exc).__name__}: {exc}). "
           f"Подробности записаны в журнал: {log_path()}")
    return TrJSONResponse(status_code=500, content={"detail": msg, "errors": [msg]})


@app.api_route("/api/ping", methods=["GET", "POST"])
def ping():
    Activity.ping()
    return {"ok": True}


@app.get("/api/instance")
def instance():
    return {"app": "HandWriter", "token": Activity.token, "version": __version__}


@app.get("/")
def index():
    return _page("index.html")


_NO_STORE = {"Cache-Control": "no-store"}
_TYPES = {".js": "text/javascript; charset=utf-8", ".html": "text/html; charset=utf-8"}


@lru_cache(maxsize=32)
def _translated(path: Path, mtime: int, code: str) -> str:
    return tr_static(path.read_text(encoding="utf-8"))


def _text(path: Path) -> str:
    return _translated(path, path.stat().st_mtime_ns, lang())


def _page(name: str) -> HTMLResponse:
    return HTMLResponse(_text(STATIC_DIR / name), headers=_NO_STORE)


@app.get("/static/{name}")
def static_file(name: str):
    path = STATIC_DIR / name
    if not path.is_file() or path.parent != STATIC_DIR:
        raise HTTPException(404)
    if path.suffix in _TYPES and lang() != "ru":
        return Response(_text(path), media_type=_TYPES[path.suffix], headers=_NO_STORE)
    return FileResponse(path)


app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")


@app.get("/api/settings")
def get_settings():
    return load_settings().model_dump(mode="json")


@app.put("/api/settings")
def put_settings(s: Settings):
    save_settings(s)
    return {"ok": True}


@app.get("/api/defaults")
def get_defaults():
    return {"settings": Settings().model_dump(mode="json"),
            "profiles": {k: v.model_dump(mode="json") for k, v in default_profiles().items()}}


@app.post("/api/preview")
def preview(s: Settings):
    return preview_payload(compose(s))


@app.post("/api/gcode")
def gcode(s: Settings):
    c = compose(s)
    try:
        code = make_gcode(c)
    except GenerationRefused as e:
        return TrJSONResponse(status_code=422, content={"errors": e.errors})
    lay = c.layout
    name = f"handwriter_w{lay.first_word}-{lay.last_word}"
    if c.resume:
        name += f"_from_w{c.resume.word}_l{c.resume.letter}"
    name += ".gcode"
    return {"gcode": code, "filename": name, "warnings": c.warnings}


@app.post("/api/testfile")
def testfile(s: Settings):
    try:
        code, _ = make_test_gcode(s)
    except GenerationRefused as e:
        return TrJSONResponse(status_code=422, content={"errors": e.errors})
    return {"gcode": code, "filename": "handwriter_test.gcode"}


@app.post("/api/testfile/preview")
def testfile_preview(s: Settings):
    from .gcode import test_pattern
    return {"strokes": [[[round(x, 3), round(y, 3)] for x, y in st] for st in test_pattern(s)]}


FONT_EXT = (".svg", ".ttf", ".otf")


@app.get("/api/fonts")
def list_fonts():
    fonts = [{"spec": BUILTIN_PREFIX + p.name, "label": f"Встроенный: {p.stem}", "mode": mode_for_path(p)}
             for p in sorted(BUILTIN_FONTS_DIR.iterdir()) if p.suffix.lower() in FONT_EXT]
    for p in sorted(user_fonts_dir().iterdir()):
        if p.suffix.lower() in FONT_EXT or p.is_dir():
            fonts.append({"spec": p.name, "label": p.name + ("/" if p.is_dir() else ""),
                          "mode": mode_for_path(p)})
    return fonts


class UploadFile(BaseModel):
    filename: str
    content: str = ""
    content_b64: str | None = None


class FontUpload(BaseModel):
    name: str
    files: list[UploadFile]


_SAFE = re.compile(r'[<>:"/\\|?*\x00-\x1f]')


@app.post("/api/fonts/upload")
def upload_font(up: FontUpload):
    base = user_fonts_dir()
    binary = [f for f in up.files if f.filename.lower().endswith((".ttf", ".otf"))]
    if binary:
        f = binary[0]
        if not f.content_b64:
            raise HTTPException(400, "TTF/OTF нужно передавать в content_b64")
        target = base / _SAFE.sub("_", Path(f.filename).name)
        target.write_bytes(base64.b64decode(f.content_b64))
        return _font_reply(target.name)
    svgs = [f for f in up.files if f.filename.lower().endswith((".svg", ".json"))]
    if not svgs:
        raise HTTPException(400, "Нужен файл .ttf, .otf или .svg")
    if len(svgs) == 1 and svgs[0].filename.lower().endswith(".svg"):
        target = base / _SAFE.sub("_", Path(svgs[0].filename).name)
        target.write_text(svgs[0].content, encoding="utf-8")
        spec = target.name
    else:
        folder = base / _SAFE.sub("_", up.name or "font")
        folder.mkdir(exist_ok=True)
        for f in svgs:
            name = Path(f.filename).name
            stem, ext = name.rsplit(".", 1)
            if _SAFE.search(stem):
                stem = "".join(f"uni{ord(ch):04X}" if _SAFE.match(ch) else ch for ch in stem)
            (folder / f"{stem}.{ext}").write_text(f.content, encoding="utf-8")
        spec = folder.name
    return _font_reply(spec)


def _font_reply(spec: str) -> dict:
    mode = mode_for_path(resolve_font_path(spec))
    try:
        prov = load_provider(spec, mode)
    except Exception as e:
        raise HTTPException(400, f"Не удалось прочитать шрифт: {e}") from e
    info = prov.info()
    return {"spec": spec, "mode": mode, "name": info.name, "glyph_count": info.glyph_count, "notes": info.notes}


@app.get("/drawing")
def drawing_page():
    return _page("drawing.html")


@app.get("/api/drawing/files")
def drawing_files():
    from .drawing.sources import list_files
    return list_files()


class DrawingUpload(BaseModel):
    filename: str
    content_b64: str


@app.post("/api/drawing/upload")
def drawing_upload(up: DrawingUpload):
    from .drawing.sources import EXTENSIONS, file_kind
    from .paths import user_drawings_dir
    name = _SAFE.sub("_", Path(up.filename).name)
    if not file_kind(name):
        raise HTTPException(400, "Нужен файл " + ", ".join(sorted(EXTENSIONS)))
    try:
        data = base64.b64decode(up.content_b64, validate=True)
    except ValueError:
        raise HTTPException(400, "Файл повреждён при передаче") from None
    (user_drawings_dir() / name).write_bytes(data)
    return {"spec": name, "kind": file_kind(name)}


@app.post("/api/drawing/preview")
def drawing_preview(s: Settings):
    from .drawing.pipeline import compose_drawing, preview_payload
    return preview_payload(compose_drawing(s))


@app.post("/api/drawing/gcode")
def drawing_gcode(s: Settings, part: int = 0, test: bool = False, all_tests: bool = False):
    from .drawing.pipeline import compose_drawing, make_all_files, make_part_gcode, make_part_test_gcode, part_filename
    c = compose_drawing(s)
    try:
        if part:
            if not 1 <= part <= len(c.parts):
                raise HTTPException(400, f"Нет прохода {part}")
            p = c.parts[part - 1]
            if c.errors:
                raise GenerationRefused(c.errors)
            code = make_part_test_gcode(c, p) if test else make_part_gcode(c, p)
            files = [{"filename": part_filename(c, p, test), "gcode": code, "pass": p.index,
                      "rotation": p.rotation, "test": test}]
        elif test:
            if c.errors:
                raise GenerationRefused(c.errors)
            files = [{"filename": part_filename(c, p, True), "gcode": make_part_test_gcode(c, p), "pass": p.index,
                      "rotation": p.rotation, "test": True} for p in c.parts]
        else:
            files = make_all_files(c, all_tests)
    except GenerationRefused as e:
        return TrJSONResponse(status_code=422, content={"errors": e.errors})
    return {"files": files, "gcode": files[0]["gcode"], "filename": files[0]["filename"], "warnings": c.warnings}


@app.post("/api/calibration/info")
def calibration_info(s: Settings):
    from .calibration import check_errors, reach_corners
    errors, warnings = check_errors(s)
    corners = reach_corners(s) if s.printer.travel is not None and not errors else []
    return {"errors": errors, "warnings": warnings, "corners": [[round(x, 2), round(y, 2)] for x, y in corners]}


@app.post("/api/calibration/check_gcode")
def calibration_check_gcode(s: Settings):
    from .calibration import make_reach_check_gcode
    try:
        code = make_reach_check_gcode(s)
    except GenerationRefused as e:
        return TrJSONResponse(status_code=422, content={"errors": e.errors})
    return {"gcode": code, "filename": "handwriter_reach_check.gcode"}


@app.post("/api/calibration/zero_gcode")
def calibration_zero_gcode(s: Settings):
    from .calibration import make_zero_gcode
    try:
        code = make_zero_gcode(s)
    except GenerationRefused as e:
        return TrJSONResponse(status_code=422, content={"errors": e.errors})
    return {"gcode": code, "filename": "handwriter_zero.gcode"}


class DebugRequest(BaseModel):
    settings: Settings
    text: str = ""
    glyph: str = ""


@app.get("/debug")
def debug_page():
    return _page("debug.html")


@app.post("/api/debug/glyphs")
def debug_glyphs(req: DebugRequest):
    s = req.settings
    try:
        prov = load_provider(s.font, s.mode).with_options(s.outline)
    except Exception as e:
        raise HTTPException(400, f"Шрифт не загрузился: {e}") from e
    items = []
    if req.glyph:
        try:
            items.append((prov.debug_glyph(req.glyph), 0.0, 0.0, "", -1))
        except KeyError:
            raise HTTPException(400, f"В шрифте нет глифа «{req.glyph}»") from None
    else:
        from .text import normalize
        text = normalize(req.text)
        x = 0.0
        for sg in prov.shape(text):
            d = prov.debug_glyph(sg.name)
            items.append((d, x + sg.x_offset, sg.y_offset, text[sg.cluster], sg.cluster))
            x += sg.advance
    m = prov.metrics
    return {
        "font": prov.name, "mode": prov.mode,
        "metrics": {"x_height": m.x_height, "cap_height": m.cap_height, "ascent": m.ascent, "descent": m.descent},
        "glyphs": [dict(d, x=x, y=y, char=ch, cluster=cl) for d, x, y, ch, cl in items],
    }


@app.post("/api/debug/names")
def debug_names(s: Settings):
    try:
        prov = load_provider(s.font, s.mode)
    except Exception as e:
        raise HTTPException(400, f"Шрифт не загрузился: {e}") from e
    return {"names": prov.glyph_names()}
