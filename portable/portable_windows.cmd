@echo off
rem =============================================================================
rem portable/portable_windows.cmd — build + bundle the portable Windows version.
rem
rem Produces:
rem   KetraCopy-portable\                          (self-contained folder)
rem     ketra.exe, ketracopy-gui.exe (if built), LICENSE, portable.flag
rem   dist\KetraCopy-1.1.0-portable-windows.zip    (via PowerShell Compress-Archive)
rem
rem The empty file "portable.flag" next to ketra.exe switches the engine into
rem portable mode: settings are read/written as ketra.json next to the exe
rem instead of %APPDATA%\KetraCopy (see crates/ketra-core/src/settings.rs).
rem
rem This is a batch file — it only runs under cmd.exe on Windows (7+). There
rem is no meaningful cross-OS guard inside cmd; the Linux/macOS equivalents
rem are portable_linux.sh / portable_macos.sh.
rem =============================================================================

setlocal EnableExtensions
set VERSION=1.1.0
set HERE=%~dp0
set ROOT=%HERE%..
pushd "%ROOT%"

if not exist "dist" mkdir "dist"

rem ---- build (or reuse an existing release build) ------------------------------
if not exist "target\release\ketra.exe" (
    echo [ketra] building release bundle ^(cargo^)...
    cargo build --release -p ketra-cli
    if errorlevel 1 goto :fail
)
if not exist "target\release\ketra.exe" (
    echo [error] target\release\ketra.exe not found and cargo build failed.
    echo         Build first with: cargo build --release -p ketra-cli
    goto :fail
)

rem ---- assemble the portable folder --------------------------------------------
if not exist "KetraCopy-portable" mkdir "KetraCopy-portable"
copy /y "target\release\ketra.exe" "KetraCopy-portable\ketra.exe" >nul
if exist "target\release\ketracopy-gui.exe" (
    copy /y "target\release\ketracopy-gui.exe" "KetraCopy-portable\ketracopy-gui.exe" >nul
) else (
    echo [note] target\release\ketracopy-gui.exe not built - bundling CLI only.
)
if exist "LICENSE" copy /y "LICENSE" "KetraCopy-portable\LICENSE" >nul

rem Empty portable.flag -> portable mode (settings next to the exe):
type nul > "KetraCopy-portable\portable.flag"

>  "KetraCopy-portable\README-portable.txt" echo KetraCopy %VERSION% portable for Windows
>> "KetraCopy-portable\README-portable.txt" echo.
>> "KetraCopy-portable\README-portable.txt" echo portable.flag marks this folder as portable: settings are kept in
>> "KetraCopy-portable\README-portable.txt" echo ketra.json next to ketra.exe instead of %%APPDATA%%\KetraCopy.
>> "KetraCopy-portable\README-portable.txt" echo.
>> "KetraCopy-portable\README-portable.txt" echo Usage:  ketra.exe copy SRC -d DEST   ^(or: ketracopy-gui.exe^)
>> "KetraCopy-portable\README-portable.txt" echo Help:   ketra.exe --help

rem ---- zip it up ----------------------------------------------------------------
powershell -NoProfile -ExecutionPolicy Bypass -Command "try { Compress-Archive -Path 'KetraCopy-portable\*' -DestinationPath 'dist\KetraCopy-%VERSION%-portable-windows.zip' -Force; exit 0 } catch { exit 1 }"
if errorlevel 1 (
    echo [warn] Compress-Archive failed ^(needs PowerShell 5+; Win7 ships PS 2.0^).
    echo        The folder 'KetraCopy-portable' itself IS the portable app -
    echo        zip it with any archiver or copy it to a USB stick as-is.
) else (
    echo [done] dist\KetraCopy-%VERSION%-portable-windows.zip
)

echo [done] KetraCopy-portable\
popd
endlocal
exit /b 0

:fail
echo [error] portable_windows.cmd failed.
popd
endlocal
exit /b 1
