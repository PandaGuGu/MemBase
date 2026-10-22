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
rem 只分发一个 exe：带参数是命令行，无参数开界面，mcp 子命令当 AI 的 MCP server
copy /y target\release\MemBase.exe    dist\ >nul
rem 老的纯控制台版不再分发，残留一并清掉
if exist dist\deskkit.exe del /q dist\deskkit.exe
if exist dist\membase-cli.exe del /q dist\membase-cli.exe

echo.
for %%F in (dist\MemBase.exe) do (
  echo   %%~nxF  %%~zF 字节
)
echo.
echo 用法：
echo   dist\MemBase.exe                    开界面
echo   dist\MemBase.exe layout list        同一个 exe 当命令行用
echo   dist\MemBase.exe mcp                同一个 exe 当 MCP server（给 AI 调用）
