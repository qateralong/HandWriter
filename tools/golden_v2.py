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
    _patch(pipeline._travel_on_sheet, [("    if cur != start:\n        moves.append((cur, start))\n", "")])
