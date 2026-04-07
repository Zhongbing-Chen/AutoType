# AutoType 日志监控脚本
param(
    [switch]$Follow = $true
)

$ErrorActionPreference = "SilentlyContinue"

Write-Host "=== AutoType 日志监控 ===" -ForegroundColor Cyan
Write-Host ""

# Node.js 服务日志
$nodejsLog = "$env:TEMP\vocotype-nodejs.log"

# GUI 日志（如果有）
$guiLog = "$env:LOCALAPPDATA\VocoType\logs\app.log"

function Show-Logs {
    Clear-Host
    Write-Host "=== AutoType 日志监控 ===" -ForegroundColor Cyan
    Write-Host "按 Ctrl+C 退出" -ForegroundColor Gray
    Write-Host ""

    # 检查 Node.js 服务
    $nodeProcess = Get-Process -Name "node" -ErrorAction SilentlyContinue
    if ($nodeProcess) {
        Write-Host "[✓] Node.js 服务运行中 (PID: $($nodeProcess.Id))" -ForegroundColor Green
    } else {
        Write-Host "[✗] Node.js 服务未运行" -ForegroundColor Red
    }

    # 检查 Tauri 进程
    $tauriProcess = Get-Process -Name "vocotype-gui" -ErrorAction SilentlyContinue
    if ($tauriProcess) {
        Write-Host "[✓] GUI 运行中 (PID: $($tauriProcess.Id))" -ForegroundColor Green
    } else {
        Write-Host "[✗] GUI 未运行" -ForegroundColor Red
    }

    # 检查端口
    $portCheck = netstat -ano | Select-String "10095"
    if ($portCheck) {
        Write-Host "[✓] 端口 10095 已监听" -ForegroundColor Green
    } else {
        Write-Host "[✗] 端口 10095 未监听" -ForegroundColor Red
    }

    Write-Host ""
    Write-Host "--- Node.js 服务日志 (最近 20 行) ---" -ForegroundColor Yellow
    if (Test-Path $nodejsLog) {
        Get-Content $nodejsLog -Tail 20 | ForEach-Object {
            if ($_ -match "ERROR") {
                Write-Host $_ -ForegroundColor Red
            } elseif ($_ -match "WARN") {
                Write-Host $_ -ForegroundColor Yellow
            } else {
                Write-Host $_
            }
        }
    } else {
        Write-Host "日志文件不存在" -ForegroundColor Gray
    }

    Write-Host ""
    Write-Host "--- Python 日志 ---" -ForegroundColor Yellow
    $pythonLog = Get-ChildItem -Path "$env:USERPROFILE\.vocotype\logs" -Filter "*.log" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($pythonLog) {
        Get-Content $pythonLog.FullName -Tail 10 | ForEach-Object {
            Write-Host $_
        }
    }
}

if ($Follow) {
    while ($true) {
        Show-Logs
        Start-Sleep 2
    }
} else {
    Show-Logs
}
