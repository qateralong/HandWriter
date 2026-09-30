@echo off
setlocal
cd /d "%~dp0"

set "UI_LANG=ru"
set "ONEFILE="
set "NOTEST="
for %%A in (%*) do (
    if /i "%%~A"=="en" set "UI_LANG=en"
    if /i "%%~A"=="ru" set "UI_LANG=ru"
    if /i "%%~A"=="onefile" set "ONEFILE=1"
    if /i "%%~A"=="notest" set "NOTEST=1"
)
set "OUT=HandWriter"
if /i "%UI_LANG%"=="en" set "OUT=HandWriter-en"

set "PY=.venv\Scripts\python.exe"
if exist "%PY%" goto have_venv

echo [1/5] Creating virtual environment .venv
where py >nul 2>nul
if not errorlevel 1 (
    py -3.12 -m venv .venv 2>nul || py -3 -m venv .venv
)
if not exist "%PY%" if exist "%LOCALAPPDATA%\Programs\Python\Python312\python.exe" "%LOCALAPPDATA%\Programs\Python\Python312\python.exe" -m venv .venv
if not exist "%PY%" python -m venv .venv
if not exist "%PY%" (
    echo ERROR: Python 3.11+ not found. Install Python 3.12 from python.org and run build.bat again.
    exit /b 1
)

:have_venv
echo [2/5] Installing dependencies
"%PY%" -m pip install --disable-pip-version-check -q -r requirements.txt -r requirements-build.txt
if errorlevel 1 (
    echo ERROR: pip install failed.
    exit /b 1
)

echo [3/5] Building dist\%OUT% (onedir, interface language: %UI_LANG%)
set "HANDWRITER_BUILD_LANG=%UI_LANG%"
set "HANDWRITER_ONEFILE="
"%PY%" -m PyInstaller --noconfirm --clean HandWriter.spec
if errorlevel 1 (
    echo ERROR: PyInstaller failed.
    exit /b 1
)

if defined ONEFILE (
    echo [3b] Building dist\%OUT%-onefile.exe
    set "HANDWRITER_ONEFILE=1"
    "%PY%" -m PyInstaller --noconfirm HandWriter.spec
    if errorlevel 1 (
        echo ERROR: onefile build failed.
        exit /b 1
    )
    set "HANDWRITER_ONEFILE="
)

if defined NOTEST goto done
echo [4/5] Self-test of the built exe (checks, libraries, data)
start "" /wait "dist\%OUT%\HandWriter.exe" --selftest --report "%CD%\dist\selftest-%UI_LANG%.txt"
set RC=%ERRORLEVEL%
powershell -NoProfile -Command "Get-Content -Encoding UTF8 'dist\selftest-%UI_LANG%.txt'"
if not "%RC%"=="0" (
    echo ERROR: self-test failed, see dist\selftest-%UI_LANG%.txt
    exit /b 1
)

:done
echo [5/5] Done: dist\%OUT%\HandWriter.exe
echo Copy the whole folder dist\%OUT% to another computer; Python is not needed there.
exit /b 0
