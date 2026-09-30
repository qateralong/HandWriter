# HandWriter

## What it is

HandWriter turns an ordinary 3D printer into a “hand with a pencil”. Instead of printing plastic, the printer moves a pencil over a sheet of paper and:

- **writes text by hand** — in any handwriting font, with natural irregularities: letters vary slightly in size and slant, the line drifts a little, and the same letter looks different each time;
- **draws technical drawings** — from SVG, DXF (AutoCAD and similar), PDF files or a PNG/JPG image, with a GOST frame, thick and thin lines, dashed and center lines.

The program does not send anything to the printer by itself. It prepares a **job file** (gcode: a list of commands like “lower the pencil, move here, lift it”). You copy the file to an SD card and start it from the printer screen. The printer never heats up.

Made for the Flying Bear Ghost 5, but any printer with Marlin firmware will do.

There are two builds with the same features: one with a **Russian** interface and one with an **English** interface. Download the one you prefer from Releases.

## How it works on the printer (the essentials)

- **The pencil** is mounted next to the nozzle on a spring and sits slightly below it. The printer lifts and lowers it with the Z axis.
- **The sheet** lies on the table and rests against the **stops** — a corner piece in one corner of the table. This way the sheet is always in the same place.
- **Zero.** Before every run you put the pencil into the sheet corner (the one at the stops) by hand and lower it until it touches the paper. The file starts with the command “this point is zero”, and all coordinates are measured from this corner. Setting it by eye is accurate to about 1 mm, and the program takes that into account.
- **Reach window.** Because of how the pencil is mounted, it cannot reach the whole table. The rectangle within which it can move without hitting the frame is the reach window. You measure it once (there is a wizard), and the program never lets the pencil go outside it (with a 2 mm margin).
- **Passes.** If the sheet is larger than the window, it is drawn in several runs. You draw one part, rotate the sheet (by 90, 180 or 270°), put another corner against the stops and run the next file. Each run is a **pass**. The program decides how many passes are needed, how to rotate the sheet and where to place the **seam** — the line that divides the work between passes.

## Starting the program

1. Unpack the archive and copy the whole `HandWriter` folder (with `HandWriter.exe` and the `_internal` folder) anywhere, for example to `C:\Programs\HandWriter`. Nothing needs to be installed.
2. Run `HandWriter.exe`. The program window opens: it is Chrome or Edge in app mode, without an address bar.
3. Close the window — the program exits by itself after 15 seconds.

Windows may warn “Windows protected your PC” → “More info” → “Run anyway”. The first start takes a few seconds.

At the top left there are two tabs: **Handwriting** (text) and **Drawing** (drawings). All settings are saved automatically and will be the same next time.

## The first time: where to start

1. **Set the pencil height** (the Printer block): `pen_up_z` is how high to lift it, `pen_down_z` is how hard to press. The defaults usually work.
2. **Measure the reach window**: the Drawing tab → Table and passes → Reach calibration wizard…. The wizard guides you step by step and gives you a check file. The pencil makes dots in the four corners of the window, so you can see it never hits the frame.
3. **Download the test file** (the Handwriting tab, Download test file) and run it on a scrap sheet. It draws the frame of the writing area and two arrows near the corner: a long one along the X axis and a short one along Y. This shows the printer moves the right way and the margins are where they should be.
4. After that you can write and draw.

---

## The Handwriting tab

Settings are on the left, the sheet preview is on the right: exactly how the text will look. The mouse wheel zooms, dragging moves the sheet.

### Font

| Setting | What it is |
|---|---|
| Font list | Which handwriting to use. Two are built in: “Hershey” (single-line) and “Bad Script” (handwritten). |
| **Strokes / Outlines** | How the font is made. **Strokes**: the letters are already drawn with lines, like a pen (SVG fonts, an SVG folder). **Outlines**: an ordinary Windows font (TTF/OTF) where letters are filled; the program finds their “middle line” and moves the pencil along it. |
| Load font… | Add your own `.ttf`, `.otf` or SVG font. |
| SVG folder… | Add a font from a folder of letter images (`a.svg`, `b.svg`… and variants `a.2.svg`, `a.3.svg`). |
| Glyph debugging ↗ | A separate page: view one letter or a word large and tune the “Outlines → lines” settings. |

Below the list there is information about the font: how many letters it has and whether it has letter variants and ligatures (joined letter pairs).

### Outlines → lines

Shown only for “Outlines” fonts. It controls how a filled letter is turned into a line for the pencil. It is easier to tune on the “Glyph debugging” page.

| Setting | What it does | Default |
|---|---|---|
| Branch threshold | Trims short “spurs” that appear on thick letters. Higher is cleaner, but short tails may disappear. | 0.08 |
| End extension | Extends line ends to the edge of the letter. 0 leaves the ends shortened. | 1.0 |
| Smoothing | Makes lines smoother. If sharp corners get too rounded, reduce it. | 0.03 |
| Simplification | Removes extra points. Higher gives a smaller file but a coarser line. | 0.004 |
| Merge close junctions | Where lines of a letter cross, helps to go straight through the crossing instead of breaking the line. | 2.5 |
| Rasterization resolution | Accuracy of letter processing. Higher is more accurate but slower. | 1500 |

### Text

| Setting | What it is |
|---|---|
| Text box | What to write. **Tab** at the start of a line is an indent. **Enter** starts a new paragraph. **An empty line** skips a line on the sheet. |
| Load .txt… | Take the text from a file (UTF-8 encoding). |
| Auto hyphenation | Break long words by syllables (with a hyphen). When off, the whole word moves to the next line. |

If the font lacks a character, a list of such characters with their positions appears under the text. For each one choose “skip” or “replace with…”, otherwise no file is created. Guillemets, the long dash and the ellipsis are replaced with simple characters automatically if the font does not have them.

### Sheet start and resuming

| Setting | What it is |
|---|---|
| Sheet starts at word | The word of the text to start this sheet with. When the text does not fit, the program says at the bottom “start the next sheet from word N” — enter that number for the next sheet. |
| Resume from: word, letter | If the pencil broke or the sheet moved in the middle of the work: the sheet is laid out exactly as before, and the file contains only what is left, starting from this letter. The easiest way is to **click the letter in the preview** — the numbers are filled in automatically. 0 means the whole sheet. |
| Write the whole sheet | Reset “resume from”. |

When resuming, what is already written is shown in gray.

### Randomness

Makes the text look handwritten. Everything “random” depends on one number, the **seed**: the same seed gives exactly the same text; another seed gives the same style but different details.

| Setting | What it does | Default |
|---|---|---|
| On | Main switch. | on |
| seed, 🎲 | The randomness number; the die picks a new one. | 1 |
| Random letter variants | If the font has several shapes of a letter, alternate them without repeating the same one in a row. | on |
| Size of each letter, ±% | Letters slightly larger or smaller. The mean size stays as set. | 3 |
| Slant of each letter, ±° | A slight spread of slant. | 2 |
| Vertical offset of letters, ±mm | Letters slightly above or below the line. | 0.3 |
| Letter spacing, ±% | Uneven gaps between letters. | 5 |
| Word spacing, ±% | Uneven gaps between words. | 10 |
| Baseline drift, ±mm | The line gently waves up and down, like real handwriting. | 0.5 |
| Line start, ±mm | Lines do not start perfectly aligned at the margin. | 1 |
| Right edge, ±mm | Lines do not end perfectly aligned. | 1.5 |
| Line jitter, mm | A slight unevenness of the line itself. | 0.1 |
| Defaults | Reset all sliders. | |

### Joined writing

| Setting | What it does | Default |
|---|---|---|
| Join letters | If the end of one letter is close to the start of the next, they are joined by a smooth line without lifting the pencil. | on |
| Join if the end and the start are closer than… | How close the ends must be to join them (fraction of the x-height). For fonts that are not very cursive, use 0.3–0.35. | 0.15 |

### Sheet

| Setting | What it is | Default |
|---|---|---|
| Profile | Ready-made sheet settings: “Grid notebook”, “Ruled notebook”, “A4”. | Grid notebook |
| Save to profile / New… / Delete / Standard | Save your settings, create a profile, delete it, restore the three standard ones. | |
| Width, height, mm | Sheet size. | 165 × 205 |
| Left margin, right margin | Distances from the edges; the text goes between them. | 20 and 8 |
| Top to 1st baseline | Where the first line is: the distance from the top edge to the line the letters “stand” on. | 15 |
| Bottom limit (from bottom) | No lines are written below this height. | 10 |
| Line spacing | Distance between lines. | 10 |
| Indent | Paragraph indent (Tab). | 10 |
| Ruling, grid step | Preview only: show a grid or lines, like a notebook page. | grid, 5 |

### Size and adjustment

| Setting | What it is | Default |
|---|---|---|
| x-height, mm | Handwriting size: the height of the small letter “x”. | 3 |
| Baseline shift, mm | Raise all lines above the notebook lines (plus is up). | 0 |
| Correction dx, dy, mm | Shift all the text if the sheet is placed slightly off. | 0 |
| Text rotation, ° | Rotate all the text if the sheet lies slightly askew (plus is counterclockwise). | 0 |

### Printer (shared by both tabs)

| Setting | What it is | Default |
|---|---|---|
| pen_up_z | How many mm to lift the pencil above the paper during travel moves. Below 2 it may drag on the paper. | 4 |
| pen_down_z | How far to lower the pencil when writing. 0 is a light touch, negative presses harder (spring). Below −3 is too hard. | −1 |
| feed_draw | Writing speed, mm/min. Lower gives a neater line. | 1200 |
| feed_travel | Speed of travel moves with the pencil lifted. | 3000 |
| feed_z | Speed of lifting and lowering the pencil. | 600 |
| Simplification tolerance, mm | How much lines may be straightened to keep the file small. 0.05 mm is invisible on paper. | 0.05 |
| Margin to reach limits | Do not bring the pencil closer than this to the edge of the reach window. | 2 |
| Test arrows from zero | Where the axis arrows of the test file start (offset from the corner). | 5 |
| flip_x, flip_y | Only if the printer moves the wrong way along an axis (you will see it with the test file). Usually off. | off |
| Pencil reach measured, x_min … y_max | The reach window: how far the pencil can move from the sheet corner. These are the same numbers as on the Drawing tab; it is easier to enter them with the wizard there. Until measured, the program uses the sheet itself as the window and warns about it. | not measured |

### Buttons and preview

- **Download gcode** — the file for the printer. Its name tells which words it contains (for example `handwriter_w1-58.gcode`). In Chrome and Edge you can save it straight to the SD card.
- **Download test file** — the frame of the writing area and the axis arrows. **Show test** puts it over the preview.
- Above the sheet: **Fit** (show the whole sheet), **Travel moves** (dashed: where the pencil moves lifted), **Ruling**, **Pencil reach** (the reach window frame).
- At the bottom: the length of the lines, the number of pen lifts, the estimated time (in reality a bit longer), the last written word and the word to start the next sheet with. Red messages block the file; yellow ones are just warnings.

---

## The Drawing tab

Load a drawing — the program fits it to the sheet, shows how it will look and prepares the files. If the sheet is larger than the reach window, the drawing is split into passes.

### File

| Setting | What it is |
|---|---|
| List / Load a drawing… | Which drawing to draw. **SVG**, **DXF**, **PDF** (with vector graphics, not a scan) and **PNG/JPG** are supported. There is a built-in test drawing: a frame, diagonals, a circle and center lines. |
| File units | The units used inside the file. Usually “from the file” — the program figures it out. If the drawing comes out 10 or 25 times too large or too small, choose them manually (mm, cm, inches…). |
| PDF page | Which PDF page to use. |
| Threshold, automatic | For images: what counts as a line. Automatic usually works well. Manually: a number 0–255; the higher it is, the lighter areas become lines. |
| DPI | For images: dots per inch. Matters only for 1:1 scale. 0 takes it from the file. |
| Light lines on a dark background | For “white on black” images. |
| Small filled shapes → center lines | Filled arrows, text converted to curves and outlined lines are drawn as one line through the middle instead of a double outline. The threshold says how small (by the shorter side), 5 mm. |

What the program **does not draw**: fills (including section hatching in DXF) and text that is still text in the file, not converted to curves. There will be a warning listing such places, with orange marks in the preview. Images embedded in the file are skipped too. PNG/JPG images are drawn with lower quality than vector files — the program warns about that.

### Sheet and scale

| Setting | What it is | Default |
|---|---|---|
| Format | A4, A3 or a custom size (width and height). | A4 |
| Orientation | Portrait, landscape or auto (by the shape of the drawing). | auto |
| **Fit to the sheet** | Make the drawing as large as possible, centered, without distortion. | selected |
| **Fit to the reachable area (one pass)** | Shrink it so everything is drawn in one run, without rotating the sheet. | |
| **Fit the scale to the passes** | The largest scale at which every line can be reached in at least one pass. | |
| 1:1 in file units | Real size, as in the file. If it does not fit, you get a warning. | |
| Set, % | Your own scale in percent. | 100 |
| Margin for “fit”, mm | Distance from the sheet edge when fitting (without a frame). | 10 |
| Shift dx, dy | Move the drawing on the sheet. | 0 |

Below the settings you see the resulting scale (for example “1:2.5” or “1.85:1”), the size on paper and the largest percentage that fits into one pass.

### GOST 2.104 frame

| Setting | What it is | Default |
|---|---|---|
| Frame | Draw a drawing frame. | off |
| Left / right / top / bottom | Frame distances from the sheet edges. | 20 / 5 / 5 / 5 |
| Title block | The table in the bottom right corner (form 1) — lines only, no text. | on |
| Title block width / height | Table size; its inner lines stretch with it. | 185 × 55 |
| GOST defaults | Restore the standard sizes. | |

With a frame, the drawing is placed inside it — above the title block or to its left.

### Line weights

The pencil draws thin lines. To make the main lines of a drawing look thicker, they can be drawn several times side by side.

| Setting | What it is | Default |
|---|---|---|
| Draw thick lines with several passes | Turn it on. | off |
| Thick if wider than, mm | Which lines count as thick (the weight is taken from the file). | 0.4 |
| Passes | How many times to draw a thick line. | 3 |
| Step between passes, mm | Distance between neighboring lines. | 0.15 |
| Layers (for DXF) | For each layer choose “by weight”, “thin” or “thick” — useful if the file has no line weights. | by weight |

### Pencil path

| Setting | What it is | Default |
|---|---|---|
| Curve tolerance, mm | How accurately arcs and circles are split into short straight pieces. | 0.05 |
| Join ends within, mm | Lines whose ends are closer than this are drawn in one movement, without lifting the pencil. | 0.05 |
| Long paths from, mm | Long lines are drawn first, small details later. | 30 |
| RDP simplification, mm | The same as “Simplification tolerance” in the Printer block: how much lines may be straightened. | 0.05 |

### Table and passes

| Setting | What it is | Default |
|---|---|---|
| Reach calibration wizard… | A step-by-step measurement of the reach window with a table diagram (see below). | |
| Reach window measured, x_min … y_max | How far the pencil can move from the sheet corner to the left, right, towards you and away from you. Negative numbers go past the corner, above the stops. | not measured |
| Margin to the limits, mm | Do not come closer than this to the edge of the window. | 2 |
| The sheet may overhang the table towards +X / +Y | Whether the sheet may hang over the table edge on that side. On the other sides the printer body is in the way. “+X” is along the long arrow of the test file, “+Y” along the short one. | +X on, +Y off |
| Table from the stops along +X / +Y, mm | Table size measured from the corner of the stops (with a ruler). Optional: if not set, the edge of the window is taken as the table edge. | not set |

**Reach calibration wizard**:

1. Put the sheet against the stops and the pencil into the sheet corner until it touches.
2. Run the zero file (it stores the corner as zero and lifts the pencil).
3. Using the printer’s Move menu, move the pencil in turn as far as it goes to the right (+X), away from you (+Y), to the left (−X) and towards you (−Y). Each time write down the coordinates shown on the printer screen. If you did not run the zero file, copy the coordinates of the corner from the screen into the Corner fields.
4. Press Save to settings — the program calculates the window itself.
5. Download the check file. The pencil visits the four corners of the window (2 mm inset), makes a dot in each and pauses. If it hits the frame anywhere, shrink the window on that side.

### Passes and files

Each pass has a **card**:

- the pass number and the sheet rotation (0°, 90°, 180°, 270° counterclockwise, seen from above);
- **which sheet corner goes under the pencil** (against the stops) — the zero is set there. The corners are labeled with letters in the preview: A bottom left, B bottom right, C top right, D top left;
- a small diagram: how the sheet lies on the table and which part of it is drawn in this pass;
- which file to run and in what order, its name, the estimated time and the length of the lines;
- **dx, dy** — a correction for this pass if the zero ends up slightly off after rotating the sheet (see “Control crosses”);
- the **gcode** and **test** buttons — download the files of this pass only.

**Seams and crosses**:

| Setting | What it is | Default |
|---|---|---|
| Overlap at the seam, mm | Lines that cross the seam are drawn a little past it by each pass. If the sheet is off by half a millimetre, the line still does not break. | 0.5 |
| Margin for zero shift, mm | The seam is chosen so that this much margin is left for an imprecise zero. | 1 |
| Control crosses along the seams | Both neighboring passes draw small crosses at the same points of the sheet. Whether they match or not shows what dx, dy correction to enter. | off |
| Cross size, crosses per seam | The size of a cross and how many per seam. | 3 mm, 3 |

The program chooses the seam itself: so that it cuts as few lines as possible and avoids cutting circles where it can. Every part of the drawing is drawn by exactly one pass; there are no double lines except the overlap at the seam and the crosses.

### Buttons and preview

- **Download N pass files** — all files at once. In Chrome and Edge the program asks you to choose a folder, for example the SD card, and writes everything there. The names look like `…_pass1_rot0.gcode`, `…_pass2_rot180.gcode`: the pass number and the sheet rotation.
- **Pass test files** — for each pass: the frame of the part of the sheet the pencil can reach, dashed — the part this pass draws, axis arrows at the corner, ticks for the pass number, crosses. Handy to run on a scrap sheet before the final one.
- Above the sheet:
  - **View** — “sheet, all passes” or “pass N on the table”: the sheet rotated the way you will put it, with its corner at the stops;
  - **Travel moves**;
  - **Pass map** — colored rectangles show what each pass reaches, red hatching shows where the pencil does not reach in any pass;
  - **Line weights**.
- Lines are colored by their pass. The dash-dot line with a gray band is the seam. Crosses are circled.
- Under the preview: how many passes the sheet and the drawing need, which rotations are not possible and why (for example “overhangs the table by 82 mm towards +Y”). If something does not fit, the program says how many millimetres are missing and what to change: allow a side, reduce the scale with the button, choose a smaller sheet.

### Drawing in two passes

1. Download the pass files to the SD card.
2. Put the sheet as shown in the card of pass 1: the indicated corner against the stops. Lower the pencil into that corner until it touches the paper. Run the `…_pass1_…` file.
3. Rotate the sheet as the card of pass 2 says (for example by 180°: corner C against the stops). Again put the pencil into the corner until it touches. Run `…_pass2_…`.
4. If the lines at the seam are slightly apart, turn on the crosses, run the test files on a scrap sheet and enter a dx, dy correction for the second pass. If the cross of the second pass landed 0.8 mm to the right, set dx = −0.8.

---

## Common questions

**The file is not downloaded and there is a red message at the bottom.** Read it: it says what is wrong. Most often it is “outside the pencil reach” (it does not fit into the window: reduce the scale or the margins), “Missing from the font” (decide what to do with the character) or “Pencil reach not measured” (only a warning, but measuring the window is better).

**The pencil does not touch the paper or presses too hard.** Change `pen_down_z` in steps of 0.2–0.5 mm.

**Lines are shaky or broken.** Reduce `feed_draw` (writing speed) and check that the pencil is not loose in its holder.

**The drawing came out 10 or 25 times the wrong size.** Choose the correct File units in the File block.

**Text from the drawing was not drawn.** In the editor where the drawing was made, convert the text to curves (in Inkscape: Path → Object to Path; in AutoCAD: the `TXTEXP` command) and load the file again.

**The program stopped responding and the window shows a red bar “HandWriter has stopped”.** Close the window and run `HandWriter.exe` again.

## Where the settings are stored

All settings, loaded fonts and drawings are stored in the `%APPDATA%\HandWriter` folder — paste this into the Explorer address bar. You can replace the program folder with a new version; your settings stay. If something breaks, the window shows a message, and the details are in `%APPDATA%\HandWriter\logs\handwriter.log` — attach it when reporting a problem.

To check that the program is complete, run `HandWriter.exe --selftest`. The result is written to `%APPDATA%\HandWriter\logs\selftest.txt`.

## For developers

Building it yourself requires Windows and Python 3.12:

```
build.bat        Russian interface  -> dist\HandWriter\HandWriter.exe
build.bat en     English interface  -> dist\HandWriter-en\HandWriter.exe
```

Run from source: `python app.py` (set `HANDWRITER_LANG=en` for the English interface), tests: `python -m pytest -q`.

## Licenses of the built-in fonts

- Hershey Complex Cyrillic and Complex Roman (A. V. Hershey, 1967) — public domain.
- Bad Script (Google Fonts) — SIL Open Font License 1.1; the license text is next to the font: `_internal\handwriter\fonts\BadScript-OFL.txt`.
