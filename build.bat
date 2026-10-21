@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

echo [1/2] 编译（release，opt-level=3 + lto）
cargo build --workspace --release
if errorlevel 1 (
  echo [x] 编译失败
  exit /b 1
)

echo [2/2] 收产物到 dist
if not exist dist mkdir dist
copy /y target\release\deskkit.exe    dist\ >nul
copy /y target\release\deskkit-ui.exe dist\ >nul

echo.
for %%F in (dist\deskkit.exe dist\deskkit-ui.exe) do (
  echo   %%~nxF  %%~zF 字节
)
echo.
echo 用法：
echo   dist\deskkit-ui.exe                 开界面
echo   dist\deskkit-ui.exe layout list     同一个 exe 当命令行用
echo   dist\deskkit.exe layout status      纯控制台版
