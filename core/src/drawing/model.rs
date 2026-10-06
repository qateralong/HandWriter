use indexmap::IndexMap;

use crate::geometry::Point;
use crate::numeric;

pub fn unit_mm(unit: &str) -> Option<f64> {
    Some(match unit {
        "mm" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "in" => 25.4,
        "ft" => 304.8,
        "pt" => 25.4 / 72.0,
        "px" => 25.4 / 96.0,
        "pc" => 25.4 / 6.0,
        "q" => 0.25,
        _ => return None,
    })
}

pub fn unit_name(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "mm" => "мм",
        "cm" => "см",
        "m" => "м",
        "in" => "дюймы",
        "ft" => "футы",
        "pt" => "пункты (1/72″)",
        "px" => "пиксели (96 на дюйм)",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct DPath {
    pub points: Vec<Point>,
    pub closed: bool,
    pub width: Option<f64>,
    pub layer: String,
    pub dash: Option<Vec<f64>>,
    pub dash_offset: f64,
}

impl DPath {
    pub fn new(points: Vec<Point>, closed: bool, width: Option<f64>, layer: &str) -> Self {
        Self { points, closed, width, layer: layer.to_string(), dash: None, dash_offset: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextMark {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerInfo {
    pub count: usize,
    pub width: Option<f64>,
    pub linetype: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<bool>,
}

impl Mask {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self { rows, cols, data: vec![false; rows * cols] }
    }

    pub fn get(&self, r: usize, c: usize) -> bool {
        self.data[r * self.cols + c]
    }

    pub fn set(&mut self, r: usize, c: usize, v: bool) {
        self.data[r * self.cols + c] = v;
    }

    pub fn any(&self) -> bool {
        self.data.iter().any(|&v| v)
    }

    pub fn transposed(&self) -> Mask {
        let mut m = Mask::new(self.cols, self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                m.set(c, r, self.get(r, c));
            }
        }
        m
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub kind: String,
    pub name: String,
    pub paths: Vec<DPath>,
    pub texts: Vec<TextMark>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub units: String,
    pub units_note: String,
    pub pages: i64,
    pub page: i64,
    pub layers: IndexMap<String, LayerInfo>,
    pub info: serde_json::Map<String, serde_json::Value>,
    pub fill_mask: Option<Mask>,
    pub fill_px: f64,
}

impl ImportResult {
    pub fn new(kind: &str, name: &str) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            paths: Vec::new(),
            texts: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            units: "mm".into(),
            units_note: String::new(),
            pages: 1,
            page: 1,
            layers: IndexMap::new(),
            info: serde_json::Map::new(),
            fill_mask: None,
            fill_px: 0.0,
        }
    }

    pub fn fill_bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let m = self.fill_mask.as_ref().filter(|m| m.any())?;
        let rows: Vec<usize> = (0..m.rows).filter(|&r| (0..m.cols).any(|c| m.get(r, c))).collect();
        let cols: Vec<usize> = (0..m.cols).filter(|&c| (0..m.rows).any(|r| m.get(r, c))).collect();
        let px = self.fill_px;
        let (r0, r1, c0, c1) = (rows[0], rows[rows.len() - 1], cols[0], cols[cols.len() - 1]);
        Some((c0 as f64 * px, (-((r1 + 1) as i64)) as f64 * px, (c1 + 1) as f64 * px, (-(r0 as i64)) as f64 * px))
    }

    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut pts: Vec<(f64, f64)> = self.paths.iter().flat_map(|p| p.points.iter().copied()).collect();
        if let Some(fb) = self.fill_bbox() {
            pts.push((fb.0, fb.1));
            pts.push((fb.2, fb.3));
        }
        let mut it = pts.into_iter();
        let (x, y) = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
        for (x, y) in it {
            x0 = numeric::min(x0, x);
            y0 = numeric::min(y0, y);
            x1 = numeric::max(x1, x);
            y1 = numeric::max(y1, y);
        }
        Some((x0, y0, x1, y1))
    }
}
