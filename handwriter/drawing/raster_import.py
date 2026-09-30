from __future__ import annotations

import io

import numpy as np

from .fills import mask_to_paths
from .model import DPath, ImportResult

MAX_SIDE = 3000
DEFAULT_DPI = 300.0


def import_raster(data: bytes, name: str, opts) -> ImportResult:
    from PIL import Image, ImageOps
    from skimage.filters import threshold_otsu
    from skimage.morphology import remove_small_holes, remove_small_objects

    res = ImportResult(kind="raster", name=name, units="px")
    try:
        img = Image.open(io.BytesIO(data))
        img = ImageOps.exif_transpose(img)
    except Exception as e:
        res.errors.append(f"Картинка не читается: {e}")
        return res
    dpi_file = img.info.get("dpi")
    if opts.raster_dpi > 0:
        dpi = float(opts.raster_dpi)
        res.units_note = f"{dpi:g} точек на дюйм (указано вручную)"
    elif dpi_file and float(dpi_file[0]) > 1:
        dpi = float(dpi_file[0])
        res.units_note = f"{dpi:g} точек на дюйм (из файла)"
    else:
        dpi = DEFAULT_DPI
        res.units_note = f"в файле нет DPI: считаю {dpi:g} точек на дюйм (важно только для «1:1»)"
    if img.mode in ("RGBA", "LA", "P"):
        img = img.convert("RGBA")
        bg = Image.new("RGBA", img.size, (255, 255, 255, 255))
        img = Image.alpha_composite(bg, img)
    gray = img.convert("L")
    w0, h0 = gray.size
    if max(w0, h0) > MAX_SIDE:
        f = MAX_SIDE / max(w0, h0)
        gray = gray.resize((max(1, round(w0 * f)), max(1, round(h0 * f))), Image.LANCZOS)
        dpi *= f
        res.warnings.append(f"Картинка {w0}×{h0} уменьшена до {gray.size[0]}×{gray.size[1]} для скорости")
    a = np.asarray(gray, dtype=np.uint8)
    if opts.threshold_auto:
        try:
            thr = float(threshold_otsu(a))
        except ValueError:
            thr = 128.0
        res.info["threshold"] = round(thr)
    else:
        thr = float(opts.threshold)
        res.info["threshold"] = opts.threshold
    dark = a <= thr if opts.threshold_auto else a < thr
    mask = ~dark if opts.invert else dark
    if mask.mean() > 0.5:
        res.warnings.append("Линиями считается больше половины картинки: проверь порог или галочку «инвертировать»")
    mask = remove_small_objects(mask, max_size=8)
    mask = remove_small_holes(mask, max_size=8)
    px_mm = 25.4 / dpi
    res.info.update({"width_px": gray.size[0], "height_px": gray.size[1], "dpi": round(dpi, 1)})
    for p in mask_to_paths(mask, px_mm, 0.5 * px_mm, -0.5 * px_mm):
        res.paths.append(DPath(p, len(p) > 2 and p[0] == p[-1]))
    res.warnings.append("Растровая картинка: линии найдены по скелету, качество ниже, чем у вектора "
                        "(SVG, DXF, PDF). По возможности используй векторный файл")
    return res
