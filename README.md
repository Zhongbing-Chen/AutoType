# AutoType - 语音转文字输入工具

一款基于 Rust + Tauri + Python FunASR 的桌面端语音转文字工具。按下快捷键即可录音，松开后自动将语音转为文字并输入到光标位置。

## 功能特点

- 🎙️ 实时语音识别（基于 FunASR Paraformer 模型）
- ⌨️ 全局快捷键支持（F2/F3）
- 🖥️ 本地运行，无需联网
- 📱 轻量级桌面应用
- 🔧 支持 ONNX 推理，CPU 即可运行

## 项目结构

```
autotype/
├── gui/                # Tauri 桌面应用 (Rust + WebView)
│   ├── src-tauri/      # Rust 后端代码
│   │   ├── src/        # 主程序、音频采集、快捷键
│   │   └── src-nodejs/ # Node.js ASR 服务
│   └── src/            # 前端页面
├── cli/                # Python 后端 (FunASR 语音识别)
│   ├── app/            # 核心识别模块
│   ├── main.py         # CLI 入口
│   └── .venv/          # Python 虚拟环境
└── logs/               # 日志监控脚本
```

## 系统要求

- Windows 10/11
- [Node.js](https://nodejs.org/) >= 16
- [Rust](https://rustup.rs/)
- Python 3.9+ (可选，已包含虚拟环境)
- 麦克风

## 快速开始

### 方式一：一键启动（开发模式）

```bash
cd autotype
start-all.bat
```

### 方式二：手动启动

1. **启动 Node.js ASR 服务**
   ```bash
   cd autotype/gui/src-tauri/src-nodejs
   npm install
   node server.js
   ```

2. **启动 GUI 应用**
   ```bash
   cd autotype/gui
   npm install
   npm run tauri dev
   ```

## 使用方法

1. 启动应用后，系统托盘会显示 VocoType 图标
2. 按下 **F3**（或 F2）开始录音
3. 对着麦克风说话
4. 再次按下快捷键停止录音
5. 文字自动输入到当前光标位置

## 配置说明

### 修改快捷键

编辑 `gui/src-tauri/src/main.rs`：

```rust
// 第 716 行附近
match app.global_shortcut_manager().register("F3", move || {
    // ...
}) {
```

### 修改录音保存目录

默认：`%USERPROFILE%/Music/VocoType`

## 故障排查

### 端口 10095 被占用
```bash
# 查找占用端口的进程
netstat -ano | findstr 10095
# 结束进程
taskkill /PID <PID> /F
```

### 音频设备初始化失败
1. 检查麦克风是否连接
2. 检查麦克风权限（设置 > 隐私 > 麦克风）
3. 关闭其他占用麦克风的应用
4. 重启应用

### 语音识别失败
1. 检查 Node.js 服务是否运行：`curl http://localhost:10095/`
2. 查看服务日志
3. 确保虚拟环境 Python 可用

## 监控日志

运行监控脚本查看实时状态：
```bash
autotype/logs/watch.bat
```

## 技术栈

- **前端**: HTML/CSS/JS + Vite
- **桌面框架**: Tauri (Rust)
- **语音识别**: FunASR (Paraformer-large) + ONNX Runtime
- **后端服务**: Node.js + Express
- **音频采集**: CPAL (Rust)

## 模型信息

- ASR: `iic/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-onnx`
- VAD: `iic/speech_fsmn_vad_zh-cn-16k-common-onnx`
- 标点: `iic/punc_ct-transformer_zh-cn-common-vocab272727-onnx`

## 开发计划

- [ ] 支持自定义快捷键
- [ ] 支持选择输入设备
- [ ] 支持多语言识别
- [ ] 打包为独立安装程序
- [ ] 添加配置界面

## License

MIT

## 致谢

- [FunASR](https://github.com/alibaba-damo-academy/FunASR) - 阿里巴巴达摩院语音识别工具包
- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
