import os
from pathlib import Path

from PyInstaller.utils.hooks import collect_data_files, collect_submodules

ONEFILE = os.environ.get("HANDWRITER_ONEFILE") == "1"
UI_LANG = os.environ.get("HANDWRITER_BUILD_LANG", "ru").lower()
if UI_LANG not in ("ru", "en"):
    raise SystemExit(f"Unknown interface language: {UI_LANG}")
OUT = "HandWriter" if UI_LANG == "ru" else "HandWriter-en"

lang_dir = Path("build") / f"lang-{UI_LANG}"
lang_dir.mkdir(parents=True, exist_ok=True)
(lang_dir / "lang.txt").write_text(UI_LANG, encoding="utf-8")

datas = [
    ("handwriter/static", "handwriter/static"),
    ("handwriter/fonts", "handwriter/fonts"),
    ("handwriter/locale", "handwriter/locale"),
    (str(lang_dir / "lang.txt"), "handwriter"),
]
datas += collect_data_files("pyphen")
datas += collect_data_files("skimage", includes=["**/*.pyi"])
datas += collect_data_files("ezdxf")

hiddenimports = (
    collect_submodules("uvicorn")
    + collect_submodules("fontTools.ttLib.tables")
    + collect_submodules("skimage.morphology")
    + collect_submodules("ezdxf.entities")
    + ["uharfbuzz", "scipy.ndimage", "scipy.spatial", "handwriter.selftest", "handwriter.i18n",
       "ezdxf", "ezdxf.recover", "ezdxf.path", "pymupdf", "PIL.Image", "PIL.ImageOps", "PIL.ImageDraw",
       "skimage.filters", "handwriter.drawing.pipeline", "handwriter.drawing.svg_import",
       "handwriter.drawing.dxf_import", "handwriter.drawing.pdf_import", "handwriter.drawing.raster_import",
       "handwriter.calibration"]
)

a = Analysis(
    ["app.py"],
    pathex=[],
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    runtime_hooks=[],
    excludes=["tkinter", "matplotlib", "pytest", "HersheyFonts", "IPython", "notebook"],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe_kwargs = dict(
    name=f"{OUT}-onefile" if ONEFILE else "HandWriter",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    icon="assets/HandWriter.ico",
    version="assets/version_info.txt",
)

if ONEFILE:
    exe = EXE(pyz, a.scripts, a.binaries, a.datas, [], runtime_tmpdir=None, **exe_kwargs)
else:
    exe = EXE(pyz, a.scripts, [], exclude_binaries=True, **exe_kwargs)
    coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name=OUT)
