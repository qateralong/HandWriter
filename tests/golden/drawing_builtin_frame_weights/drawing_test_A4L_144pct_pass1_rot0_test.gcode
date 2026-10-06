; HandWriter gcode
; DRAWING: builtin:test (test)
; TEST FILE for PASS 1/1: rotate sheet 0 deg CCW, corner A (bottom left in layout) at the stops = zero
; TEST: available part of the sheet (solid), this pass cell (dashed), corner mark X long / Y short with pass number ticks, control crosses
; run order: 1) drawing_test_A4L_144pct_pass1_rot0_test.gcode
; zero correction dx 0.00 dy 0.00 mm (pass coordinates)
; scale 1.44:1 (144.40%), mode fit_reach
; sheet A4 landscape 297.00x210.00 mm (in this pass 297.00x210.00 along X x Y), drawing origin offset X40.48 Y211.92
; table: sheet may overhang +X 1 +Y 0; table edge X300.00 Y300.00 (= reach window edge)
; frame GOST 2.104 form 1: L20.00 R5.00 T5.00 B5.00, title block 185.00x55.00
; line weights: thick > 0.40 mm drawn in 3 passes, step 0.15 mm
; curves 0.05 mm, join 0.05 mm, long paths first >= 30.00 mm
; pen up Z4.00 down Z-1.00, end Z14.00, feed draw 1200 travel 3000 z 600, simplify 0.05
; travel X-2.00..300.00 Y-2.00..300.00, flip_x 0 flip_y 0
; strokes 6, draw 1056 mm, travel 55 mm, est 1.0 min
G21
G90
M104 S0
M140 S0
M420 S0
M211 S0
G92 X0 Y0 Z0
G0 Z4.00 F600
G0 X0.00 Y0.00 F3000
G1 Z-1.00 F600
G1 X297.00 Y0.00 F1200
G1 X297.00 Y210.00
G1 X0.00 Y210.00
G1 X0.00 Y0.00
G0 Z4.00 F600
G0 X5.00 Y5.00 F3000
G1 Z-1.00 F600
G1 X25.00 Y5.00 F1200
G0 Z4.00 F600
G0 X23.00 Y6.20 F3000
G1 Z-1.00 F600
G1 X25.00 Y5.00 F1200
G1 X23.00 Y3.80
G0 Z4.00 F600
G0 X5.00 Y5.00 F3000
G1 Z-1.00 F600
G1 X5.00 Y15.00 F1200
G0 Z4.00 F600
G0 X3.80 Y13.00 F3000
G1 Z-1.00 F600
G1 X5.00 Y15.00 F1200
G1 X6.20 Y13.00
G0 Z4.00 F600
G0 X30.00 Y3.50 F3000
G1 Z-1.00 F600
G1 X30.00 Y6.50 F1200
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
