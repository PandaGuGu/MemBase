@echo off
rem Keep this file ASCII-only.
rem cmd tracks the batch file position in BYTES but decodes with code page 65001,
rem so every multibyte line shifts the read position and the parser starts eating
rem the beginning of a later line (that is where "'e.exe' is not recognized" came
rem from). Chinese text here WILL break the build sooner or later.
chcp 65001 >nul
setlocal
cd /d "%~dp0"

echo [1/2] build (release, opt-level=3 + lto)
cargo build --workspace --release
if errorlevel 1 (
  echo [x] build failed
  exit /b 1
)

echo [2/2] collect to dist
if not exist dist mkdir dist
rem One exe only: with args it is the CLI, without args it opens the GUI,
rem "mcp" makes it an MCP server.
copy /y target\release\MemBase.exe    dist\ >nul
if errorlevel 1 (
  rem Usually a running instance or an antivirus scan holding the file.
  ping -n 3 127.0.0.1 >nul
  copy /y target\release\MemBase.exe    dist\ >nul
)
if errorlevel 1 (
  echo [x] copy to dist failed: MemBase.exe is locked. Close it and retry.
  exit /b 1
)

rem Old console-only builds are no longer shipped; clean leftovers.
if exist dist\deskkit.exe del /q dist\deskkit.exe
if exist dist\membase-cli.exe del /q dist\membase-cli.exe

echo.
for %%F in (dist\MemBase.exe) do echo   %%~nxF  %%~zF bytes
echo.
echo usage:
echo   dist\MemBase.exe                    open the GUI
echo   dist\MemBase.exe layout list        same exe, CLI mode
echo   dist\MemBase.exe mcp                same exe, MCP server for AI
