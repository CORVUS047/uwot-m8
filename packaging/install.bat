@echo off
rem Builds the windowed frontend and installs it for the current user: the
rem executable under %LOCALAPPDATA%, plus a Start Menu shortcut.
rem
rem Only uwot-m8 is installed. The terminal frontend needs a Unix terminal
rem for raw mode and key releases, so it is not built here.
setlocal EnableDelayedExpansion

set "here=%~dp0"
set "repo=%here%.."
set "dest=%LOCALAPPDATA%\Programs\uwot-m8"
set "startmenu=%APPDATA%\Microsoft\Windows\Start Menu\Programs"

where cargo >nul 2>&1
if errorlevel 1 (
    echo cargo was not found. Install Rust from https://rustup.rs and try again.
    exit /b 1
)

rem SDL2 has to come from somewhere. cmake means it can be built and linked in,
rem which leaves one executable with nothing beside it; without cmake the build
rem links against an SDL2 you provide yourself.
where cmake >nul 2>&1
if errorlevel 1 (
    set "sdl=system"
    echo cmake was not found, so SDL2 will not be built in.
    echo SDL2.dll will have to sit next to uwot-m8.exe, or on your PATH.
    echo Installing cmake and running this again avoids that.
    echo.
) else (
    set "sdl=bundled"
)

echo building uwot-m8
rem call, not a bare invocation: if cargo is a shim script rather than an exe, a
rem bare one hands control over and never comes back here.
if "!sdl!"=="bundled" (
    call cargo build --release --manifest-path "%repo%\Cargo.toml" --bin uwot-m8 --features bundled-sdl
) else (
    call cargo build --release --manifest-path "%repo%\Cargo.toml" --bin uwot-m8
)
if errorlevel 1 (
    echo the build failed; nothing was installed.
    exit /b 1
)

if not exist "%dest%" mkdir "%dest%"
copy /y "%repo%\target\release\uwot-m8.exe" "%dest%\uwot-m8.exe" >nul
if errorlevel 1 (
    echo could not copy uwot-m8.exe to %dest%
    exit /b 1
)

rem A shortcut rather than a change to PATH: setx truncates a long PATH, which
rem is a poor trade for saving a few keystrokes.
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('%startmenu%\uwot-m8.lnk');" ^
    "$s.TargetPath = '%dest%\uwot-m8.exe';" ^
    "$s.WorkingDirectory = '%dest%';" ^
    "$s.Description = 'Show a Dirtywave M8 on the desktop';" ^
    "$s.Save()" >nul 2>&1
rem Whether the shortcut is there, not what powershell claimed: a stubbed-out
rem powershell exits happily having done nothing.
if exist "%startmenu%\uwot-m8.lnk" (
    echo Start Menu shortcut: %startmenu%\uwot-m8.lnk
) else (
    echo note: could not create the Start Menu shortcut. uwot-m8 is installed
    echo all the same, at %dest%\uwot-m8.exe
)

echo installed uwot-m8 to %dest%
echo.
echo Settings live in %APPDATA%\uwot-m8\config.conf, and can be changed from
echo inside the app with Escape.
echo.
echo The M8 appears as a USB serial device and needs no driver on Windows 10 or
echo later. If uwot-m8 cannot find it, close anything else that has the port
echo open and try again.
endlocal
