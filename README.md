# HandWriter

HandWriter turns a 3D printer into a hand with a pencil. It writes text in handwriting fonts with natural irregularities and draws technical drawings from SVG, DXF, PDF or PNG/JPG files.

The program makes gcode files: copy them to an SD card and run them from the printer screen. The gcode contains no heating commands and no `G28 Z`. Made for the Flying Bear Ghost 5; any Marlin printer will do.

## Start

Download a zip from Releases, unpack it and run `HandWriter.exe`. Keep the `_internal` folder next to it. The window opens in Chrome or Edge.

## Drawing on A4 and A3

1. Put the sheet on the printer marks and the pencil at 0, 0.
2. Enter the work area: where the pencil writes on the sheet, in printer coordinates.
3. Load a file that contains only the frame, without margins. The program puts the frame on the 20/5/5/5 mm margins and keeps the views 1:1.
4. Download the gcode archive and run the files in order:
   - A4 takes 2 runs: rotate the sheet by 180° between them;
   - A3 takes 4 runs: rotate by 180°, then by 90° clockwise, then by 180°.

## Build from source

Windows, Python 3.12:

```
build.bat        Russian interface  -> dist\HandWriter\HandWriter.exe
build.bat en     English interface  -> dist\HandWriter-en\HandWriter.exe
```

Run: `python app.py`. Tests: `python -m pytest -q`.

### HandWriter 2.0 (Rust, branch `rust-port`)

One `HandWriter.exe` without `_internal`, its own window (WebView2), interface language in the settings, printing over USB.

Windows: install Rust (rustup.rs), Build Tools for Visual Studio 2022 with "Desktop development with C++" and LLVM (`winget install LLVM.LLVM`), then:

```
build-2.0.bat    -> dist\HandWriter-2.0\HandWriter.exe
```

Linux: `cargo build --release -p handwriter`. Tests: `cargo test --workspace`. Self-test: `handwriter --selftest`. USB printing on Linux needs the `uucp` group: `sudo usermod -aG uucp $USER`, then log in again.

## Fonts

Hershey fonts are in the public domain. Bad Script is under the SIL Open Font License 1.1.
