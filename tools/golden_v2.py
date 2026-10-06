from __future__ import annotations

import inspect
import textwrap

END_LIFT = "getattr(pr, 'end_lift', 10.0)"
END_PAD = '["G4 P100"] * 8'
RETURN = 'return "\\n".join(lines) + "\\n"'
PADDED = f'return "\\n".join(lines + {END_PAD}) + "\\n"'


def _patch(func, replacements) -> None:
    src = textwrap.dedent(inspect.getsource(func))
    for old, new in replacements:
        if old not in src:
            raise RuntimeError(f"{func.__name__}: fragment not found: {old!r}")
        src = src.replace(old, new)
    ns: dict = {}
    exec(compile(src, f"<v2 {func.__name__}>", "exec"), func.__globals__, ns)
    func.__code__ = ns[func.__name__].__code__


def _pass_offset(ds, r):
    off = ds.split.offsets or {}
    v = off.get(str(r), off.get(r))
    return (float(v[0]), float(v[1])) if v is not None else (0.0, 0.0)


def _rotation_rects_off(W, H, printer, ds):
    from handwriter.drawing import passes as P
    allowed, rects, notes = [], {}, []
    for r in P.ROTATIONS:
        tb = P.make_table(printer, W, H, r)
        ok, why = P.rotation_allowed(r, W, H, tb)
        if not ok:
            notes.append(f"{r}°: {why}")
            continue
        dx, dy = _pass_offset(ds, r)
        safe = (tb.safe[0] - dx, tb.safe[1] - dy, tb.safe[2] - dx, tb.safe[3] - dy)
        R = P.pass_rect(r, W, H, safe)
        if R is None:
            notes.append(f"{r}°: окно достижимости не заходит на лист")
            continue
        allowed.append(r)
        rects[r] = R
    return allowed, rects, notes


def _a3_rects_off(z, W, H, ds):
    from handwriter.drawing import passes as P
    allowed, rects, notes = [], {}, []
    for i, r in enumerate(P.A3_RUNS):
        m = P.a3_affine(z, W, H, r)
        dx, dy = _pass_offset(ds, r)
        q = [P.invert_affine(m, p) for p in ((z.x_min - dx, z.y_min - dy), (z.x_max - dx, z.y_max - dy))]
        R = P.intersect((min(q[0][0], q[1][0]), min(q[0][1], q[1][1]), max(q[0][0], q[1][0]), max(q[0][1], q[1][1])),
                        (0.0, 0.0, W, H))
        if R is None:
            notes.append(f"заход {i + 1}: зона не заходит на лист")
            continue
        allowed.append(r)
        rects[r] = R
    return allowed, rects, notes


def _marked_rects_off(mk, W, H, ds):
    from handwriter.drawing import passes as P
    allowed, rects, notes = [], {}, []
    for r in (0, 180):
        m = P.marked_affine(mk, W, H, r)
        _, dy = _pass_offset(ds, r)
        A, B, C = m[2], m[3], m[5] - (mk.y_min - dy)
        if abs(B) >= abs(A):
            bounds = [(-C - A * u) / B for u in (0.0, W)]
            R = (0.0, max(0.0, max(bounds)), W, H) if B > 0 else (0.0, 0.0, W, min(H, min(bounds)))
        else:
            bounds = [(-C - B * v) / A for v in (0.0, H)]
            R = (max(0.0, max(bounds)), 0.0, W, H) if A > 0 else (0.0, 0.0, min(W, min(bounds)), H)
        if R[3] - R[1] <= P.EPS or R[2] - R[0] <= P.EPS:
            notes.append(f"{r}°: лист не заходит выше линии нуля")
            continue
        allowed.append(r)
        rects[r] = R
    return allowed, rects, notes


def apply() -> None:
    from handwriter import calibration, gcode
    from handwriter.drawing import pipeline

    _patch(gcode.travel_moves, [("    if cur != (0.0, 0.0):\n        moves.append((cur, (0.0, 0.0)))\n", "")])
    _patch(gcode.compute_stats, [("+ 2 * abs(pr.pen_up_z) / pr.feed_z)",
                                  f"+ (abs(pr.pen_up_z) + {END_LIFT}) / pr.feed_z)")])
    _patch(gcode.generate_gcode, [
        ('f"; pen up Z{up} down Z{down}, feed', f'f"; pen up Z{{up}} down Z{{down}}, end Z{{fmt(pr.pen_up_z + {END_LIFT})}}, feed'),
        ('        f"G0 Z{up} F{fz}",\n        f"G0 X0.00 Y0.00 F{ft}",\n',
         f'        f"G0 Z{{fmt(pr.pen_up_z + {END_LIFT})}} F{{fz}}",\n'),
        (RETURN, PADDED),
    ])
    _patch(calibration.make_reach_check_gcode, [
        ('lines += [f"G0 Z{up} F{fz}", f"G0 X0.00 Y0.00 F{ft}", "M400"]',
         f'lines += [f"G0 Z{{fmt(pr.pen_up_z + {END_LIFT})}} F{{fz}}", "M400"]'),
        (RETURN, PADDED),
    ])
    _patch(calibration.make_zero_gcode, [(RETURN, PADDED)])
    pipeline._v2_rotation_rects = _rotation_rects_off
    pipeline._v2_a3_rects = _a3_rects_off
    pipeline._v2_marked_rects = _marked_rects_off
    _patch(pipeline.compose_drawing, [
        ("a3_rects(ds.a3, lay_.width, lay_.height)", "_v2_a3_rects(ds.a3, lay_.width, lay_.height, ds)"),
        ("marked_rects(ds.marked, lay_.width, lay_.height)", "_v2_marked_rects(ds.marked, lay_.width, lay_.height, ds)"),
        ("rotation_rects(lay_.width, lay_.height, s.printer)", "_v2_rotation_rects(lay_.width, lay_.height, s.printer, ds)"),
    ])
    _patch(pipeline.part_test_strokes, [
        ("        fixed.append(rect_pts(reach))", "        fixed.append([(p[0] + part.dx, p[1] + part.dy) for p in rect_pts(reach)])"),
    ])
    _patch(pipeline._travel_on_sheet, [("    if cur != start:\n        moves.append((cur, start))\n", "")])
