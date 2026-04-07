@echo off
chcp 65001 >nul
echo === AutoType 启动脚本 ===
echo.

:: 检查 Node.js 服务
curl -s http://localhost:10095/ >nul 2>&1
if %errorlevel% neq 0 (
    echo [INFO] 启动 Node.js ASR 服务...
    start /min cmd /c "cd /d %~dp0\gui\src-tauri\src-nodejs && node server.js"
    timeout /t 3 /nobreak >nul
) else (
    echo [✓] Node.js ASR 服务已在运行
)

echo.
echo [INFO] 启动 GUI...
cd /d %~dp0\gui
start npm run tauri dev

echo.
echo [INFO] 应用启动中...
echo [HINT] 按 F3 开始/停止录音
echo.

:: 打开监控窗口
start cmd /c "%~dp0\logs\watch.bat"
