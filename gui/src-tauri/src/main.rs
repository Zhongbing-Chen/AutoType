#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, State, GlobalShortcutManager, AppHandle};
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

// 修复后的音频采集模块 - 使用相位累积避免长时间漂移
mod audio {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::StreamConfig;
    use std::sync::atomic::{AtomicBool, Ordering, AtomicUsize};
    use std::sync::Arc;
    use std::sync::mpsc::{sync_channel, SyncSender, Receiver};
    use hound::{WavSpec, WavWriter};
    use std::path::PathBuf;
    use std::thread::{self, JoinHandle};

    // 最大录音时长：10分钟
    const MAX_RECORDING_SAMPLES: usize = 16000 * 60 * 10;

    pub enum AudioCommand {
        Start,
        Stop,
    }

    pub struct AudioRecorder {
        cmd_sender: SyncSender<AudioCommand>,
        data_receiver: Receiver<Vec<i16>>,
        amp_receiver: Receiver<f32>,
        is_recording: Arc<AtomicBool>,
        _thread_handle: Option<JoinHandle<()>>,
        sample_rate: u32,
    }

    impl AudioRecorder {
        pub fn new() -> Result<Self, String> {
            let (cmd_sender, cmd_receiver) = sync_channel(1);
            let (data_sender, data_receiver) = sync_channel(1);
            let (amp_sender, amp_receiver) = sync_channel(100);
            let is_recording = Arc::new(AtomicBool::new(false));
            let is_recording_thread = is_recording.clone();

            let thread_handle = thread::spawn(move || {
                AudioRecorder::audio_thread(cmd_receiver, data_sender, amp_sender, is_recording_thread);
            });

            thread::sleep(std::time::Duration::from_millis(100));

            Ok(Self {
                cmd_sender,
                data_receiver,
                amp_receiver,
                is_recording,
                _thread_handle: Some(thread_handle),
                sample_rate: 16000,
            })
        }

        fn audio_thread(
            cmd_receiver: Receiver<AudioCommand>,
            data_sender: SyncSender<Vec<i16>>,
            amp_sender: SyncSender<f32>,
            is_recording: Arc<AtomicBool>,
        ) {
            use cpal::traits::DeviceTrait;
            let host = cpal::default_host();
            println!("音频线程: 使用主机 {:?}", host.id());

            let mut device = None;
            for attempt in 0..5 {
                if let Some(d) = host.default_input_device() {
                    device = Some(d);
                    break;
                }
                if let Ok(mut devices) = host.input_devices() {
                    if let Some(d) = devices.next() {
                        device = Some(d);
                        break;
                    }
                }
                println!("音频线程: 未找到设备，重试 {}/5...", attempt + 1);
                thread::sleep(std::time::Duration::from_millis(500));
            }

            let device = match device {
                Some(d) => d,
                None => {
                    eprintln!("音频线程: 无法找到音频输入设备");
                    // 发送空数据保持通道开启
                    loop {
                        match cmd_receiver.recv() {
                            Ok(_) => {
                                let _ = data_sender.try_send(Vec::new());
                                let _ = amp_sender.try_send(0.0);
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

            let final_buffer = Arc::new(std::sync::Mutex::new(Vec::with_capacity(16000 * 60)));
            let sample_count = Arc::new(AtomicUsize::new(0));

            // 重采样参数
            let target_sr = 16000f64;
            let source_sr = config.sample_rate.0 as f64;
            let ratio = target_sr / source_sr; // 输出/输入比率

            println!("音频线程: {}Hz -> {}Hz, ratio={}", source_sr, target_sr, ratio);

            let is_recording_stream = is_recording.clone();
            let final_buffer_stream = final_buffer.clone();
            let sample_count_stream = sample_count.clone();
            let amp_sender_stream = amp_sender.clone();

            // 重采样状态：相位累积
            let mut phase: f64 = 0.0;

            let stream = match device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if !is_recording_stream.load(Ordering::SeqCst) {
                        return;
                    }

                    // 计算振幅用于可视化
                    if !data.is_empty() {
                        let sum: i64 = data.iter().map(|&s| (s as i64).abs()).sum();
                        let avg_amp = (sum as f32 / data.len() as f32) / 32768.0;
                        let _ = amp_sender_stream.try_send(avg_amp.min(1.0));
                    }

                    if (source_sr - target_sr).abs() < 1.0 {
                        // 无需重采样
                        if let Ok(mut buffer) = final_buffer_stream.lock() {
                            let count = sample_count_stream.load(Ordering::Relaxed);
                            let space = MAX_RECORDING_SAMPLES.saturating_sub(count);
                            let to_add = data.len().min(space);
                            if to_add > 0 {
                                buffer.extend_from_slice(&data[..to_add]);
                                sample_count_stream.store(buffer.len(), Ordering::Relaxed);
                            }
                        }
                    } else {
                        // 需要重采样 - 使用相位累积进行高质量重采样
                        if let Ok(mut buffer) = final_buffer_stream.lock() {
                            let input_len = data.len() as f64;

                            while phase < input_len {
                                // 计算插值位置
                                let idx = phase as usize;
                                let frac = phase - idx as f64;

                                if idx + 1 < data.len() {
                                    // 线性插值
                                    let s0 = data[idx] as f64;
                                    let s1 = data[idx + 1] as f64;
                                    let sample = (s0 + frac * (s1 - s0)) as i16;
                                    buffer.push(sample);
                                } else if idx < data.len() {
                                    buffer.push(data[idx]);
                                }

                                phase += 1.0 / ratio; // 按比率前进

                                // 检查是否超过最大长度
                                if buffer.len() >= MAX_RECORDING_SAMPLES {
                                    break;
                                }
                            }

                            // 保存剩余相位，处理进位
                            phase -= input_len;
                            sample_count_stream.store(buffer.len(), Ordering::Relaxed);
                        }
                    }
                },
                |err| eprintln!("音频流错误: {}", err),
                Some(std::time::Duration::from_millis(1000)),
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("构建音频流失败: {}", e);
                    return;
                }
            };

            let _ = stream.play();
            let _ = stream.pause();
            println!("音频线程: 音频流已准备");

            loop {
                match cmd_receiver.recv() {
                    Ok(AudioCommand::Start) => {
                        if let Ok(mut buffer) = final_buffer.lock() {
                            buffer.clear();
                        }
                        sample_count.store(0, Ordering::Relaxed);
                        phase = 0.0;
                        is_recording.store(true, Ordering::SeqCst);
                        let _ = stream.play();
                        println!("音频线程: 录音开始");
                    }
                    Ok(AudioCommand::Stop) => {
                        is_recording.store(false, Ordering::SeqCst);
                        let _ = stream.pause();

                        let recorded = if let Ok(data) = final_buffer.lock() {
                            data.clone()
                        } else {
                            Vec::new()
                        };

                        println!("音频线程: 录音停止，{} 样本 ({}秒)",
                            recorded.len(), recorded.len() / 16000);

                        if data_sender.try_send(recorded).is_err() {
                            let _ = data_sender.try_send(Vec::new());
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        pub fn is_recording(&self) -> bool {
            self.is_recording.load(Ordering::SeqCst)
        }

        pub fn try_get_amplitude(&self) -> Option<f32> {
            self.amp_receiver.try_recv().ok()
        }

        pub fn start_recording(&self) -> Result<(), String> {
            if self.is_recording() {
                return Ok(());
            }
            self.cmd_sender.send(AudioCommand::Start)
                .map_err(|_| "音频线程通信失败".to_string())
        }

        pub fn stop_recording(&self) -> Result<Vec<i16>, String> {
            if !self.is_recording() {
                return Ok(Vec::new());
            }

            while self.data_receiver.try_recv().is_ok() {}

            self.cmd_sender.send(AudioCommand::Stop)
                .map_err(|e| format!("发送停止命令失败: {}", e))?;

            match self.data_receiver.recv_timeout(std::time::Duration::from_secs(10)) {
                Ok(data) => Ok(data),
                Err(_) => Err("接收音频数据超时".to_string()),
            }
        }

        pub fn save_to_wav(&self, data: &[i16], path: &PathBuf) -> Result<(), String> {
            if data.is_empty() {
                return Err("没有音频数据".to_string());
            }

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
        Ok(text) => {
            // 自动输入到当前焦点窗口
            println!("识别结果: {}，正在自动输入...", text);
            auto_type_text(&text);
            text
        }
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
    // 将 Windows 反斜杠路径转换为正斜杠，避免 JSON 转义问题
    let audio_path_str = wav_path.to_string_lossy().to_string().replace("\\", "/");
    let request_body = serde_json::json!({
        "audioPath": audio_path_str
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
        // 将路径转换为正斜杠避免转义问题
        let cli_path = std::env::current_dir()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("autotype").join("cli")
            .to_string_lossy().to_string().replace("\\", "/");
        let audio_path = wav_path.to_string_lossy().to_string().replace("\\", "/");

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
                cli_path,
                audio_path
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

// 获取音频振幅（用于声纹可视化）
#[tauri::command]
fn get_audio_amplitude(state: State<AppState>) -> Result<f32, String> {
    let recorder = state.recorder.lock().map_err(|e| e.to_string())?;
    Ok(recorder.try_get_amplitude().unwrap_or(0.0))
}

// 自动输入文本到当前焦点窗口（Windows API）
#[cfg(target_os = "windows")]
fn auto_type_text(text: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // 将文本转换为宽字符
    let wide: Vec<u16> = OsStr::new(text).encode_wide().chain(Some(0)).collect();

    for chunk in wide.chunks(32) {
        if chunk.is_empty() {
            break;
        }

        // 使用 SendInput 发送按键
        let mut inputs: Vec<windows::Win32::UI::Input::KeyboardAndMouse::INPUT> = Vec::new();

        for &c in chunk {
            if c == 0 {
                continue;
            }

            // 按键按下
            let mut input_down = windows::Win32::UI::Input::KeyboardAndMouse::INPUT::default();
            input_down.r#type = windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD;
            input_down.Anonymous.ki = windows::Win32::UI::Input::KeyboardAndMouse::KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                wScan: c,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE,
                time: 0,
                dwExtraInfo: 0,
            };
            inputs.push(input_down);

            // 按键释放
            let mut input_up = windows::Win32::UI::Input::KeyboardAndMouse::INPUT::default();
            input_up.r#type = windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD;
            input_up.Anonymous.ki = windows::Win32::UI::Input::KeyboardAndMouse::KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                wScan: c,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE | windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            };
            inputs.push(input_up);
        }

        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SendInput(
                &inputs,
                std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
            );
        }

        // 小延迟确保输入顺序
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(not(target_os = "windows"))]
fn auto_type_text(_text: &str) {
    // 非 Windows 平台暂不支持
}

// 打开录音文件所在目录
#[tauri::command]
fn open_recording_folder(state: State<AppState>, id: String) -> Result<(), String> {
    let recordings_dir = state.recordings_dir.lock().map_err(|e| e.to_string())?;
    let wav_path = recordings_dir.join(format!("{}.wav", id));

    if !wav_path.exists() {
        return Err("录音文件不存在".to_string());
    }

    // 获取文件所在目录
    let folder_path = wav_path.parent()
        .ok_or("无法获取文件目录")?;

    // 使用系统默认程序打开目录
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .args([folder_path])
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args([folder_path])
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .args([folder_path])
            .spawn();
    }

    Ok(())
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
            check_audio_devices,
            open_recording_folder,
            get_audio_amplitude
        ])
        .setup(|app| {
            // 注册全局快捷键 F4
            let handle = app.handle();
            let mut shortcut_manager = app.global_shortcut_manager();

            // 尝试注册 F4，失败则尝试 F2/F3 作为备用
            let f4_result = shortcut_manager.register("F4", move || {
                println!("快捷键 F4 被按下");
                let state: State<AppState> = handle.state();
                match toggle_recording(state) {
                    Ok(is_recording) => {
                        println!("快捷键 F4: 录音状态切换为 {}", is_recording);
                        // 发送事件到前端更新 UI
                        if let Err(e) = handle.emit_all("recording-status-change", serde_json::json!({
                            "recording": is_recording
                        })) {
                            eprintln!("发送事件失败: {}", e);
                        } else {
                            println!("事件已发送: recording-status-change, recording={}", is_recording);
                        }
                    }
                    Err(e) => {
                        eprintln!("快捷键 F4: 切换录音状态失败: {}", e);
                    }
                }
            });

            match f4_result {
                Ok(_) => println!("✓ 全局快捷键 F4 已注册"),
                Err(e) => {
                    println!("✗ 无法注册 F4: {}", e);
                    println!("  尝试注册 F2 作为备用...");

                    let handle2 = app.handle();
                    match app.global_shortcut_manager().register("F2", move || {
                        let state: State<AppState> = handle2.state();
                        if let Ok(is_recording) = toggle_recording(state) {
                            println!("快捷键 F2 触发，录音状态: {}", is_recording);
                            let _ = handle2.emit_all("recording-status-change", serde_json::json!({
                                "recording": is_recording
                            }));
                        }
                    }) {
                        Ok(_) => println!("✓ 全局快捷键 F2 已注册（备用）"),
                        Err(e2) => {
                            println!("✗ 也无法注册 F2: {}", e2);
                            println!("  尝试注册 F3...");

                            let handle3 = app.handle();
                            let _ = app.global_shortcut_manager().register("F3", move || {
                                let state: State<AppState> = handle3.state();
                                if let Ok(is_recording) = toggle_recording(state) {
                                    println!("快捷键 F3 触发，录音状态: {}", is_recording);
                                    let _ = handle3.emit_all("recording-status-change", serde_json::json!({
                                        "recording": is_recording
                                    }));
                                }
                            });
                        }
                    }
                }
            }

            // 创建桌面悬浮窗口（用于显示声纹）
            let overlay_window = tauri::WindowBuilder::new(
                app,
                "overlay",
                tauri::WindowUrl::App("/overlay.html".into())
            )
            .title("")
            .inner_size(300.0, 100.0)
            .position(100.0, 100.0)
            .always_on_top(true)
            .transparent(true)
            .decorations(false)
            .skip_taskbar(true)
            .resizable(false)
            .visible(false)
            .build();

            if let Ok(overlay) = overlay_window {
                let overlay_handle = overlay.clone();
                let app_handle = app.handle();

                // 监听录音状态变化，控制悬浮窗口显示
                std::thread::spawn(move || {
                    let mut was_recording = false;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(100));

                        let state: State<AppState> = app_handle.state();
                        let is_recording = state.is_recording.lock()
                            .map(|v| *v)
                            .unwrap_or(false);

                        if is_recording != was_recording {
                            was_recording = is_recording;
                            if is_recording {
                                let _ = overlay_handle.show();
                                let _ = overlay_handle.set_focus();
                            } else {
                                let _ = overlay_handle.hide();
                            }
                        }

                        // 如果正在录音，发送振幅数据到悬浮窗口
                        if is_recording {
                            if let Ok(recorder) = state.recorder.lock() {
                                if let Some(amp) = recorder.try_get_amplitude() {
                                    let _ = overlay_handle.emit("audio-amplitude", amp);
                                }
                            }
                        }
                    }
                });
            } else if let Err(e) = overlay_window {
                eprintln!("创建悬浮窗口失败: {}", e);
            }

            // 使用 rdev 监听 F4 按住/松开（实现按住录音模式）
            let handle = app.handle();
            std::thread::spawn(move || {
                use rdev::{listen, EventType, Key};

                let handle_for_keydown = handle.clone();
                let handle_for_keyup = handle.clone();
                let mut is_pressed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let is_pressed_clone = is_pressed.clone();

                if let Err(e) = listen(move |event| {
                    match event.event_type {
                        EventType::KeyPress(Key::F4) => {
                            if !is_pressed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                // F4 刚按下，开始录音
                                println!("rdev: F4 按下，开始录音");
                                let state: State<AppState> = handle_for_keydown.state();
                                let is_recording = state.is_recording.lock().map(|v| *v).unwrap_or(false);
                                if !is_recording {
                                    let recorder = state.recorder.lock().unwrap();
                                    if recorder.start_recording().is_ok() {
                                        drop(recorder);
                                        *state.is_recording.lock().unwrap() = true;
                                        let _ = handle_for_keydown.emit_all("recording-status-change",
                                            serde_json::json!({"recording": true }));
                                    }
                                }
                            }
                        }
                        EventType::KeyRelease(Key::F4) => {
                            if is_pressed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                                // F4 松开，停止录音
                                println!("rdev: F4 松开，停止录音");
                                let state: State<AppState> = handle_for_keyup.state();
                                let is_recording = state.is_recording.lock().map(|v| *v).unwrap_or(false);
                                if is_recording {
                                    let recorder = state.recorder.lock().unwrap();
                                    let audio_data = recorder.stop_recording().unwrap_or_default();
                                    drop(recorder);
                                    *state.is_recording.lock().unwrap() = false;

                                    // 保存录音
                                    if !audio_data.is_empty() {
                                        let recordings_dir = state.recordings_dir.lock().unwrap();
                                        let id = Local::now().format("%Y%m%d_%H%M%S").to_string();
                                        let wav_path = recordings_dir.join(format!("{}.wav", id));
                                        let _ = state.recorder.lock().unwrap().save_to_wav(&audio_data, &wav_path);
                                        let txt_path = recordings_dir.join(format!("{}.txt", id));
                                        let _ = std::fs::write(&txt_path, "[识别中...]");
                                        drop(recordings_dir);

                                        // 触发识别
                                        let id_clone = id.clone();
                                        let handle_clone = handle_for_keyup.clone();
                                        std::thread::spawn(move || {
                                            std::thread::sleep(std::time::Duration::from_millis(500));
                                            let state: State<AppState> = handle_clone.state();
                                            let _ = transcribe_recording(state, id_clone);
                                            let _ = handle_clone.emit_all("recording-saved",
                                                serde_json::json!({"id": id }));
                                        });
                                    }

                                    let _ = handle_for_keyup.emit_all("recording-status-change",
                                        serde_json::json!({"recording": false }));
                                }
                            }
                        }
                        _ => {}
                    }
                }) {
                    eprintln!("rdev 监听失败: {:?}", e);
                }
            });

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
