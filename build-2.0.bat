@echo off
setlocal
cd /d "%~dp0"

set "NOTEST="
for %%A in (%*) do (
    if /i "%%~A"=="notest" set "NOTEST=1"
)
set "OUT=dist\HandWriter-2.0"

echo [1/4] Checking tools
where cargo >nul 2>nul
if errorlevel 1 (
    echo ERROR: Rust not found. Install it from https://rustup.rs ^(default MSVC toolchain^) and run build-2.0.bat again.
    exit /b 1
)
if not defined LIBCLANG_PATH (
    if exist "%ProgramFiles%\LLVM\bin\libclang.dll" set "LIBCLANG_PATH=%ProgramFiles%\LLVM\bin"
)
if not defined LIBCLANG_PATH (
    echo ERROR: LLVM ^(libclang^) not found. Install it: winget install LLVM.LLVM
    exit /b 1
)
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo ERROR: Visual Studio Build Tools not found. Install "Build Tools for Visual Studio 2022" with "Desktop development with C++".
    exit /b 1
)

echo [2/4] Building handwriter.exe ^(release^)
cargo build --release -p handwriter
if errorlevel 1 (
    echo ERROR: cargo build failed.
    exit /b 1
)

echo [3/4] Copying to %OUT%
if not exist "%OUT%" mkdir "%OUT%"
copy /y "target\release\handwriter.exe" "%OUT%\HandWriter.exe" >nul
if errorlevel 1 (
    echo ERROR: copy failed. Is HandWriter.exe still running?
    exit /b 1
)

if defined NOTEST goto done
echo [4/4] Self-test of the built exe
start "" /wait "%OUT%\HandWriter.exe" --selftest --report "%CD%\dist\selftest-2.0.txt"
set RC=%ERRORLEVEL%
powershell -NoProfile -Command "Get-Content -Encoding UTF8 'dist\selftest-2.0.txt'"
if not "%RC%"=="0" (
    echo ERROR: self-test failed, see dist\selftest-2.0.txt
    exit /b 1
)

:done
echo Done: %OUT%\HandWriter.exe
echo It is a single file; copy it anywhere. Windows 10/11 already has WebView2, nothing else is needed.
exit /b 0
