from __future__ import annotations

import hashlib
import json
import os
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import uharfbuzz as hb
from fontTools.pens.basePen import BasePen
from fontTools.ttLib import TTFont

from ..paths import user_dir
from .base import FontInfo, FontMetrics, Glyph, GlyphProvider, ShapedGlyph
from .font_analysis import SHAPING_FEATURES, analyze, is_free_variant
from .skeleton import SkeletonParams, skeleton_strokes
from .svgparse import _cubic_flat, _mid

CACHE_VERSION = 2


class _ContourPen(BasePen):
    def __init__(self, glyphset, tol: float):
        super().__init__(glyphset)
        self.tol = tol
        self.contours: list[list[tuple[float, float]]] = []
        self._cur: list[tuple[float, float]] | None = None

    def _moveTo(self, p):
        self._flush()
        self._cur = [p]

    def _lineTo(self, p):
        self._cur.append(p)

    def _curveToOne(self, p1, p2, p3):
        self._cubic(self._cur[-1], p1, p2, p3, 0)

    def _qCurveToOne(self, p1, p2):
        p0 = self._cur[-1]
        c1 = (p0[0] + 2 / 3 * (p1[0] - p0[0]), p0[1] + 2 / 3 * (p1[1] - p0[1]))
        c2 = (p2[0] + 2 / 3 * (p1[0] - p2[0]), p2[1] + 2 / 3 * (p1[1] - p2[1]))
        self._cubic(p0, c1, c2, p2, 0)

    def _cubic(self, p0, p1, p2, p3, depth):
        if depth > 12 or _cubic_flat(p0, p1, p2, p3, self.tol):
            self._cur.append(p3)
            return
        p01, p12, p23 = _mid(p0, p1), _mid(p1, p2), _mid(p2, p3)
        p012, p123 = _mid(p01, p12), _mid(p12, p23)
        m = _mid(p012, p123)
        self._cubic(p0, p01, p012, m, depth + 1)
        self._cubic(m, p123, p23, p3, depth + 1)

    def _closePath(self):
        if self._cur and self._cur[0] != self._cur[-1]:
            self._cur.append(self._cur[0])
        self._flush()

    def _endPath(self):
        self._flush()

    def _flush(self):
        if self._cur and len(self._cur) > 1:
            self.contours.append(self._cur)
        self._cur = None


class _FontData:
    def __init__(self, path: Path):
        self.path = path
        raw = path.read_bytes()
        self.hash = hashlib.sha1(raw).hexdigest()[:16]
        self.tt = TTFont(path, lazy=False)
        self.upem = self.tt["head"].unitsPerEm
        self.order = self.tt.getGlyphOrder()
        self.glyphset = self.tt.getGlyphSet()
        self.cmap = self.tt.getBestCmap() or {}
        self.hmtx = self.tt["hmtx"].metrics
        self.hb_font = hb.Font(hb.Face(hb.Blob(raw)))
        self.analysis = analyze(self.tt)
        self.lock = threading.RLock()
        self.contour_cache: dict[str, list] = {}
        self.glyph_cache: dict[tuple[str, str], Glyph] = {}
        self.disk_loaded: set[str] = set()
        self.name = self._family_name()
        self.metrics = self._metrics()

    def _family_name(self) -> str:
        nt = self.tt["name"]
        for nid in (4, 1):
            rec = nt.getDebugName(nid)
            if rec:
                return rec
        return self.path.stem

    def contours(self, name: str) -> list[list[tuple[float, float]]]:
        with self.lock:
            c = self.contour_cache.get(name)
            if c is None:
                pen = _ContourPen(self.glyphset, tol=self.upem * 0.0002)
                self.glyphset[name].draw(pen)
                pen._flush()
                s = 1.0 / self.upem
                c = [[(x * s, y * s) for x, y in cont] for cont in pen.contours]
                self.contour_cache[name] = c
            return c

    def _top(self, ch: str) -> float | None:
        name = self.cmap.get(ord(ch))
        if not name:
            return None
        ys = [p[1] for c in self.contours(name) for p in c]
        return max(ys) if ys else None

    def _metrics(self) -> FontMetrics:
        os2 = self.tt["OS/2"] if "OS/2" in self.tt else None
        sx = getattr(os2, "sxHeight", 0) / self.upem if os2 is not None else 0
        xh, src = None, ""
        for ch in "хx":
            t = self._top(ch)
            if t and t > 0:
                xh, src = t, f"по глифу «{ch}»" + (f", sxHeight {sx:.3f}" if sx else "")
                break
        if xh is None:
            xh, src = (sx, "OS/2.sxHeight") if sx > 0 else (0.5, "не найдена, принято 0.5 em")
        cap = getattr(os2, "sCapHeight", 0) / self.upem if os2 is not None else 0
        if not cap:
            cap = self._top("Н") or self._top("H") or xh * 1.4
        hhea = self.tt["hhea"]
        return FontMetrics(x_height=xh, cap_height=cap, ascent=hhea.ascent / self.upem,
                           descent=hhea.descent / self.upem, x_height_source=src)

    def _cache_file(self, key: str) -> Path:
        d = user_dir() / "cache" / "outlines"
        d.mkdir(parents=True, exist_ok=True)
        return d / f"{self.hash}_v{CACHE_VERSION}_{key}.jsonl"

    def load_disk(self, key: str) -> None:
        if key in self.disk_loaded:
            return
        self.disk_loaded.add(key)
        f = self._cache_file(key)
        self._trim_disk(keep=f)
        if not f.exists():
            return
        for line in f.read_text(encoding="utf-8").splitlines():
            try:
                d = json.loads(line)
                g = Glyph(name=d["n"], strokes=tuple(tuple((p[0], p[1]) for p in s) for s in d["s"]),
                          advance=d["a"])
                self.glyph_cache[(d["n"], key)] = g
            except (ValueError, KeyError):
                continue

    def _trim_disk(self, keep: Path, limit: int = 6) -> None:
        try:
            files = sorted(keep.parent.glob(f"{self.hash}_*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)
            for old in [p for p in files if p != keep][limit - 1:]:
                old.unlink(missing_ok=True)
        except OSError:
            pass

    def save_disk(self, key: str, g: Glyph) -> None:
        line = json.dumps({"n": g.name, "a": g.advance, "s": [[[x, y] for x, y in s] for s in g.strokes]})
        try:
            with self._cache_file(key).open("a", encoding="utf-8") as f:
                f.write(line + "\n")
        except OSError:
            pass


class OutlineGlyphProvider(GlyphProvider):
    mode = "outlines"

    def __init__(self, data: _FontData, params: SkeletonParams | None = None):
        self.data = data
        self.params = params or SkeletonParams()

    @classmethod
    def from_path(cls, path: str | Path) -> "OutlineGlyphProvider":
        p = Path(path)
        if p.suffix.lower() not in (".ttf", ".otf"):
            raise ValueError(f"Режим «Контуры» принимает .ttf или .otf: {p.name}")
        return cls(_FontData(p))

    def with_options(self, options) -> "OutlineGlyphProvider":
        if options is None:
            return self
        d = options if isinstance(options, dict) else options.model_dump()
        params = SkeletonParams(**{k: float(d[k]) for k in SkeletonParams.__dataclass_fields__ if k in d})
        return self if params == self.params else OutlineGlyphProvider(self.data, params)

    @property
    def name(self) -> str:
        return self.data.name

    @property
    def metrics(self) -> FontMetrics:
        return self.data.metrics

    def resolve(self, name: str) -> str:
        if name.startswith("#") and name[1:].isdigit():
            return self.data.order[int(name[1:])]
        if name not in self.data.hmtx:
            raise KeyError(name)
        return name

    def glyph(self, name: str) -> Glyph:
        name = self.resolve(name)
        key = self.params.key()
        d = self.data
        with d.lock:
            d.load_disk(key)
            g = d.glyph_cache.get((name, key))
        if g is not None:
            return g
        res = skeleton_strokes(d.contours(name), d.metrics.x_height, self.params)
        g = Glyph(name=name, strokes=tuple(tuple(s) for s in res.strokes),
                  advance=d.hmtx[name][0] / d.upem)
        with d.lock:
            d.glyph_cache[(name, key)] = g
            d.save_disk(key, g)
        return g

    def prepare(self, names) -> None:
        key = self.params.key()
        with self.data.lock:
            self.data.load_disk(key)
            todo = sorted({self.resolve(n) for n in names} - {n for n, k in self.data.glyph_cache if k == key})
        if len(todo) < 2:
            return
        workers = min(len(todo), max(1, (os.cpu_count() or 2) - 1), 8)
        with ThreadPoolExecutor(max_workers=workers) as ex:
            list(ex.map(self.glyph, todo))

    def glyph_names_for_char(self, ch: str) -> list[str]:
        primary = self.data.cmap.get(ord(ch)) if len(ch) == 1 else None
        if not primary:
            return []
        return [primary] + [n for n in self.data.analysis["variants"].get(ch, {}) if n != primary]

    def advance(self, name: str) -> float:
        return self.data.hmtx[self.resolve(name)][0] / self.data.upem

    def variant_pool(self, ch: str) -> list[str]:
        names = self.glyph_names_for_char(ch)
        if len(names) < 2:
            return names
        src = self.data.analysis["variants"].get(ch, {})
        return names[:1] + [n for n in names[1:] if any(is_free_variant(s) for s in src.get(n, []))]

    def has_char(self, ch: str) -> bool:
        return len(ch) == 1 and ord(ch) in self.data.cmap

    def variants(self) -> dict[str, list[str]]:
        return {ch: self.glyph_names_for_char(ch) for ch in self.data.analysis["variants"]}

    def space_advance(self) -> float:
        name = self.data.cmap.get(32)
        return self.data.hmtx[name][0] / self.data.upem if name else 0.3

    def shape(self, text: str) -> list[ShapedGlyph]:
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        buf.language = "ru"
        with self.data.lock:
            hb.shape(self.data.hb_font, buf, SHAPING_FEATURES)
        s = 1.0 / self.data.upem
        out = []
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            if info.codepoint == 0:
                continue
            out.append(ShapedGlyph(name=self.data.order[info.codepoint], cluster=info.cluster,
                                   advance=pos.x_advance * s, x_offset=pos.x_offset * s,
                                   y_offset=pos.y_offset * s))
        return out

    def glyph_names(self) -> list[str]:
        return list(self.data.order)

    def info(self) -> FontInfo:
        a = self.data.analysis
        return FontInfo(
            name=self.data.name, mode=self.mode, source=str(self.data.path),
            glyph_count=len(self.data.order),
            chars="".join(sorted(chr(c) for c in self.data.cmap if c > 31)),
            variants=self.variants(), metrics=self.data.metrics,
            features=a["gsub_features"], gpos_features=a["gpos_features"],
            variant_sources=a["variants"], ligatures=a["ligatures"],
        )

    def debug_glyph(self, name: str) -> dict:
        name = self.resolve(name)
        t = time.perf_counter()
        res = skeleton_strokes(self.data.contours(name), self.data.metrics.x_height, self.params, want_raw=True)
        return {"name": name, "outline": self.data.contours(name), "strokes": res.strokes, "raw": res.raw,
                "closed": res.closed, "advance": self.data.hmtx[name][0] / self.data.upem,
                "ms": round((time.perf_counter() - t) * 1000)}
