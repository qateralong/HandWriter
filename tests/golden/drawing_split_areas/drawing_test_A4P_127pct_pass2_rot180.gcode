; HandWriter gcode
; DRAWING: builtin:test (test)
; PASS 2/2: rotate sheet 180 deg CCW, corner C (top right in layout) at the stops = zero
; run order: 1) drawing_test_A4P_127pct_pass1_rot0.gcode, 2) drawing_test_A4P_127pct_pass2_rot180.gcode
; zero correction dx 0.00 dy 0.00 mm (pass coordinates)
; scale 1.27:1 (126.67%), mode fit
; seams (layout mm): x = 159.50; overlap 0.50 mm along lines
; sheet A4 portrait 210.00x297.00 mm (in this pass 210.00x297.00 along X x Y), drawing origin offset X3.67 Y218.17
; table: sheet may overhang +X 1 +Y 0; table edge X200.00 Y215.00 (= reach window edge)
; frame: off
; line weights: off (single pass for all lines)
; curves 0.05 mm, join 0.05 mm, long paths first >= 30.00 mm
; pen up Z4.00 down Z-1.00, end Z14.00, feed draw 1200 travel 3000 z 600, simplify 0.05
; travel X-3.00..200.00 Y-3.00..215.00, flip_x 0 flip_y 0
; strokes 3, draw 307 mm, travel 140 mm, est 0.4 min
G21
G90
M104 S0
M140 S0
M420 S0
M211 S0
G92 X0 Y0 Z0
G0 Z4.00 F600
G0 X10.00 Y85.17 F3000
G1 Z-1.00 F600
G1 X50.92 Y112.45 F1200
G0 Z4.00 F600
G0 X51.00 Y85.17 F3000
G1 Z-1.00 F600
G1 X10.00 Y85.17 F1200
G1 X10.00 Y211.83
G1 X51.00 Y211.83
G0 Z4.00 F600
G0 X50.92 Y184.55 F3000
G1 Z-1.00 F600
G1 X10.00 Y211.83 F1200
G0 Z4.00 F600
G0 Z14.00 F600
M400
G4 P100
G4 P100
G4 P100
G4 P100
G4 P100
G4 P100
G4 P100
G4 P100
