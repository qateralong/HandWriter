; HandWriter gcode
; DRAWING: builtin:test (test)
; PASS 1/2: sheet on the printer marks, rotated 0 deg, pencil on the zero line
; run order: 1) drawing_test_297x197L_100pct_pass1_rot0.gcode, 2) drawing_test_297x197L_100pct_pass2_rot180.gcode
; zero correction dx 0.00 dy 0.00 mm (pass coordinates)
; scale 1:1 (100.00%), mode fit
; seams (layout mm): x = 110.75; overlap 0.50 mm along lines
; sheet custom landscape 297.00x197.00 mm (in this pass 297.00x197.00 along X x Y), drawing origin offset X20.00 Y5.00
; table: sheet may overhang +X 1 +Y 0; table edge X220.00 Y220.00 (= reach window edge)
; frame: off
; line weights: off (single pass for all lines)
; curves 0.05 mm, join 0.05 mm, long paths first >= 30.00 mm
; pen up Z4.00 down Z-1.00, end Z14.00, feed draw 1200 travel 3000 z 600, simplify 0.05
; travel X0.00..209.00 Y0.00..189.00, flip_x 0 flip_y 0
; strokes 3, draw 591 mm, travel 296 mm, est 0.7 min
G21
G90
M104 S0
M140 S0
M420 S0
M211 S0
G92 X0 Y0 Z0
G0 Z4.00 F600
G0 X17.00 Y77.75 F3000
G1 Z-1.00 F600
G1 X17.00 Y169.00 F1200
G1 X204.00 Y169.00
G1 X204.00 Y77.75
G0 Z4.00 F600
G0 X141.32 Y77.83 F3000
G1 Z-1.00 F600
G1 X204.00 Y169.00 F1200
G0 Z4.00 F600
G0 X79.68 Y77.83 F3000
G1 Z-1.00 F600
G1 X17.00 Y169.00 F1200
G0 Z4.00 F600
G0 Z14.00 F600
M400
