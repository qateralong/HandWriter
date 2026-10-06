use super::curves::*;
use super::doc::*;
use super::geom::*;
use super::path::*;

const TRANSFORMABLE: [&str; 44] = [
    "3DFACE",
    "3DSOLID",
    "ARC",
    "ARC_DIMENSION",
    "ATTDEF",
    "ATTRIB",
    "BODY",
    "CIRCLE",
    "DGNUNDERLAY",
    "DIMENSION",
    "DWFUNDERLAY",
    "ELLIPSE",
    "EXTRUDEDSURFACE",
    "HATCH",
    "HELIX",
    "IMAGE",
    "INSERT",
    "LARGE_RADIAL_DIMENSION",
    "LEADER",
    "LIGHT",
    "LINE",
    "LOFTEDSURFACE",
    "LWPOLYLINE",
    "MESH",
    "MLEADER",
    "MLINE",
    "MPOLYGON",
    "MTEXT",
    "MULTILEADER",
    "PDFREFERENCE",
    "PDFUNDERLAY",
    "POINT",
    "POLYLINE",
    "RAY",
    "REGION",
    "REVOLVEDSURFACE",
    "SHAPE",
    "SOLID",
    "SPLINE",
    "SURFACE",
    "SWEPTSURFACE",
    "TEXT",
    "TOLERANCE",
    "TRACE",
];

const TRANSFORMABLE_MORE: [&str; 3] = ["VERTEX", "WIPEOUT", "XLINE"];

fn is_transformable(t: &str) -> bool {
    TRANSFORMABLE.contains(&t) || TRANSFORMABLE_MORE.contains(&t)
}

fn sign(f: f64) -> f64 {
    if f < 0.0 { -1.0 } else { 1.0 }
}

fn thickness_extrusion_without_ocs(c: &mut Common, m: &M44) {
    let mut ext = c.extrusion.unwrap_or(Z_AXIS);
    if ext.is_null() {
        ext = Z_AXIS;
    }
    if let Some(t) = c.thickness
        && t != 0.0
    {
        let r = sign(t);
        c.thickness = Some(m.transform_direction(ext * t).magnitude() * r);
    }
    let e = m.transform_direction(ext);
    c.extrusion = Some(e.normalize().unwrap_or(Z_AXIS));
}

pub fn transform(e: &mut Entity, m: &M44) -> R<()> {
    let ext = e.c.extrusion();
    match &mut e.body {
        Body::Line { start, end } => {
            *start = m.transform(*start);
            *end = m.transform(*end);
            thickness_extrusion_without_ocs(&mut e.c, m);
        }
        Body::Point { location } => {
            *location = m.transform(*location);
            thickness_extrusion_without_ocs(&mut e.c, m);
        }
        Body::Circle { center, radius } => {
            let ocs = OcsTransform::new(ext, m)?;
            circle_transform(&mut e.c, &ocs, center, radius)?;
        }
        Body::Arc { center, radius, start, end } => {
            let ocs = OcsTransform::new(ext, m)?;
            circle_transform(&mut e.c, &ocs, center, radius)?;
            if !isclose_def(arc_angle_span_deg(*start, *end), 360.0) {
                let (s, en) = ocs.transform_ccw_arc_angles_deg(*start, *end);
                *start = s;
                *end = en;
            }
        }
        Body::Ellipse { center, major, ratio, start, end } => {
            let mut ce = Ellipse::new(*center, *major, ext, *ratio, *start, *end, true)?;
            ce.transform(m)?;
            let a = ce.dxfattribs()?;
            *center = a.center;
            *major = a.major_axis;
            *ratio = a.ratio;
            *start = a.start_param;
            *end = a.end_param;
            e.c.extrusion = Some(a.extrusion);
        }
        Body::LwPolyline { pts, const_width, elevation, .. } => {
            let ocs = OcsTransform::new(ext, m)?;
            if !ocs.scale_uniform && pts.iter().any(|p| p[4] != 0.0) {
                return Err(Fail::NonUniform);
            }
            let elev = elevation.unwrap_or(0.0);
            let verts: Vec<V3> = pts.iter().map(|p| ocs.transform_vertex(v3(p[0], p[1], elev))).collect();
            for (p, v) in pts.iter_mut().zip(&verts) {
                *p = [v.x, v.y, ocs.transform_width(p[2]), ocs.transform_width(p[3]), p[4]];
            }
            if let Some(v) = verts.first() {
                *elevation = Some(v.z);
            }
            if let Some(w) = const_width {
                *w = ocs.transform_width(*w);
            }
            if let Some(t) = e.c.thickness {
                e.c.thickness = Some(ocs.transform_thickness(t));
            }
            e.c.extrusion = Some(ocs.new_extrusion());
        }
        Body::Polyline { flags, elevation, vertices } => {
            if *flags & (8 | 16 | 64) == 0 {
                let ocs = OcsTransform::new(ext, m)?;
                if !ocs.scale_uniform && vertices.iter().any(|v| v.bulge.is_some_and(|b| b != 0.0)) {
                    return Err(Fail::NonUniform);
                }
                let z = elevation.map(|p| p.z);
                let verts: Vec<V3> = vertices
                    .iter()
                    .map(|v| {
                        ocs.transform_vertex(match z {
                            Some(z) => v.location.with_z(z),
                            None => v.location,
                        })
                    })
                    .collect();
                if let Some(v) = verts.first() {
                    *elevation = Some(v3(0.0, 0.0, v.z));
                }
                for (v, l) in vertices.iter_mut().zip(verts) {
                    v.location = l;
                    v.start_width = v.start_width.map(|w| ocs.transform_width(w));
                    v.end_width = v.end_width.map(|w| ocs.transform_width(w));
                }
                if let Some(t) = e.c.thickness {
                    e.c.thickness = Some(ocs.transform_thickness(t));
                }
                e.c.extrusion = Some(ocs.new_extrusion());
            } else {
                for v in vertices.iter_mut() {
                    if v.flags & 192 != 128 {
                        v.location = m.transform(v.location);
                    }
                }
            }
        }
        Body::Spline { cps, fits, st, et, .. } => {
            m.transform_array(cps);
            m.transform_array(fits);
            for v in [st, et].into_iter().flatten() {
                *v = m.transform_direction(*v);
            }
            if let Some(x) = e.c.extrusion {
                e.c.extrusion = Some(m.transform_direction(x));
            }
        }
        Body::Insert { insert, sx, sy, sz, rotation, .. } => {
            let ocs = Ocs::new(ext)?;
            let ux = m.transform_direction(ocs.ux());
            let uy = m.transform_direction(ocs.uy());
            let uz = m.transform_direction(ocs.uz());
            let xs = ux.magnitude() * *sx;
            let mut ys = uy.magnitude() * *sy;
            let zs = uz.magnitude() * *sz;
            let (ux, uy, uz) = (ux.normalize()?, uy.normalize()?, uz.normalize()?);
            let tol = 1e-9;
            if ux.dot(uz).abs() > tol || ux.dot(uy).abs() > tol || uz.dot(uy).abs() > tol {
                return Err(Fail::InsertTransform);
            }
            if !uz.cross(ux).isclose_tol(uy, 1e-9, tol) {
                ys = -ys;
            }
            let ot = OcsTransform::from_ocs(Ocs::new(ext)?, Ocs::new(uz)?, m);
            *insert = Some(ot.transform_vertex(insert.unwrap_or(NULLVEC)));
            *rotation = ot.transform_deg_angle(*rotation);
            e.c.extrusion = Some(uz);
            *sx = xs;
            *sy = ys;
            *sz = zs;
        }
        Body::Dimension { text_midpoint, insert, content, .. } => {
            let ocs = OcsTransform::new(ext, m)?;
            if let Some(p) = text_midpoint {
                *p = ocs.transform_vertex(*p);
            }
            if let Some(p) = insert {
                *p = ocs.transform_vertex(*p);
            }
            e.c.extrusion = Some(ocs.new_extrusion());
            if insert.is_none()
                && let Some(items) = content
            {
                for it in items.iter_mut() {
                    if !is_transformable(&it.dxftype) {
                        continue;
                    }
                    match transform(it, m) {
                        Ok(()) | Err(Fail::NonUniform) => {}
                        Err(err) => return Err(err),
                    }
                }
            }
        }
        Body::Leader { vertices, horizontal_direction, .. } => {
            for v in vertices.iter_mut() {
                *v = m.transform(*v);
            }
            *horizontal_direction = m.transform_direction(*horizontal_direction);
        }
        Body::Text { insert, align_point, rotation, oblique, width, height, .. } => {
            let ocs = OcsTransform::new(ext, m)?;
            let ins = insert.unwrap_or(NULLVEC);
            let ap = align_point.unwrap_or(ins);
            *insert = Some(ocs.transform_vertex(ins));
            *align_point = Some(ocs.transform_vertex(ap));
            let old = *rotation;
            let new_rot = ocs.transform_deg_angle(old);
            let xs = ocs.transform_length(V3::from_deg_angle(old));
            let mut ys = ocs.transform_length(V3::from_deg_angle(old + 90.0));
            if !ocs.scale_uniform {
                let ov = V3::from_deg_angle(old + 90.0 - *oblique);
                let no = new_rot + 90.0 - ocs.transform_direction(ov).angle_deg();
                *oblique = no;
                ys *= radians(no).cos();
            }
            *width *= div(xs, ys)?;
            *height *= ys;
            *rotation = new_rot;
            if let Some(t) = e.c.thickness {
                e.c.thickness = Some(ocs.transform_thickness(t));
            }
            e.c.extrusion = Some(ocs.new_extrusion());
        }
        Body::MText { insert, text_direction, rotation, char_height, width, .. } => {
            let (new_ext, _) = transform_extrusion(ext, m)?;
            if let Some(r) = rotation.take()
                && text_direction.is_none()
            {
                *text_direction = Some(Ocs::new(ext)?.to_wcs(V3::from_deg_angle(r)));
            }
            let otd = text_direction.unwrap_or(X_AXIS);
            let ntd = m.transform_direction(otd);
            let ovd = ext.cross(otd);
            let och = *char_height;
            let nchv = m.transform_direction(ovd.normalize_to(och)?);
            let obl = ntd.angle_between(nchv)?;
            *char_height = nchv.magnitude() * obl.sin();
            if let Some(w) = width {
                *w = m.transform_direction(otd.normalize_to(*w)?).magnitude();
            }
            *insert = Some(m.transform(insert.unwrap_or(NULLVEC)));
            *text_direction = Some(ntd);
            e.c.extrusion = Some(new_ext);
        }
        Body::Solid { vtx } => {
            let ocs = OcsTransform::new(ext, m)?;
            for v in vtx.iter_mut().flatten() {
                *v = ocs.transform_vertex(*v);
            }
            if let Some(t) = e.c.thickness {
                e.c.thickness = Some(ocs.transform_thickness(t));
            }
            e.c.extrusion = Some(ocs.new_extrusion());
        }
        Body::Hatch { elevation, paths, .. } => {
            let ocs = OcsTransform::new(ext, m)?;
            if !ocs.scale_uniform {
                hatch_to_edges(paths)?;
            }
            let el = elevation.z;
            for p in paths.iter_mut() {
                hatch_path_transform(p, &ocs, el)?;
            }
            *elevation = v3(0.0, 0.0, ocs.transform_vertex(v3(0.0, 0.0, el)).z);
            e.c.extrusion = Some(ocs.new_extrusion());
        }
        Body::Image { insert } => {
            *insert = Some(m.transform(insert.unwrap_or(NULLVEC)));
        }
        Body::Other => {}
    }
    Ok(())
}

fn circle_transform(c: &mut Common, ocs: &OcsTransform, center: &mut V3, radius: &mut f64) -> R<()> {
    if !ocs.scale_uniform {
        return Err(Fail::NonUniform);
    }
    c.extrusion = Some(ocs.new_extrusion());
    *center = ocs.transform_vertex(*center);
    *radius = ocs.transform_length(v3(*radius, 0.0, 0.0));
    if let Some(t) = c.thickness {
        c.thickness = Some(ocs.transform_thickness(t));
    }
    Ok(())
}

fn hatch_to_edges(paths: &mut [BPath]) -> R<()> {
    for p in paths.iter_mut() {
        if let BPath::Poly { vertices, closed } = p
            && vertices.iter().any(|v| v.2 != 0.0)
        {
            let mut vs = vertices.clone();
            if *closed {
                vs.push(vs[0]);
            }
            let mut edges = Vec::new();
            let mut prev: Option<(V3, f64)> = None;
            for (x, y, b) in vs {
                let point = V3::xy(x, y);
                let Some((pp, pb)) = prev else {
                    prev = Some((point, b));
                    continue;
                };
                if pb != 0.0 {
                    let (center, sa, ea, radius) = super::curves::bulge_to_arc(pp, point, pb)?;
                    let start = py_mod(degrees(sa), 360.0);
                    let mut end = py_mod(degrees(ea), 360.0);
                    if isclose_def(start, end) && isclose_def(start, 0.0) {
                        end = 360.0;
                    }
                    edges.push(Edge::Arc { center, radius, start, end });
                } else {
                    edges.push(Edge::Line { start: V2::of(pp), end: V2::of(point) });
                }
                prev = Some((point, b));
            }
            *p = BPath::Edges(edges);
        }
    }
    for p in paths.iter_mut() {
        if let BPath::Edges(edges) = p {
            for e in edges.iter_mut() {
                if let Edge::Arc { center, radius, start, end } = *e {
                    *e = Edge::Ellipse { center, major: V2 { x: radius, y: 0.0 }, ratio: 1.0, start, end };
                }
            }
        }
    }
    Ok(())
}

fn hatch_path_transform(p: &mut BPath, ocs: &OcsTransform, el: f64) -> R<()> {
    match p {
        BPath::Poly { vertices, .. } => {
            for v in vertices.iter_mut() {
                let t = ocs.transform_vertex(v3(v.0, v.1, el));
                *v = (t.x, t.y, v.2);
            }
        }
        BPath::Edges(edges) => {
            for e in edges.iter_mut() {
                match e {
                    Edge::Line { start, end } => {
                        *start = ocs.transform_2d_vertex(*start, el);
                        *end = ocs.transform_2d_vertex(*end, el);
                    }
                    Edge::Arc { center, radius, start, end } => {
                        *center = ocs.transform_2d_vertex(*center, el);
                        *radius = ocs.transform_length(v3(*radius, 0.0, 0.0));
                        if !isclose_def(arc_angle_span_deg(*start, *end), 360.0) {
                            *start = ocs.transform_deg_angle(*start);
                            *end = ocs.transform_deg_angle(*end);
                        } else {
                            *start = ocs.transform_deg_angle(*start);
                            *end = *start + 360.0;
                        }
                    }
                    Edge::Ellipse { center, major, ratio, start, end } => {
                        let sp = angle_to_param(*ratio, radians(*start))?;
                        let ep = angle_to_param(*ratio, radians(*end))?;
                        let mut ce = Ellipse::new(center.v3(), major.v3(), Z_AXIS, *ratio, sp, ep, true)?;
                        ce.center = ocs.old_ocs.to_wcs(ce.center.with_z(el));
                        ce.major_axis = ocs.old_ocs.to_wcs(ce.major_axis);
                        ce.extrusion = ocs.old_extrusion();
                        ce.transform(&ocs.m)?;
                        *center = V2::of(ocs.new_ocs.from_wcs(ce.center));
                        *major = V2::of(ocs.new_ocs.from_wcs(ce.major_axis));
                        *ratio = ce.ratio;
                        *start = degrees(param_to_angle(ce.ratio, ce.start_param));
                        *end = degrees(param_to_angle(ce.ratio, ce.end_param));
                        *start = py_mod(*start, 360.0);
                        *end = py_mod(*end, 360.0);
                        if isclose_def(*end, 0.0) {
                            *end = 360.0;
                        }
                    }
                    Edge::Spline { cps, fits, st, et, .. } => {
                        for v in cps.iter_mut().chain(fits.iter_mut()) {
                            *v = ocs.transform_2d_vertex(*v, el);
                        }
                        for v in [st, et].into_iter().flatten() {
                            *v = V2::of(ocs.transform_direction(v3(v.x, v.y, el)));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn ellipse_from_arc(e: &Entity) -> R<Entity> {
    let (center, radius, s, en) = match &e.body {
        Body::Arc { center, radius, start, end } => (*center, *radius, *start, *end),
        Body::Circle { center, radius } => (*center, *radius, 0.0, 360.0),
        _ => return Err(Fail::Error),
    };
    let ce = Ellipse::from_arc(center, radius, e.c.extrusion(), s, en, true)?.dxfattribs()?;
    let mut c = e.c.clone();
    c.thickness = None;
    c.extrusion = Some(ce.extrusion);
    Ok(Entity {
        dxftype: "ELLIPSE".into(),
        c,
        body: Body::Ellipse {
            center: ce.center,
            major: ce.major_axis,
            ratio: ce.ratio,
            start: ce.start_param,
            end: ce.end_param,
        },
    })
}

fn polyline_virtual(e: &Entity) -> R<Vec<Entity>> {
    let (mut points, elevation, ext): (Vec<(f64, f64, f64)>, f64, Option<V3>) = match &e.body {
        Body::LwPolyline { pts, flags, elevation, .. } => {
            if pts.len() < 2 {
                return Ok(Vec::new());
            }
            let mut p: Vec<_> = pts.iter().map(|q| (q[0], q[1], q[4])).collect();
            if flags & 1 != 0 {
                p.push(p[0]);
            }
            (p, elevation.unwrap_or(0.0), e.c.extrusion)
        }
        Body::Polyline { flags, elevation, vertices } => {
            if vertices.len() < 2 {
                return Ok(Vec::new());
            }
            if flags & 8 != 0 {
                let g = e.c.graphic();
                let start: i64 = if flags & 1 != 0 { -1 } else { 0 };
                let n = vertices.len() as i64;
                return Ok((start..n - 1)
                    .map(|i| {
                        let a = vertices[((i + n) % n) as usize].location;
                        let b = vertices[(i + 1) as usize].location;
                        Entity { dxftype: "LINE".into(), c: g.clone(), body: Body::Line { start: a, end: b } }
                    })
                    .collect());
            }
            if flags & (16 | 64) != 0 {
                return Ok(Vec::new());
            }
            let mut p: Vec<_> = vertices.iter().map(|v| (v.location.x, v.location.y, v.bulge.unwrap_or(0.0))).collect();
            if flags & 1 != 0 {
                p.push(p[0]);
            }
            (p, elevation.unwrap_or(NULLVEC).z, e.c.extrusion)
        }
        _ => return Ok(Vec::new()),
    };
    let ext = ext.filter(|x| !x.is_null());
    let ocs = match ext {
        Some(x) => Ocs::new(x)?,
        None => Ocs::wcs(),
    };
    let g = e.c.graphic();
    let mut out = Vec::new();
    let mut prev: Option<(V3, f64)> = None;
    for (x, y, b) in points.drain(..) {
        let point = v3(x, y, elevation);
        let Some((pp, pb)) = prev else {
            prev = Some((point, b));
            continue;
        };
        if pb != 0.0 {
            let (c, sa, ea, r) = bulge_to_arc(pp, point, pb)?;
            if r > 0.0 {
                let mut cc = g.clone();
                cc.extrusion = ext;
                out.push(Entity {
                    dxftype: "ARC".into(),
                    c: cc,
                    body: Body::Arc {
                        center: v3(c.x, c.y, elevation),
                        radius: r,
                        start: degrees(sa),
                        end: degrees(ea),
                    },
                });
            }
        } else {
            out.push(Entity {
                dxftype: "LINE".into(),
                c: g.clone(),
                body: Body::Line { start: ocs.to_wcs(pp), end: ocs.to_wcs(point) },
            });
        }
        prev = Some((point, b));
    }
    Ok(out)
}

pub type Sink<'a> = dyn FnMut(Entity) + 'a;

fn transform_stream(entities: Vec<Entity>, m: &M44, doc: &Doc, out: &mut Sink) -> R<()> {
    for mut e in entities {
        if !is_transformable(&e.dxftype) {
            continue;
        }
        let saved = e.clone();
        match transform(&mut e, m) {
            Ok(()) => out(e),
            Err(Fail::NonUniform) => match saved.dxftype.as_str() {
                "ARC" | "CIRCLE" => {
                    let r = match saved.body {
                        Body::Arc { radius, .. } | Body::Circle { radius, .. } => radius,
                        _ => 0.0,
                    };
                    if r.abs() > ABS_TOL {
                        let mut el = ellipse_from_arc(&saved)?;
                        transform(&mut el, m)?;
                        out(el);
                    }
                }
                "LWPOLYLINE" | "POLYLINE" => transform_stream(polyline_virtual(&saved)?, m, doc, out)?,
                _ => {}
            },
            Err(Fail::InsertTransform) => {
                let mut inner: Vec<Entity> = Vec::new();
                let mut err = None;
                if let Err(x) = insert_entities(&saved, doc, &mut |v| inner.push(v)) {
                    err = Some(x);
                }
                transform_stream(inner, m, doc, out)?;
                if let Some(x) = err {
                    return Err(x);
                }
            }
            Err(x) => return Err(x),
        }
    }
    Ok(())
}

pub fn insert_matrix(e: &Entity, doc: &Doc) -> R<M44> {
    let Body::Insert { name, insert, sx, sy, sz, rotation } = &e.body else { return Err(Fail::Error) };
    let ocs = Ocs::new(e.c.extrusion())?;
    let ext = ocs.uz();
    let ux = ocs.to_wcs(X_AXIS);
    let uy = ocs.to_wcs(Y_AXIS);
    let mut m = M44::ucs(ux * *sx, uy * *sy, ext * *sz);
    let angle = radians(*rotation);
    if angle != 0.0 {
        m = m.matmul(&M44::axis_rotate(ext, angle)?);
    }
    let mut ins = ocs.to_wcs(insert.unwrap_or(NULLVEC));
    if let Some(b) = doc.block(name) {
        ins = ins - m.transform_direction(b.base_point);
    }
    m.set_row3(ins);
    Ok(m)
}

fn copy_entity(e: &Entity, doc: &Doc) -> R<Option<Entity>> {
    match e.dxftype.as_str() {
        "ACAD_PROXY_ENTITY" | "OLE2FRAME" => Ok(None),
        "DIMENSION" | "ARC_DIMENSION" | "LARGE_RADIAL_DIMENSION" => {
            let mut c = e.clone();
            if let Body::Dimension { insert, content, .. } = &mut c.body
                && content.is_none()
            {
                let mut items = Vec::new();
                dimension_entities(e, doc, &mut |v| items.push(v))?;
                *content = Some(items);
                *insert = None;
            }
            Ok(Some(c))
        }
        _ => Ok(Some(e.clone())),
    }
}

pub fn insert_entities(e: &Entity, doc: &Doc, out: &mut Sink) -> R<()> {
    let m = insert_matrix(e, doc)?;
    let Body::Insert { name, .. } = &e.body else { return Err(Fail::Error) };
    let Some(block) = doc.block(name) else { return Err(Fail::Error) };
    for ent in &block.entities {
        if ent.dxftype == "ATTDEF" {
            continue;
        }
        let Some(copy) = copy_entity(ent, doc)? else { continue };
        transform_stream(vec![copy], &m, doc, out)?;
    }
    Ok(())
}

pub fn dimension_entities(e: &Entity, doc: &Doc, out: &mut Sink) -> R<()> {
    let Body::Dimension { geometry, text_midpoint, insert, content } = &e.body else { return Ok(()) };
    let ocs = Ocs::new(e.c.extrusion())?;
    let elevation = text_midpoint.unwrap_or(NULLVEC).z;
    let m = match insert {
        Some(i) if !i.is_null() => {
            let p = ocs.to_wcs(*i);
            Some(M44::translate(p.x, p.y, p.z))
        }
        _ => None,
    };
    let items: Vec<Entity> = match content {
        Some(c) => c.clone(),
        None => doc.block(geometry.as_deref().unwrap_or("*")).map(|b| b.entities.clone()).unwrap_or_default(),
    };
    for ent in &items {
        let Some(mut copy) = copy_entity(ent, doc)? else { continue };
        if ocs.transform() {
            dim_ocs_to_wcs(&mut copy, &ocs, elevation);
        }
        if let Some(m) = &m {
            if !is_transformable(&copy.dxftype) {
                return Err(Fail::Error);
            }
            transform(&mut copy, m)?;
        }
        out(copy);
    }
    Ok(())
}

fn dim_ocs_to_wcs(e: &mut Entity, ocs: &Ocs, elevation: f64) {
    match &mut e.body {
        Body::Line { start, end } => {
            *start = ocs.to_wcs(start.with_z(elevation));
            *end = ocs.to_wcs(end.with_z(elevation));
        }
        Body::MText { insert, text_direction, rotation, .. } => {
            if let Some(r) = rotation.take()
                && text_direction.is_none()
            {
                let o = Ocs::new(e.c.extrusion()).unwrap_or(Ocs::wcs());
                *text_direction = Some(o.to_wcs(V3::from_deg_angle(r)));
            }
            e.c.extrusion = Some(ocs.uz());
            *text_direction = Some(ocs.to_wcs(text_direction.unwrap_or(X_AXIS)));
            *insert = Some(ocs.to_wcs(insert.unwrap_or(NULLVEC).with_z(elevation)));
        }
        Body::Point { location } => *location = ocs.to_wcs(location.with_z(elevation)),
        body => {
            e.c.extrusion = Some(ocs.uz());
            match body {
                Body::Text { insert: Some(p), .. }
                | Body::Insert { insert: Some(p), .. }
                | Body::Image { insert: Some(p) } => *p = p.with_z(elevation),
                Body::Circle { center, .. } | Body::Arc { center, .. } | Body::Ellipse { center, .. } => {
                    *center = center.with_z(elevation)
                }
                Body::Solid { vtx } if e.dxftype == "SOLID" => {
                    for v in vtx.iter_mut() {
                        *v = Some(v.unwrap_or(NULLVEC).with_z(elevation));
                    }
                }
                _ => {}
            }
        }
    }
}

fn dimvar(doc: &Doc, style: &str, code: i32) -> Option<f64> {
    let ds = doc.dimstyles.get(&key(style))?;
    ds.vars.get(&code).map(|v| match v {
        Val::F(f) => *f,
        Val::I(i) => *i as f64,
        _ => 0.0,
    })
}

fn arrow_rotate_translate(pts: &mut [V3], angle_deg: f64, insert: V3) {
    let a = radians(angle_deg);
    let (c, s) = (a.cos(), a.sin());
    let rot = [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0];
    let tr = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, insert.x, insert.y, 1.0];
    for m in [rot, tr] {
        for p in pts.iter_mut() {
            let row = [p.x, p.y, 1.0];
            let mut r = [0.0; 2];
            for (j, rj) in r.iter_mut().enumerate() {
                let mut acc = 0.0;
                for k in 0..3 {
                    acc = row[k].mul_add(m[k * 3 + j], acc);
                }
                *rj = acc;
            }
            *p = v3(r[0], r[1], 0.0);
        }
    }
}

pub fn leader_entities(e: &Entity, doc: &Doc, out: &mut Sink) -> R<()> {
    let Body::Leader {
        vertices,
        dimstyle,
        path_type,
        annotation_type,
        has_hookline,
        hookline_direction,
        has_arrowhead,
        text_width,
        horizontal_direction,
    } = &e.body
    else {
        return Ok(());
    };
    let mut vertices = vertices.clone();
    if vertices.len() < 2 {
        return Err(Fail::Error);
    }
    let measurement = doc.header_f("$MEASUREMENT").unwrap_or(0.0) != 0.0;
    let dimtad = dimvar(doc, dimstyle, 77).unwrap_or(1.0);
    let dimgap = dimvar(doc, dimstyle, 147).unwrap_or(if measurement { 0.625 } else { 0.0625 });
    let mut dimscale = dimvar(doc, dimstyle, 40).unwrap_or(1.0);
    if dimscale == 0.0 {
        dimscale = 1.0;
    }
    if *annotation_type == 0 && *has_hookline != 0 {
        let mut hv = *horizontal_direction;
        if *hookline_direction == 1 {
            hv = -hv;
        }
        if dimtad != 0.0 && *text_width > 0.0 {
            let last = vertices[vertices.len() - 1];
            vertices.push(last + hv * (dimgap * dimscale + *text_width));
        }
    }
    let mut g = e.c.graphic();
    g.linetype = Some(e.c.linetype.clone().unwrap_or_else(|| "BYLAYER".into()));
    g.lineweight = Some(dimvar(doc, dimstyle, 371).map(|v| v as i64).or(e.c.lineweight).unwrap_or(-1));
    if *path_type == 1 {
        let n = vertices.len();
        let st = vertices[1] - vertices[0];
        let et = vertices[n - 1] - vertices[n - 2];
        let bs = fit_points_to_cad_cv(&vertices, Some((st, et)))?;
        out(Entity {
            dxftype: "SPLINE".into(),
            c: Common::default(),
            body: Body::Spline {
                degree: bs.degree() as i64,
                knot_tol: 1e-10,
                cps: bs.cps.clone(),
                fits: Vec::new(),
                knots: bs.knots.clone(),
                weights: bs.weights.clone(),
                st: None,
                et: None,
            },
        });
    } else {
        for w in vertices.windows(2) {
            out(Entity { dxftype: "LINE".into(), c: g.clone(), body: Body::Line { start: w[0], end: w[1] } });
        }
    }
    let custom_arrow = doc.dimstyles.get(&key(dimstyle)).is_some_and(|d| d.vars.contains_key(&341));
    if *has_arrowhead != 0 && !custom_arrow {
        let size = dimvar(doc, dimstyle, 41).unwrap_or(if measurement { 2.5 } else { 0.1875 }) * dimscale;
        let rotation = (vertices[0] - vertices[1]).angle_deg();
        let h = radians(18.924644 / 2.0).sin() * size;
        let mut shape = [v3(-size, h, 0.0), NULLVEC, v3(-size, -h, 0.0)];
        arrow_rotate_translate(&mut shape, rotation, vertices[0]);
        out(Entity {
            dxftype: "SOLID".into(),
            c: g,
            body: Body::Solid { vtx: [Some(shape[0]), Some(shape[1]), Some(shape[2]), Some(shape[2])] },
        });
    }
    Ok(())
}

pub fn make_path(e: &Entity) -> R<Path> {
    let ext = e.c.extrusion();
    match &e.body {
        Body::Line { start, end } => {
            let mut p = Path::new(*start);
            p.line_to(*end);
            Ok(p)
        }
        Body::Circle { center, radius } => arc_path(*center, *radius, ext, 0.0, 360.0),
        Body::Arc { center, radius, start, end } => arc_path(*center, *radius, ext, *start, *end),
        Body::Ellipse { center, major, ratio, start, end } => {
            let ce = Ellipse::new(*center, *major, ext, *ratio, *start, *end, true)?;
            let mut p = Path::new(NULLVEC);
            add_ellipse(&mut p, &ce, 1)?;
            Ok(p)
        }
        Body::LwPolyline { pts, flags, elevation, .. } => {
            let mut p = Path::new(NULLVEC);
            let points: Vec<_> = pts.iter().map(|q| (q[0], q[1], q[4])).collect();
            add_2d_polyline(&mut p, &points, flags & 1 != 0, &Ocs::new(ext)?, elevation.unwrap_or(0.0))?;
            Ok(p)
        }
        Body::Polyline { flags, elevation, vertices } => {
            if flags & (16 | 64) != 0 {
                return Err(Fail::Error);
            }
            if vertices.is_empty() {
                return Ok(Path::new(NULLVEC));
            }
            if flags & 8 != 0 {
                let pts: Vec<V3> = vertices.iter().map(|v| v.location).collect();
                return Ok(from_vertices(&pts, flags & 1 != 0));
            }
            let points: Vec<_> =
                vertices.iter().map(|v| (v.location.x, v.location.y, v.bulge.unwrap_or(0.0))).collect();
            let el = match elevation {
                Some(p) => p.z,
                None => vertices[0].location.z,
            };
            let mut p = Path::new(NULLVEC);
            add_2d_polyline(&mut p, &points, flags & 1 != 0, &Ocs::new(ext)?, el)?;
            Ok(p)
        }
        Body::Spline { degree, knot_tol, cps, fits, knots, weights, st, et } => {
            let bs = if !cps.is_empty() {
                let w = (!weights.is_empty()).then(|| weights.clone());
                let k = (!knots.is_empty()).then(|| round_knots(knots, *knot_tol));
                let order = usize::try_from(*degree + 1).map_err(|_| Fail::Error)?;
                BSpline::new(cps.clone(), order, k, w)?
            } else if !fits.is_empty() {
                let t = match (st, et) {
                    (Some(a), Some(b)) => Some((*a, *b)),
                    _ => None,
                };
                fit_points_to_cad_cv(fits, t)?
            } else {
                return Err(Fail::Error);
            };
            let mut p = Path::new(NULLVEC);
            add_spline(&mut p, &bs)?;
            Ok(p)
        }
        _ => Err(Fail::Error),
    }
}

fn arc_path(center: V3, radius: f64, ext: V3, start: f64, end: f64) -> R<Path> {
    let mut p = Path::new(NULLVEC);
    let r = radius.abs();
    if r > 1e-12 {
        let e = Ellipse::from_arc(center, r, ext, start, end, true)?;
        add_ellipse(&mut p, &e, 1)?;
    }
    Ok(p)
}

fn edge_path(edges: &[Edge], ocs: &Ocs, elevation: f64) -> R<Path> {
    let extrusion = ocs.uz();
    let wcs = |x: f64, y: f64| {
        let v = v3(x, y, elevation);
        if ocs.transform() { ocs.to_wcs(v) } else { v }
    };
    let wcs_t = |x: f64, y: f64| {
        let v = v3(x, y, 0.0);
        if ocs.transform() { ocs.to_wcs(v) } else { v }
    };
    let mut path = Path::new(NULLVEC);
    let mut lp: Option<Path> = None;
    for edge in edges {
        let seg = match edge {
            Edge::Line { start, end } => {
                let mut s = Path::new(wcs(start.x, start.y));
                s.line_to(wcs(end.x, end.y));
                Some(s)
            }
            Edge::Arc { center, radius, start, end } => {
                if radius.abs() > ABS_TOL {
                    let e =
                        Ellipse::from_arc(v3(center.x, center.y, elevation), *radius, extrusion, *start, *end, true)?;
                    let mut s = Path::new(NULLVEC);
                    add_ellipse(&mut s, &e, 1)?;
                    Some(s)
                } else {
                    None
                }
            }
            Edge::Ellipse { center, major, ratio, start, end } => {
                if !major.is_null() {
                    let sp = angle_to_param(*ratio, radians(*start))?;
                    let ep = angle_to_param(*ratio, radians(*end))?;
                    Ellipse::new(center.v3(), major.v3(), Z_AXIS, *ratio, sp, ep, true)?;
                    let e = Ellipse::new(
                        wcs(center.x, center.y),
                        wcs_t(major.x, major.y),
                        extrusion,
                        *ratio,
                        sp,
                        ep,
                        true,
                    )?;
                    let mut s = Path::new(NULLVEC);
                    add_ellipse(&mut s, &e, 1)?;
                    Some(s)
                } else {
                    None
                }
            }
            Edge::Spline { degree, knots, weights, cps, fits, st, et } => {
                let cp: Vec<V3> = cps.iter().map(|p| wcs(p.x, p.y)).collect();
                let bs = if cp.is_empty() {
                    let fp: Vec<V3> = fits.iter().map(|p| wcs(p.x, p.y)).collect();
                    if fp.is_empty() {
                        None
                    } else {
                        let t = match (st, et) {
                            (Some(a), Some(b)) if !a.is_null() && !b.is_null() => {
                                Some((wcs_t(a.x, a.y), wcs_t(b.x, b.y)))
                            }
                            _ => None,
                        };
                        Some(fit_points_to_cad_cv(&fp, t)?)
                    }
                } else {
                    let order = usize::try_from(*degree + 1).map_err(|_| Fail::Error)?;
                    let w = (!weights.is_empty()).then(|| weights.clone());
                    Some(BSpline::new(cp, order, Some(knots.clone()), w)?)
                };
                match bs {
                    Some(bs) => {
                        let mut s = Path::new(NULLVEC);
                        add_spline(&mut s, &bs)?;
                        Some(s)
                    }
                    None => None,
                }
            }
        };
        let Some(next) = seg else { continue };
        let Some(mut lo) = lp.take() else {
            lp = Some(next);
            continue;
        };
        if lo.end().isclose(next.start()) {
            lo.append_path(&next);
        } else if lo.end().isclose(next.end()) {
            lo.append_path(&next.reversed());
        } else if lo.start().isclose(next.end()) {
            let mut n = next;
            n.append_path(&lo);
            lo = n;
        } else if lo.start().isclose(next.start()) {
            lo = lo.reversed();
            lo.append_path(&next);
        } else if lo.is_closed() {
            path.extend_multi_path(&lo);
            lo = next;
        } else {
            lo.append_path(&next);
        }
        lp = Some(lo);
    }
    if let Some(mut lo) = lp {
        lo.close();
        path.extend_multi_path(&lo);
    }
    Ok(path)
}

pub fn hatch_paths(e: &Entity) -> R<Vec<Path>> {
    let Body::Hatch { elevation, paths, .. } = &e.body else { return Err(Fail::Error) };
    let ocs = Ocs::new(e.c.extrusion())?;
    let el = elevation.z;
    let mut out = Vec::new();
    for b in paths {
        let p = match b {
            BPath::Poly { vertices, closed } => {
                let mut p = Path::new(NULLVEC);
                add_2d_polyline(&mut p, vertices, *closed, &ocs, el)?;
                p
            }
            BPath::Edges(edges) => edge_path(edges, &ocs, el)?,
        };
        if p.has_sub_paths {
            out.extend(p.sub_paths());
        } else {
            out.push(p);
        }
    }
    Ok(out)
}
