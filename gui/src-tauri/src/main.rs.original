#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, State, GlobalShortcutManager};
use std::sync::{Arc, Mutex};
use std::process::{Command, Stdio};
use std::io::Write;
use std::path::PathBuf;
use chrono::Local;

// 音频设备检测
#[tauri::command]
fn check_audio_devices() -> Result<String, String> {
    use cpal::traits::{HostTrait, DeviceTrait};

    let mut result = String::new();

    // 1. 检查可用主机
    result.push_str("=== CPAL 音频后端 ===\n");
    result.push_str(&format!("默认主机: {:?}\n", cpal::default_host().id()));
    result.push_str(&format!("可用主机: {:?}\n\n", cpal::available_hosts()));

    // 2. 尝试所有可用主机
    for host_id in cpal::available_hosts() {
        result.push_str(&format!("--- {:?} 主机 ---\n", host_id));
        let host = cpal::host_from_id(host_id).map_err(|e| e.to_string())?;

        // 输入设备
        match host.input_devices() {
            Ok(devices) => {
                let device_list: Vec<_> = devices.collect();
                result.push_str(&format!("  输入设备: {} 个\n", device_list.len()));
                for (i, d) in device_list.iter().enumerate() {
                    match d.name() {
                        Ok(name) => {
                            // 尝试获取默认配置
                            match d.default_input_config() {
                                Ok(config) => {
                                    result.push_str(&format!(
                                        "    [{}] {} ({}Hz, {:?}, {}ch)\n",
                                        i,
                                        name,
                                        config.sample_rate().0,
                                        config.sample_format(),
                                        config.channels()
                                    ));
                                }
                                Err(_) => {
                                    result.push_str(&format!("    [{}] {}\n", i, name));
                                }
                            }
                        }
                        Err(_) => result.push_str(&format!("    [{}] [无法获取名称]\n", i)),
                    }
                }
            }
            Err(e) => result.push_str(&format!("  输入设备错误: {}\n", e)),
        }

        // 默认输入设备
        match host.default_input_device() {
            Some(d) => {
                let name = d.name().unwrap_or_else(|_| "Unknown".to_string());
                result.push_str(&format!("  默认输入: {}\n", name));
            }
            None => result.push_str("  默认输入: [无]\n"),
        }

        // 输出设备
        match host.output_devices() {
            Ok(devices) => {
                let device_list: Vec<_> = devices.collect();
                result.push_str(&format!("  输出设备: {} 个\n", device_list.len()));
            }
            Err(e) => result.push_str(&format!("  输出设备错误: {}\n", e)),
        }
        result.push('\n');
    }

    // 3. Windows 特定检查
    #[cfg(target_os = "windows")]
    {
        result.push_str("=== Windows 音频服务状态 ===\n");
        use std::process::Command;

        // 检查 Windows Audio 服务
        match Command::new("sc").args(["query", "Audiosrv"]).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("RUNNING") {
                    result.push_str("Windows Audio 服务: 运行中\n");
                } else if stdout.contains("STOPPED") {
                    result.push_str("Windows Audio 服务: 已停止\n");
                } else {
                    result.push_str("Windows Audio 服务: 未知状态\n");
                }
            }
            Err(e) => result.push_str(&format!("无法查询音频服务: {}\n", e)),
        }

        // 检查远程桌面音频
        match Command::new("sc").args(["query", "TermService"]).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("RUNNING") {
                    result.push_str("远程桌面服务: 运行中\n");
                }
            }
            Err(_) => {}
        }
    }

    Ok(result)
}

// 音频采集模块 - 使用消息传递架构避免 Send  trait 问题
mod audio {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Stream, StreamConfig};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::sync::mpsc::{channel, Sender, Receiver};
    use hound::{WavSpec, WavWriter};
    use std::path::PathBuf;
    use std::thread::{self, JoinHandle};

    pub enum AudioCommand {
        Start,
        Stop,
    }

    pub struct AudioRecorder {
        cmd_sender: Sender<AudioCommand>,
        data_receiver: Receiver<Vec<i16>>,
        is_recording: Arc<AtomicBool>,
        _thread_handle: Option<JoinHandle<()>>,
        sample_rate: u32,
    }

    impl AudioRecorder {
        pub fn new() -> Result<Self, String> {
            let (cmd_sender, cmd_receiver): (Sender<AudioCommand>, Receiver<AudioCommand>) = channel();
            let (data_sender, data_receiver): (Sender<Vec<i16>>, Receiver<Vec<i16>>) = channel();
            let is_recording = Arc::new(AtomicBool::new(false));
            let is_recording_thread = is_recording.clone();

            let thread_handle = thread::spawn(move || {
                AudioRecorder::audio_thread(cmd_receiver, data_sender, is_recording_thread);
            });

            // 等待一小段时间让音频线程初始化
            thread::sleep(std::time::Duration::from_millis(100));

            Ok(Self {
                cmd_sender,
                data_receiver,
                is_recording,
                _thread_handle: Some(thread_handle),
                sample_rate: 16000,
            })
        }

        fn audio_thread(
            cmd_receiver: Receiver<AudioCommand>,
            data_sender: Sender<Vec<i16>>,
            is_recording: Arc<AtomicBool>,
        ) {
            use cpal::traits::DeviceTrait;
            let host = cpal::default_host();
            println!("音频线程: 使用主机 {:?}", host.id());

            // 带重试的设备获取
            let mut device: Option<cpal::Device> = None;
            for attempt in 0..3 {
                // 尝试获取默认设备
                if let Some(d) = host.default_input_device() {
                    device = Some(d);
                    break;
                }

                // 如果没有默认设备，尝试列表中的第一个
                match host.input_devices() {
                    Ok(mut devices) => {
                        if let Some(d) = devices.next() {
                            device = Some(d);
                            break;
                        }
                    }
                    Err(e) => println!("音频线程: 无法枚举输入设备: {}", e),
                }

                if attempt < 2 {
                    println!("音频线程: 未找到设备，等待重试...");
                    thread::sleep(std::time::Duration::from_millis(500));
                }
            }

            let device = match device {
                Some(d) => d,
                None => {
                    eprintln!("[ERROR] 音频线程: 未找到可用输入设备，音频功能将不可用");
                    eprintln!("[HINT] 请检查: 1. 麦克风是否连接 2. 麦克风权限是否开启 3. 是否有其他应用占用麦克风");
                    // 进入空循环，保持通道开启但什么都不做
                    loop {
                        match cmd_receiver.recv() {
                            Ok(_) => {
                                let _ = data_sender.send(Vec::new());
                            }
                            Err(_) => break,
                        }
                    }
                    return;
                }
            };

            let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            println!("音频线程: 使用设备 {}", device_name);

            let config: StreamConfig = match device.default_input_config() {
                Ok(c) => c.into(),
                Err(e) => {
                    eprintln!("获取默认输入配置失败: {}", e);
                    return;
                }
            };

            let recorded_data = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded_data_stream = recorded_data.clone();
            let is_recording_stream = is_recording.clone();

            let stream = match device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if is_recording_stream.load(Ordering::SeqCst) {
                        if let Ok(mut buffer) = recorded_data_stream.lock() {
                            buffer.extend_from_slice(data);
                        }
                    }
                },
                |err| eprintln!("音频流错误: {}", err),
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("构建音频流失败: {}", e);
                    return;
                }
            };

            // 主循环：处理命令
            loop {
                match cmd_receiver.recv() {
                    Ok(AudioCommand::Start) => {
                        // 清空之前的数据
                        if let Ok(mut data) = recorded_data.lock() {
                            data.clear();
                        }
                        is_recording.store(true, Ordering::SeqCst);
                        let _ = stream.play();
                        println!("音频线程: 录音已开始");
                    }
                    Ok(AudioCommand::Stop) => {
                        is_recording.store(false, Ordering::SeqCst);
                        let _ = stream.pause();

                        let recorded = if let Ok(data) = recorded_data.lock() {
                            data.clone()
                        } else {
                            Vec::new()
                        };

                        println!("音频线程: 录音已停止，采集 {} 样本", recorded.len());
                        let _ = data_sender.send(recorded);
                    }
                    Err(_) => {
                        println!("音频线程: 命令通道关闭，退出");
                        break;
                    }
                }
            }
        }

        pub fn is_recording(&self) -> bool {
            self.is_recording.load(Ordering::SeqCst)
        }

        pub fn start_recording(&self) -> Result<(), String> {
            if self.is_recording() {
                return Ok(());
            }
            // 检查音频线程是否存活（通过发送测试命令）
            match self.cmd_sender.send(AudioCommand::Start) {
                Ok(_) => Ok(()),
                Err(e) => {
                    eprintln!("音频线程通信失败: {:?}", e);
                    Err("音频设备初始化失败，请检查麦克风连接或重启应用".to_string())
                }
            }
        }

        pub fn stop_recording(&self) -> Result<Vec<i16>, String> {
            if !self.is_recording() {
                return Ok(Vec::new());
            }

            self.cmd_sender.send(AudioCommand::Stop)
                .map_err(|e| format!("发送停止命令失败: {}", e))?;

            // 等待数据返回（最多等待2秒）
            match self.data_receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(data) => Ok(data),
                Err(_) => Err("接收音频数据超时".to_string()),
            }
        }

        pub fn save_to_wav(&self, data: &[i16], path: &PathBuf) -> Result<(), String> {
            let spec = WavSpec {
                channels: 1,
                sample_rate: self.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let mut writer = WavWriter::create(path, spec)
                .map_err(|e| e.to_string())?;

            for &sample in data {
                writer.write_sample(sample).map_err(|e| e.to_string())?;
            }

            writer.finalize().map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

// 应用状态
struct AppState {
    is_recording: Mutex<bool>,
    recorder: Mutex<audio::AudioRecorder>,
    recordings_dir: Mutex<PathBuf>,
}

// 历史记录项
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct RecordingItem {
    id: String,
    timestamp: String,
    audio_file: String,
    text: String,
    duration_secs: f64,
}

// 切换录音状态
#[tauri::command]
fn toggle_recording(state: State<AppState>) -> Result<bool, String> {
    let mut is_recording = state.is_recording.lock().map_err(|e| e.to_string())?;

    if *is_recording {
        // 停止录音
        let recorder = state.recorder.lock().map_err(|e| e.to_string())?;
        let audio_data = recorder.stop_recording()?;
        *is_recording = false;

        // 保存录音文件
        if !audio_data.is_empty() {
            let recordings_dir = state.recordings_dir.lock().map_err(|e| e.to_string())?;
            let id = Local::now().format("%Y%m%d_%H%M%S").to_string();
            let wav_path = recordings_dir.join(format!("{}.wav", id));

            recorder.save_to_wav(&audio_data, &wav_path)?;
            println!("录音保存到: {:?}", wav_path);

            // 启动异步识别（这里简化处理，实际应该调用 Python）
            let txt_path = recordings_dir.join(format!("{}.txt", id));
            std::fs::write(&txt_path, "[识别中...]").map_err(|e| e.to_string())?;
        }

        Ok(false)
    } else {
        // 开始录音
        let recorder = state.recorder.lock().map_err(|e| e.to_string())?;
        recorder.start_recording().map_err(|e| {
            println!("开始录音失败: {}", e);
            format!("开始录音失败: {}", e)
        })?;
        *is_recording = true;
        println!("开始录音");
        Ok(true)
    }
}

// 获取录音状态
#[tauri::command]
fn get_recording_status(state: State<AppState>) -> Result<bool, String> {
    let is_recording = state.is_recording.lock().map_err(|e| e.to_string())?;
    Ok(*is_recording)
}

// 获取历史记录列表
#[tauri::command]
fn get_recordings(state: State<AppState>) -> Result<Vec<RecordingItem>, String> {
    let recordings_dir = state.recordings_dir.lock().map_err(|e| e.to_string())?;
    let mut items = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&*recordings_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "wav").unwrap_or(false) {
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                let txt_path = path.with_extension("txt");
                let text = if txt_path.exists() {
                    std::fs::read_to_string(&txt_path).unwrap_or_default()
                } else {
                    "[无转录结果]".to_string()
                };

                // 获取文件创建时间
                let metadata = entry.metadata().ok();
                let timestamp = metadata.and_then(|m| m.created().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        let dt = chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .unwrap_or_else(|| chrono::DateTime::UNIX_EPOCH);
                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|| stem.clone());

                items.push(RecordingItem {
                    id: stem.clone(),
                    timestamp,
                    audio_file: path.to_string_lossy().to_string(),
                    text,
                    duration_secs: 0.0, // 可以计算 WAV 文件时长
                });
            }
        }
    }

    // 按时间倒序排列
    items.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(items)
}

// 识别指定录音 - 支持多种识别方式 (Node.js > HTTP API > Python)
#[tauri::command]
fn transcribe_recording(state: State<AppState>, id: String) -> Result<String, String> {
    let recordings_dir = state.recordings_dir.lock().map_err(|e| e.to_string())?;
    let wav_path = recordings_dir.join(format!("{}.wav", id));
    let txt_path = recordings_dir.join(format!("{}.txt", id));

    if !wav_path.exists() {
        return Err("录音文件不存在".to_string());
    }

    // 先标记为识别中
    let _ = std::fs::write(&txt_path, "[识别中...]");

    // 尝试多种识别方式
    let result: Result<String, String> = transcribe_with_nodejs(&wav_path)
        .or_else(|err| {
            println!("Node.js 识别失败: {}, 尝试 HTTP API...", err);
            transcribe_with_http(&wav_path)
        })
        .or_else(|err| {
            println!("HTTP API 失败: {}, 尝试 Python...", err);
            transcribe_with_python(&wav_path)
        })
        .or_else(|err| {
            println!("所有识别方式失败: {}", err);
            Ok("[语音识别服务未启动，请先运行: cd src-nodejs && npm install && npm start]".to_string())
        });

    let result_text = match result {
        Ok(text) if text.is_empty() => "[识别结果为空]".to_string(),
        Ok(text) => text,
        Err(_) => "[识别失败]".to_string(),
    };

    // 保存结果
    std::fs::write(&txt_path, &result_text).map_err(|e| e.to_string())?;

    Ok(result_text)
}

// 使用 Node.js Whisper 服务识别
fn transcribe_with_nodejs(wav_path: &PathBuf) -> Result<String, String> {
    // 检查 Node.js 服务是否运行
    let check = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "http://localhost:10095/"])
        .output();

    match check {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout);
            if code.trim() != "200" {
                return Err("Node.js 服务未运行".to_string());
            }
        }
        Err(_) => return Err("无法检查服务状态".to_string()),
    }

    // 调用 Node.js 服务进行转录
    let request_body = serde_json::json!({
        "audioPath": wav_path.to_string_lossy().to_string()
    });
    let output = Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", "Content-Type: application/json",
            "-d", &request_body.to_string(),
            "http://localhost:10095/api/transcribe/local"
        ])
        .output()
        .map_err(|e| format!("调用 Node.js 服务失败: {}", e))?;

    let response = String::from_utf8_lossy(&output.stdout);
    println!("Node.js 响应: {}", response);

    // 解析 JSON 响应
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
        if json.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                return Ok(text.to_string());
            }
        }
    }

    Err("Node.js 识别失败".to_string())
}

// 使用 Python FunASR 识别
fn transcribe_with_python(wav_path: &PathBuf) -> Result<String, String> {
    // 尝试多种 Python 路径
    // 首先尝试使用 cli 虚拟环境中的 Python
    let venv_python = std::env::current_dir()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("autotype")
        .join("cli")
        .join(".venv")
        .join("Scripts")
        .join("python.exe");

    let python_cmds: Vec<&str> = if venv_python.exists() {
        println!("使用虚拟环境 Python: {}", venv_python.display());
        vec![venv_python.to_str().unwrap_or("python"), "python", "python3", "py"]
    } else {
        println!("虚拟环境未找到，尝试系统 Python");
        vec!["python", "python3", "py"]
    };

    for py_cmd in &python_cmds {
        let output = Command::new(py_cmd)
            .arg("-c")
            .arg(format!(
                r#"import sys
try:
    sys.path.insert(0, r'{}')
    from app.funasr_server import FunASRServer
    s = FunASRServer()
    s.initialize()
    result = s.transcribe_audio(r'{}')
    print(result.get('text', ''))
except Exception as ex:
    print(f'ERROR: {{ex}}', file=sys.stderr)
    sys.exit(1)
"#,
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("autotype").join("cli")
                    .to_string_lossy(),
                wav_path.to_string_lossy()
            ))
            .output();

        match output {
            Ok(out) if out.status.success() => {
                return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                println!("{} 识别错误: {}", py_cmd, err);
            }
            Err(e) => {
                println!("{} 不可用: {}", py_cmd, e);
            }
        }
    }

    Err("所有 Python 路径都失败".to_string())
}

// 使用 HTTP API 识别（备用方案）
fn transcribe_with_http(wav_path: &PathBuf) -> Result<String, String> {
    // 检查本地 FunASR 服务是否运行
    let check = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "http://localhost:10095/"])
        .output();

    match check {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout);
            if code.trim() == "200" {
                // 本地服务可用，使用它
                return call_local_asr_service(wav_path);
            }
        }
        Err(_) => {}
    }

    Err("HTTP 服务不可用".to_string())
}

// 调用本地 ASR 服务
fn call_local_asr_service(wav_path: &PathBuf) -> Result<String, String> {
    use std::process::Stdio;
    use std::io::Write;

    let file_content = std::fs::read(wav_path).map_err(|e| e.to_string())?;

    let mut child = Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", "Content-Type: audio/wav",
            "--data-binary", "@-",
            "http://localhost:10095/api/transcribe"
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 curl: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&file_content).map_err(|e| e.to_string())?;
    }

    let output = child.wait_with_output().map_err(|e| e.to_string())?;

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        // 解析 JSON 响应
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
                return Ok(text.to_string());
            }
        }
        Ok(response.trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

// 获取录音保存目录
#[tauri::command]
fn get_recordings_dir(state: State<AppState>) -> Result<String, String> {
    let dir = state.recordings_dir.lock().map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

// 启动 Node.js ASR 服务
fn start_nodejs_service() {
    use std::process::{Command, Stdio};

    // 检查服务是否已在运行
    let check = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "http://localhost:10095/"])
        .output();

    if let Ok(out) = check {
        let code = String::from_utf8_lossy(&out.stdout);
        if code.trim() == "200" {
            println!("✓ Node.js ASR 服务已在运行");
            return;
        }
    }

    // 启动服务
    let nodejs_dir = std::env::current_dir()
        .ok()
        .map(|p| p.join("src-nodejs"))
        .unwrap_or_else(|| std::path::PathBuf::from("src-nodejs"));

    if nodejs_dir.exists() {
        println!("正在启动 Node.js ASR 服务...");

        // 尝试使用 npm start
        let _ = Command::new("cmd")
            .args([
                "/C", "start", "/B", "/MIN",
                "npm", "start"
            ])
            .current_dir(&nodejs_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    } else {
        println!("⚠ Node.js 服务目录不存在: {:?}", nodejs_dir);
    }
}

fn main() {
    // 创建录音保存目录
    let recordings_dir = dirs::audio_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Audio"))
        .join("VocoType");

    std::fs::create_dir_all(&recordings_dir).expect("无法创建录音目录");
    println!("录音保存目录: {:?}", recordings_dir);

    // 启动 Node.js 服务
    start_nodejs_service();

    tauri::Builder::default()
        .manage(AppState {
            is_recording: Mutex::new(false),
            recorder: Mutex::new(audio::AudioRecorder::new().expect("无法创建录音器")),
            recordings_dir: Mutex::new(recordings_dir),
        })
        .invoke_handler(tauri::generate_handler![
            toggle_recording,
            get_recording_status,
            get_recordings,
            transcribe_recording,
            get_recordings_dir,
            check_audio_devices
        ])
        .setup(|app| {
            // 注册全局快捷键 F2
            let handle = app.handle();
            let mut shortcut_manager = app.global_shortcut_manager();

            // 尝试注册 F2，失败则尝试 F3
            let f2_result = shortcut_manager.register("F2", move || {
                let state: State<AppState> = handle.state();
                if let Ok(is_recording) = toggle_recording(state) {
                    println!("快捷键 F2 触发，录音状态: {}", is_recording);
                }
            });

            match f2_result {
                Ok(_) => println!("✓ 全局快捷键 F2 已注册"),
                Err(e) => {
                    println!("✗ 无法注册 F2: {}", e);
                    println!("  尝试注册 F3 作为备用...");

                    let handle2 = app.handle();
                    match app.global_shortcut_manager().register("F3", move || {
                        let state: State<AppState> = handle2.state();
                        if let Ok(is_recording) = toggle_recording(state) {
                            println!("快捷键 F3 触发，录音状态: {}", is_recording);
                        }
                    }) {
                        Ok(_) => println!("✓ 全局快捷键 F3 已注册（备用）"),
                        Err(e2) => println!("✗ 也无法注册 F3: {}", e2),
                    }
                }
            }

            Ok(())
        })
        .on_window_event(|event| match event.event() {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                event.window().hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
