use std::io::Cursor;

use serde_json::json;

use crate::drawing::model::{DPath, ImportResult, Mask};
use crate::numeric::{format_g, round_to};
use crate::settings::{DrawingImport, RasterMode};
use crate::skeleton::{mask_to_paths, skeletonize};

pub const MAX_SIDE: usize = 3000;
pub const DEFAULT_DPI: f64 = 300.0;
pub const MAX_SKELETON_PX: usize = 25000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channels {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

struct Raw {
    w: usize,
    h: usize,
    ch: Channels,
    data: Vec<u8>,
    composite: bool,
    dpi: Option<f64>,
    orientation: u16,
}

impl Channels {
    fn n(self) -> usize {
        match self {
            Channels::Gray => 1,
            Channels::GrayAlpha => 2,
            Channels::Rgb => 3,
            Channels::Rgba => 4,
        }
    }
}

struct Exif {
    orientation: Option<u16>,
    x_resolution: Option<(u32, u32)>,
    resolution_unit: Option<u16>,
}

fn parse_tiff(t: &[u8]) -> Option<Exif> {
    let le = match t.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    let mut ex = Exif { orientation: None, x_resolution: None, resolution_unit: None };
    for i in 0..n {
        let e = ifd + 2 + 12 * i;
        let (tag, typ) = (u16_at(e)?, u16_at(e + 2)?);
        match (tag, typ) {
            (0x0112, 3) => ex.orientation = u16_at(e + 8),
            (0x0128, 3) => ex.resolution_unit = u16_at(e + 8),
            (0x011A, 5) => {
                let off = u32_at(e + 8)? as usize;
                ex.x_resolution = Some((u32_at(off)?, u32_at(off + 4)?));
            }
            _ => {}
        }
    }
    Some(ex)
}

fn unpack(row: &[u8], bits: u8, count: usize) -> Vec<u16> {
    match bits {
        8 => row[..count].iter().map(|&b| b as u16).collect(),
        16 => (0..count).map(|i| u16::from_be_bytes([row[2 * i], row[2 * i + 1]])).collect(),
        b => {
            let per = 8 / b as usize;
            let mask = (1u16 << b) - 1;
            (0..count)
                .map(|i| {
                    let byte = row[i / per] as u16;
                    let shift = 8 - b as usize * (i % per + 1);
                    (byte >> shift) & mask
                })
                .collect()
        }
    }
}

fn decode_png(data: &[u8]) -> Result<Raw, String> {
    let mut dec = png::Decoder::new(Cursor::new(data));
    dec.set_transformations(png::Transformations::IDENTITY);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let info = reader.info().clone();
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.bit_depth as u8;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("image too large")?];
    let out = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let line = out.line_size;
    let dpi = info.pixel_dims.filter(|d| d.unit == png::Unit::Meter).map(|d| d.xppu as f64 * 0.0254);
    let orientation = info.exif_metadata.as_deref().and_then(parse_tiff).and_then(|e| e.orientation).unwrap_or(1);
    let msb = |v: u16| if bits == 16 { (v >> 8) as u8 } else { v as u8 };
    let mut px = Vec::new();
    let (ch, composite) = match info.color_type {
        png::ColorType::Grayscale => {
            for y in 0..h {
                for v in unpack(&buf[y * line..], bits, w) {
                    px.push(match bits {
                        1 => {
                            if v != 0 {
                                255
                            } else {
                                0
                            }
                        }
                        2 => (v * 85) as u8,
                        4 => (v * 17) as u8,
                        8 => v as u8,
                        _ => v.min(255) as u8,
                    });
                }
            }
            (Channels::Gray, false)
        }
        png::ColorType::GrayscaleAlpha => {
            for y in 0..h {
                px.extend(unpack(&buf[y * line..], bits, 2 * w).into_iter().map(msb));
            }
            (Channels::GrayAlpha, true)
        }
        png::ColorType::Rgb => {
            for y in 0..h {
                px.extend(unpack(&buf[y * line..], bits, 3 * w).into_iter().map(msb));
            }
            (Channels::Rgb, false)
        }
        png::ColorType::Rgba => {
            for y in 0..h {
                px.extend(unpack(&buf[y * line..], bits, 4 * w).into_iter().map(msb));
            }
            (Channels::Rgba, true)
        }
        png::ColorType::Indexed => {
            let pal = info.palette.as_deref().unwrap_or(&[]);
            let trns = info.trns.as_deref().unwrap_or(&[]);
            for y in 0..h {
                for i in unpack(&buf[y * line..], bits, w) {
                    let i = i as usize;
                    let rgb = pal.get(3 * i..3 * i + 3).unwrap_or(&[0, 0, 0]);
                    px.extend_from_slice(rgb);
                    px.push(trns.get(i).copied().unwrap_or(255));
                }
            }
            (Channels::Rgba, true)
        }
    };
    Ok(Raw { w, h, ch, data: px, composite, dpi, orientation })
}

struct JpegMeta {
    dpi: Option<f64>,
    orientation: u16,
}

fn jpeg_meta(data: &[u8]) -> JpegMeta {
    let mut jfif_dpi: Option<f64> = None;
    let mut exif: Option<Option<Exif>> = None;
    let mut i = 2;
    while data.len() >= 4 && data[0] == 0xFF && data[1] == 0xD8 && i + 4 <= data.len() {
        if data[i] != 0xFF {
            break;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 || i + 2 + len > data.len() {
            break;
        }
        let m = &data[i + 4..i + 2 + len];
        if marker == 0xE0 && m.starts_with(b"JFIF") && m.len() >= 12 {
            let xd = u16::from_be_bytes([m[8], m[9]]) as f64;
            match m[7] {
                1 => jfif_dpi = Some(xd),
                2 => jfif_dpi = Some(xd * 2.54),
                _ => {}
            }
        }
        if marker == 0xE1 && m.starts_with(b"Exif\0\0") && exif.is_none() {
            exif = Some(parse_tiff(&m[6..]));
        }
        i += 2 + len;
    }
    let orientation = exif.as_ref().and_then(|e| e.as_ref()).and_then(|e| e.orientation).unwrap_or(1);
    let dpi = match (jfif_dpi, &exif) {
        (Some(v), _) => Some(v),
        (None, None) => None,
        (None, Some(e)) => Some(
            e.as_ref()
                .and_then(|e| match (e.resolution_unit, e.x_resolution) {
                    (Some(unit), Some((num, den))) if den != 0 => {
                        let v = num as f64 / den as f64;
                        Some(if unit == 3 { v * 2.54 } else { v })
                    }
                    _ => None,
                })
                .unwrap_or(72.0),
        ),
    };
    JpegMeta { dpi, orientation }
}

#[cfg(not(windows))]
fn decode_jpeg(data: &[u8]) -> Result<Raw, String> {
    let meta = jpeg_meta(data);
    let data = data.to_vec();
    std::panic::catch_unwind(move || -> Result<Raw, String> {
        let d = mozjpeg::Decompress::new_mem(&data).map_err(|e| e.to_string())?;
        let gray = matches!(d.color_space(), mozjpeg::ColorSpace::JCS_GRAYSCALE);
        let (w, h) = d.size();
        let (dpi, orientation) = (meta.dpi, meta.orientation);
        if gray {
            let mut s = d.grayscale().map_err(|e| e.to_string())?;
            let px: Vec<u8> = s.read_scanlines().map_err(|e| e.to_string())?;
            Ok(Raw { w, h, ch: Channels::Gray, data: px, composite: false, dpi, orientation })
        } else {
            let mut s = d.rgb().map_err(|e| e.to_string())?;
            let px: Vec<[u8; 3]> = s.read_scanlines().map_err(|e| e.to_string())?;
            Ok(Raw { w, h, ch: Channels::Rgb, data: px.concat(), composite: false, dpi, orientation })
        }
    })
    .map_err(|_| "cannot decode JPEG".to_string())?
}

#[cfg(windows)]
fn decode_jpeg(data: &[u8]) -> Result<Raw, String> {
    decode_jpeg_mupdf(data)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn decode_jpeg_mupdf(data: &[u8]) -> Result<Raw, String> {
    let meta = jpeg_meta(data);
    let img = mupdf::Image::from_bytes(data).map_err(|e| e.to_string())?;
    let pix = img.to_pixmap().map_err(|e| e.to_string())?;
    let (w, h, n) = (pix.width() as usize, pix.height() as usize, pix.n() as usize);
    let stride = pix.stride() as usize;
    let src = pix.samples();
    let alpha = usize::from(pix.alpha());
    let colors = n - alpha;
    let mut out = Vec::with_capacity(w * h * if colors == 1 { 1 } else { 3 });
    for y in 0..h {
        let row = &src[y * stride..y * stride + w * n];
        for p in row.chunks_exact(n) {
            match colors {
                1 => out.push(p[0]),
                3 => out.extend_from_slice(&p[..3]),
                4 => {
                    let k = 255 - p[3] as u32;
                    out.extend(p[..3].iter().map(|&c| ((255 - c as u32) * k / 255) as u8));
                }
                _ => return Err(format!("unsupported JPEG with {colors} channels")),
            }
        }
    }
    let ch = if colors == 1 { Channels::Gray } else { Channels::Rgb };
    Ok(Raw { w, h, ch, data: out, composite: false, dpi: meta.dpi, orientation: meta.orientation })
}

type PixelMap = Box<dyn Fn(usize, usize) -> (usize, usize)>;

fn transpose(raw: Raw) -> Raw {
    let (w, h, n) = (raw.w, raw.h, raw.ch.n());
    let map: Option<(usize, usize, PixelMap)> = match raw.orientation {
        2 => Some((w, h, Box::new(move |x, y| (w - 1 - x, y)))),
        3 => Some((w, h, Box::new(move |x, y| (w - 1 - x, h - 1 - y)))),
        4 => Some((w, h, Box::new(move |x, y| (x, h - 1 - y)))),
        5 => Some((h, w, Box::new(move |x, y| (y, x)))),
        6 => Some((h, w, Box::new(move |x, y| (y, h - 1 - x)))),
        7 => Some((h, w, Box::new(move |x, y| (w - 1 - y, h - 1 - x)))),
        8 => Some((h, w, Box::new(move |x, y| (w - 1 - y, x)))),
        _ => None,
    };
    let Some((nw, nh, src)) = map else { return raw };
    let mut out = vec![0u8; nw * nh * n];
    for y in 0..nh {
        for x in 0..nw {
            let (sx, sy) = src(x, y);
            let (d, s) = ((y * nw + x) * n, (sy * w + sx) * n);
            out[d..d + n].copy_from_slice(&raw.data[s..s + n]);
        }
    }
    Raw { w: nw, h: nh, data: out, ..raw }
}

fn l24(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16) as u8
}

fn div255(a: u32) -> u32 {
    ((a >> 8) + a) >> 8
}

fn composite_over_white(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    if a == 0 {
        return (255, 255, 255);
    }
    const PB: u32 = 7;
    let a = a as u32;
    let blend = 255 * (255 - a);
    let outa255 = a * 255 + blend;
    let coef1 = a * 255 * 255 * (1 << PB) / outa255;
    let coef2 = 255 * (1 << PB) - coef1;
    let ch = |s: u8| (div255(s as u32 * coef1 + 255 * coef2 + (0x80 << PB)) >> PB) as u8;
    (ch(r), ch(g), ch(b))
}

fn to_gray(raw: &Raw) -> Vec<u8> {
    let n = raw.ch.n();
    raw.data
        .chunks_exact(n)
        .map(|p| match raw.ch {
            Channels::Gray => p[0],
            Channels::GrayAlpha => {
                let (r, g, b) = composite_over_white(p[0], p[0], p[0], p[1]);
                l24(r, g, b)
            }
            Channels::Rgb => l24(p[0], p[1], p[2]),
            Channels::Rgba => {
                if raw.composite {
                    let (r, g, b) = composite_over_white(p[0], p[1], p[2], p[3]);
                    l24(r, g, b)
                } else {
                    l24(p[0], p[1], p[2])
                }
            }
        })
        .collect()
}

fn lanczos(x: f64) -> f64 {
    let sinc = |x: f64| {
        if x == 0.0 {
            1.0
        } else {
            let x = x * std::f64::consts::PI;
            x.sin() / x
        }
    };
    if (-3.0..3.0).contains(&x) { sinc(x) * sinc(x / 3.0) } else { 0.0 }
}

const PRECISION_BITS: u32 = 32 - 8 - 2;

fn coeffs(in_size: usize, out_size: usize) -> (usize, Vec<(usize, usize)>, Vec<i32>) {
    let (in0, in1) = (0.0f32, in_size as f32);
    let scale = (in1 - in0) as f64 / out_size as f64;
    let filterscale = if scale < 1.0 { 1.0 } else { scale };
    let support = 3.0 * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let mut kk = vec![0f64; out_size * ksize];
    let mut bounds = Vec::with_capacity(out_size);
    let inv = 1.0 / filterscale;
    for xx in 0..out_size {
        let center = in0 as f64 + (xx as f64 + 0.5) * scale;
        let mut ww = 0.0;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize - xmin;
        let k = &mut kk[xx * ksize..(xx + 1) * ksize];
        for (x, kx) in k.iter_mut().enumerate().take(xmax) {
            let w = lanczos((x as f64 + xmin as f64 - center + 0.5) * inv);
            *kx = w;
            ww += w;
        }
        if ww != 0.0 {
            for v in k.iter_mut().take(xmax) {
                *v /= ww;
            }
        }
        bounds.push((xmin, xmax));
    }
    let fixed = kk
        .iter()
        .map(|&v| {
            let s = v * (1u32 << PRECISION_BITS) as f64;
            if v < 0.0 { (-0.5 + s) as i32 } else { (0.5 + s) as i32 }
        })
        .collect();
    (ksize, bounds, fixed)
}

fn clip8(v: i32) -> u8 {
    (v >> PRECISION_BITS).clamp(0, 255) as u8
}

fn resize_lanczos(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let (ks_v, bounds_v, kk_v) = coeffs(h, nh);
    let first = bounds_v[0].0;
    let last = bounds_v[nh - 1].0 + bounds_v[nh - 1].1;
    let need_h = nw != w;
    let need_v = nh != h;
    let (mut img, mut iw, mut ih, mut row0) = (src.to_vec(), w, h, 0usize);
    if need_h {
        let (ks, bounds, kk) = coeffs(w, nw);
        let rows = last - first;
        let mut out = vec![0u8; nw * rows];
        for yy in 0..rows {
            let line = &src[(yy + first) * w..(yy + first + 1) * w];
            for (xx, &(xmin, xmax)) in bounds.iter().enumerate() {
                let k = &kk[xx * ks..];
                let mut ss = 1i32 << (PRECISION_BITS - 1);
                for x in 0..xmax {
                    ss = ss.wrapping_add(line[x + xmin] as i32 * k[x]);
                }
                out[yy * nw + xx] = clip8(ss);
            }
        }
        img = out;
        iw = nw;
        ih = rows;
        row0 = first;
    }
    if !need_v {
        return img;
    }
    let mut out = vec![0u8; iw * nh];
    for (yy, &(ymin, ymax)) in bounds_v.iter().enumerate() {
        let ymin = ymin - row0;
        let k = &kk_v[yy * ks_v..];
        for xx in 0..iw {
            let mut ss = 1i32 << (PRECISION_BITS - 1);
            for y in 0..ymax {
                ss = ss.wrapping_add(img[(y + ymin) * iw + xx] as i32 * k[y]);
            }
            out[yy * iw + xx] = clip8(ss);
        }
    }
    let _ = ih;
    out
}

pub fn threshold_otsu(a: &[u8]) -> f64 {
    let first = a[0];
    if a.iter().all(|&v| v == first) {
        return first as f64;
    }
    let (lo, hi) = (*a.iter().min().expect("pixels") as usize, *a.iter().max().expect("pixels") as usize);
    let mut counts = vec![0i64; hi - lo + 1];
    for &v in a {
        counts[v as usize - lo] += 1;
    }
    let n = counts.len();
    let centers: Vec<i64> = (lo as i64..=hi as i64).collect();
    let mut w1 = vec![0i64; n];
    let mut acc = 0;
    for i in 0..n {
        acc += counts[i];
        w1[i] = acc;
    }
    let mut w2 = vec![0i64; n];
    acc = 0;
    for i in (0..n).rev() {
        acc += counts[i];
        w2[i] = acc;
    }
    let mut m1 = vec![0f64; n];
    let mut cs = 0i64;
    for i in 0..n {
        cs += counts[i] * centers[i];
        m1[i] = cs as f64 / w1[i] as f64;
    }
    let mut m2 = vec![0f64; n];
    cs = 0;
    for i in (0..n).rev() {
        cs += counts[i] * centers[i];
        m2[i] = cs as f64 / w2[i] as f64;
    }
    let mut best = 0;
    let mut best_v = f64::NEG_INFINITY;
    for i in 0..n - 1 {
        let d = m1[i] - m2[i + 1];
        let v = (w1[i] * w2[i + 1]) as f64 * (d * d);
        if v.is_nan() {
            best = i;
            break;
        }
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    centers[best] as f64
}

fn label4_sizes(mask: &[bool], w: usize, h: usize) -> (Vec<usize>, Vec<usize>) {
    let mut lab = vec![0usize; w * h];
    let mut sizes = vec![0usize];
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || lab[start] != 0 {
            continue;
        }
        let id = sizes.len();
        sizes.push(0);
        lab[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            sizes[id] += 1;
            let (r, c) = (i / w, i % w);
            let mut visit = |j: usize| {
                if mask[j] && lab[j] == 0 {
                    lab[j] = id;
                    stack.push(j);
                }
            };
            if r > 0 {
                visit(i - w);
            }
            if r + 1 < h {
                visit(i + w);
            }
            if c > 0 {
                visit(i - 1);
            }
            if c + 1 < w {
                visit(i + 1);
            }
        }
    }
    (lab, sizes)
}

fn remove_small_objects(mask: &mut [bool], w: usize, h: usize, max_size: usize) {
    let (lab, sizes) = label4_sizes(mask, w, h);
    for (m, &l) in mask.iter_mut().zip(&lab) {
        if l != 0 && sizes[l] <= max_size {
            *m = false;
        }
    }
}

fn remove_small_holes(mask: &mut [bool], w: usize, h: usize, max_size: usize) {
    let mut inv: Vec<bool> = mask.iter().map(|v| !v).collect();
    remove_small_objects(&mut inv, w, h, max_size);
    for (m, v) in mask.iter_mut().zip(inv) {
        *m = !v;
    }
}

fn py_round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

pub fn import_raster(data: &[u8], name: &str, opts: &DrawingImport) -> ImportResult {
    let mut res = ImportResult::new("raster", name);
    res.units = "px".into();
    let decoded = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(data)
    } else if data.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(data)
    } else {
        Err("cannot identify image file".into())
    };
    let raw = match decoded {
        Ok(r) => transpose(r),
        Err(e) => {
            res.errors.push(format!("Картинка не читается: {e}"));
            return res;
        }
    };
    let mut dpi;
    if opts.raster_dpi > 0.0 {
        dpi = opts.raster_dpi;
        res.units_note = format!("{} точек на дюйм (указано вручную)", format_g(dpi, 6));
    } else if let Some(d) = raw.dpi.filter(|&d| d > 1.0) {
        dpi = d;
        res.units_note = format!("{} точек на дюйм (из файла)", format_g(dpi, 6));
    } else {
        dpi = DEFAULT_DPI;
        res.units_note = format!("в файле нет DPI: считаю {} точек на дюйм (важно только для «1:1»)", format_g(dpi, 6));
    }
    let mut gray = to_gray(&raw);
    let (w0, h0) = (raw.w, raw.h);
    let (mut w, mut h) = (w0, h0);
    if w0.max(h0) > MAX_SIDE {
        let f = MAX_SIDE as f64 / w0.max(h0) as f64;
        let nw = py_round(w0 as f64 * f).max(1) as usize;
        let nh = py_round(h0 as f64 * f).max(1) as usize;
        gray = resize_lanczos(&gray, w0, h0, nw, nh);
        (w, h) = (nw, nh);
        dpi *= f;
        res.warnings.push(format!("Картинка {w0}×{h0} уменьшена до {w}×{h} для скорости"));
    }
    let thr = if opts.threshold_auto {
        let t = if gray.is_empty() { 128.0 } else { threshold_otsu(&gray) };
        res.info.insert("threshold".into(), json!(py_round(t)));
        t
    } else {
        res.info.insert("threshold".into(), json!(opts.threshold));
        opts.threshold as f64
    };
    let mut mask: Vec<bool> = gray
        .iter()
        .map(|&v| {
            let dark = if opts.threshold_auto { v as f64 <= thr } else { (v as f64) < thr };
            if opts.invert { !dark } else { dark }
        })
        .collect();
    let on = mask.iter().filter(|&&v| v).count();
    if on as f64 / mask.len() as f64 > 0.5 {
        res.warnings
            .push("Линиями считается больше половины картинки: проверь порог или галочку «инвертировать»".into());
    }
    remove_small_objects(&mut mask, w, h, 8);
    remove_small_holes(&mut mask, w, h, 8);
    let px_mm = 25.4 / dpi;
    res.info.insert("width_px".into(), json!(w));
    res.info.insert("height_px".into(), json!(h));
    res.info.insert("dpi".into(), json!(round_to(dpi, 1)));
    let m = Mask { rows: h, cols: w, data: mask };
    let mut too_complex = false;
    if opts.raster_mode != RasterMode::Fill {
        let sk = skeletonize(&m).data.iter().filter(|&&v| v).count();
        if sk > MAX_SKELETON_PX {
            too_complex = true;
            res.warnings.push(format!(
                "Картинка слишком детальная для центральных линий ({sk} точек скелета): нарисована заливкой штрихами"
            ));
        }
    }
    if opts.raster_mode == RasterMode::Fill || too_complex {
        res.fill_mask = Some(m);
        res.fill_px = px_mm;
        res.info.insert("mode".into(), json!("fill"));
        return res;
    }
    for p in mask_to_paths(&m, px_mm, 0.5 * px_mm, -0.5 * px_mm) {
        let closed = p.len() > 2 && p[0] == p[p.len() - 1];
        res.paths.push(DPath::new(p, closed, None, ""));
    }
    res.warnings.push(
        "Растровая картинка: линии найдены по скелету, качество ниже, чем у вектора (SVG, DXF, PDF). По возможности \
         используй векторный файл"
            .into(),
    );
    res
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn mupdf_jpeg_matches_mozjpeg() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/raster/input");
        for name in ["rgb.jpg", "gray.jpg", "rotated_exif.jpg", "rotated_noresunit.jpg"] {
            let data = std::fs::read(dir.join(name)).unwrap();
            let a = decode_jpeg(&data).unwrap();
            let b = decode_jpeg_mupdf(&data).unwrap();
            assert_eq!(
                (a.w, a.h, a.ch.n(), a.dpi, a.orientation),
                (b.w, b.h, b.ch.n(), b.dpi, b.orientation),
                "{name}"
            );
            assert!(a.data == b.data, "{name}");
        }
    }
}
