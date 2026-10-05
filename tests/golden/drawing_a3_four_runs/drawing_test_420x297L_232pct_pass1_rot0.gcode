; HandWriter gcode
; DRAWING: builtin:test (test)
; PASS 1/4: sheet on the printer marks, rotated 0 deg, pencil on the zero line
; run order: 1) drawing_test_420x297L_232pct_pass1_rot0.gcode, 2) drawing_test_420x297L_232pct_pass2_rot180.gcode, 3) drawing_test_420x297L_232pct_pass3_rot90.gcode, 4) drawing_test_420x297L_232pct_pass4_rot270.gcode
; zero correction dx 0.00 dy 0.00 mm (pass coordinates)
; scale 2.32:1 (232.00%), mode fit
; seams (layout mm): y = 148.50; x = 234.50; x = 185.50; overlap 0.50 mm along lines
; sheet custom landscape 420.00x297.00 mm (in this pass 420.00x297.00 along X x Y), drawing origin offset X31.90 Y303.60
; table: sheet may overhang +X 1 +Y 0; table edge X220.00 Y220.00 (= reach window edge)
; frame GOST 2.104 form 1: L20.00 R5.00 T5.00 B5.00, title block 185.00x55.00
; line weights: off (single pass for all lines)
; curves 0.05 mm, join 0.05 mm, long paths first >= 30.00 mm
; pen up Z4.00 down Z-1.00, feed draw 1200 travel 3000 z 600, simplify 0.05
; travel X0.00..239.00 Y0.00..190.00, flip_x 0 flip_y 0
; strokes 7, draw 961 mm, travel 524 mm, est 1.1 min
G21
G90
M104 S0
M140 S0
M420 S0
M211 S0
G92 X0 Y0 Z0
G0 Z4.00 F600
G0 X90.03 Y48.75 F3000
G1 Z-1.00 F600
G1 X90.08 Y48.77 F1200
G1 X93.85 Y50.05
G1 X97.71 Y51.14
G1 X101.64 Y52.05
G1 X105.63 Y52.76
G1 X109.70 Y53.28
G1 X113.82 Y53.59
G1 X118.00 Y53.70
G1 X122.18 Y53.59
G1 X126.30 Y53.28
G1 X130.37 Y52.76
G1 X134.36 Y52.05
G1 X138.29 Y51.14
G1 X142.15 Y50.05
G1 X145.92 Y48.77
G1 X149.61 Y47.32
G1 X153.20 Y45.69
G1 X156.70 Y43.90
G1 X160.11 Y41.94
G1 X163.40 Y39.83
G1 X166.58 Y37.57
G1 X169.65 Y35.16
G1 X172.60 Y32.61
G1 X175.42 Y29.92
G1 X178.11 Y27.10
G1 X180.66 Y24.15
G1 X183.07 Y21.08
G1 X185.33 Y17.90
G1 X187.44 Y14.61
G1 X189.40 Y11.20
G1 X191.19 Y7.70
G1 X192.82 Y4.11
G1 X192.84 Y4.04
G0 Z4.00 F600
G0 X139.06 Y4.08 F3000
G1 Z-1.00 F600
G1 X234.00 Y146.50 F1200
G0 Z4.00 F600
G0 X90.00 Y146.50 F3000
G1 Z-1.00 F600
G1 X234.00 Y146.50 F1200
G1 X234.00 Y4.00
G1 X234.00 Y170.00
G1 X90.00 Y170.00
G0 Z4.00 F600
G0 X118.00 Y76.90 F3000
G1 Z-1.00 F600
G1 X118.00 Y49.06 F1200
G0 Z4.00 F600
G0 X118.00 Y42.10 F3000
G1 Z-1.00 F600
G1 X118.00 Y39.78 F1200
G0 Z4.00 F600
G0 X118.00 Y32.82 F3000
G1 Z-1.00 F600
G1 X118.00 Y4.98 F1200
G0 Z4.00 F600
G0 X96.94 Y4.08 F3000
G1 Z-1.00 F600
G1 X90.22 Y14.17 F1200
G0 Z4.00 F600
G0 Z4.00 F600
G0 X0.00 Y0.00 F3000
M400
