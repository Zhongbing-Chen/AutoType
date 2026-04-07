@echo off
chcp 65001 >nul
echo === AutoType 日志监控 ===
echo.
echo 按 Ctrl+C 停止监控
echo.

:loop
cls
echo === AutoType 状态 ===
echo.

:: 检查 Node.js 服务
netstat -ano | findstr "10095" >nul
if %errorlevel% == 0 (
    echo [✓] Node.js ASR 服务: 运行中 (端口 10095)
) else (
    echo [✗] Node.js ASR 服务: 未运行
    echo       启动命令: cd autotype/gui/src-tauri/src-nodejs ^&^& node server.js
)

echo.
:: 检查 GUI 进程
tasklist | findstr "vocotype-gui" >nul
if %errorlevel% == 0 (
    echo [✓] GUI 应用: 运行中
) else (
    echo [✗] GUI 应用: 未运行
)

echo.
echo === 最近日志 ===
echo.

:: 显示 Node.js 服务日志（如果有）
if exist "%~dp0..\gui\src-tauri\src-nodejs\npm-debug.log" (
    echo Node.js 错误日志:
    type "%~dp0..\gui\src-tauri\src-nodejs\npm-debug.log" 2^>nul | tail -5
)

:: 显示录音目录
if exist "%USERPROFILE%\Music\VocoType" (
    echo 录音文件 (%USERPROFILE%\Music\VocoType):
    dir /b "%USERPROFILE%\Music\VocoType\*.wav" 2^>nul | tail -3
)

echo.
echo 刷新间隔: 3秒
timeout /t 3 /nobreak >nul
goto loop
