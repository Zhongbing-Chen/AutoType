// 修复后的音频采集模块 - 使用相位累积避免长时间漂移
mod audio {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::StreamConfig;
    use std::sync::atomic::{AtomicBool, Ordering, AtomicU64};
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

            let device = loop {
                if let Some(d) = host.default_input_device() {
                    break d;
                }
                if let Ok(mut devices) = host.input_devices() {
                    if let Some(d) = devices.next() {
                        break d;
                    }
                }
                println!("音频线程: 未找到设备，等待重试...");
                thread::sleep(std::time::Duration::from_millis(500));
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
            let sample_count = Arc::new(std::sync::AtomicUsize::new(0));

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
