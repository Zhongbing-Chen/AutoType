import { invoke } from '@tauri-apps/api/tauri';
import { open } from '@tauri-apps/api/dialog';
import { listen } from '@tauri-apps/api/event';

// DOM 元素
const recordingIndicator = document.getElementById('recordingIndicator');
const resultDiv = document.getElementById('result');
const copyBtn = document.getElementById('copyBtn');
const historyList = document.getElementById('historyList');
const waveformContainer = document.getElementById('waveform');
const waveformCanvas = document.getElementById('waveformCanvas');

// 设置相关 DOM
const settingsBtn = document.getElementById('settingsBtn');
const settingsPanel = document.getElementById('settingsPanel');
const closeSettings = document.getElementById('closeSettings');
const overlay = document.getElementById('overlay');
const saveSettingsBtn = document.getElementById('saveSettingsBtn');
const resetSettingsBtn = document.getElementById('resetSettingsBtn');
const hotkeyInput = document.getElementById('hotkeyInput');
const changeHotkeyBtn = document.getElementById('changeHotkeyBtn');
const saveRecordingsCheck = document.getElementById('saveRecordingsCheck');
const autoTranscribeCheck = document.getElementById('autoTranscribeCheck');
const recordingsPathInput = document.getElementById('recordingsPathInput');
const browsePathBtn = document.getElementById('browsePathBtn');
const checkAudioBtn = document.getElementById('checkAudioBtn');
const audioDevicesDiv = document.getElementById('audioDevices');

// 状态
let isRecording = false;
let lastRecordingId = null;
let currentConfig = null;
let lastTranscribedText = '';

// 监听 Rust 发来的录音状态变化 (通过 Tauri 事件)
// 设置 Tauri 事件监听
async function setupEventListeners() {
  await listen('recording-status-change', async (event) => {
    const { recording } = event.payload;
    isRecording = recording;
    updateRecordingUI();

    if (!recording) {
      // 录音停止
      resultDiv.textContent = '正在识别...';
      copyBtn.style.display = 'none';

      // 等待一小段时间让后端保存文件，然后触发识别
      setTimeout(async () => {
        await loadRecordings();
        // 识别最新的一条（触发自动输入）
        const recordings = await invoke('get_recordings');
        if (recordings.length > 0) {
          const latest = recordings[0];
          if (latest.text === '[识别中...]') {
            const text = await transcribeAudio(latest.id, true);
            lastTranscribedText = text;
          } else {
            lastTranscribedText = latest.text;
            resultDiv.textContent = latest.text.startsWith('[') ? '识别完成' : latest.text;
            if (!latest.text.startsWith('[')) {
              copyBtn.style.display = 'inline-block';
            }
          }
        }
      }, 300);
    } else {
      resultDiv.textContent = '正在聆听...';
      copyBtn.style.display = 'none';
    }
  });
}

// 声纹可视化
let amplitudeInterval = null;
const amplitudeHistory = new Array(30).fill(0); // 保持最近30个振幅值

function startWaveformVisualization() {
  if (!waveformCanvas || !waveformContainer) return;

  waveformContainer.style.display = 'block';
  const ctx = waveformCanvas.getContext('2d');
  const width = waveformCanvas.width;
  const height = waveformCanvas.height;

  // 定期获取振幅数据
  amplitudeInterval = setInterval(async () => {
    if (!isRecording) return;

    try {
      const amp = await invoke('get_audio_amplitude');
      // 添加到历史，移除最旧的
      amplitudeHistory.push(amp);
      amplitudeHistory.shift();
    } catch (e) {
      // 忽略错误
    }
  }, 50); // 20fps

  // 动画循环
  function draw() {
    if (!isRecording) {
      ctx.clearRect(0, 0, width, height);
      return;
    }

    ctx.clearRect(0, 0, width, height);

    // 绘制声纹
    const barWidth = width / amplitudeHistory.length;
    const centerY = height / 2;

    amplitudeHistory.forEach((amp, i) => {
      // 添加一些动画效果
      const animatedAmp = amp * (0.8 + Math.random() * 0.4);
      const barHeight = animatedAmp * height * 0.8;

      const x = i * barWidth;
      const y = centerY - barHeight / 2;

      // 渐变色
      const gradient = ctx.createLinearGradient(0, 0, 0, height);
      gradient.addColorStop(0, '#3b82f6');
      gradient.addColorStop(0.5, '#8b5cf6');
      gradient.addColorStop(1, '#3b82f6');

      ctx.fillStyle = gradient;
      ctx.fillRect(x + 1, y, barWidth - 2, barHeight);
    });

    requestAnimationFrame(draw);
  }

  draw();
}

function stopWaveformVisualization() {
  if (amplitudeInterval) {
    clearInterval(amplitudeInterval);
    amplitudeInterval = null;
  }
  if (waveformContainer) {
    waveformContainer.style.display = 'none';
  }
  // 清空历史
  amplitudeHistory.fill(0);
}

// 更新录音 UI
function updateRecordingUI() {
  if (isRecording) {
    recordingIndicator.classList.add('recording');
    recordingIndicator.querySelector('.recording-status').textContent = '正在录音';
    recordingIndicator.querySelector('.recording-hint').textContent = '点击或按 F4 停止录音';
    startWaveformVisualization();
  } else {
    recordingIndicator.classList.remove('recording');
    recordingIndicator.querySelector('.recording-status').textContent = '就绪';
    recordingIndicator.querySelector('.recording-hint').textContent = '点击或按 F4 开始/停止录音';
    stopWaveformVisualization();
  }
}

// 点击录音按钮切换录音状态
async function toggleRecording() {
  try {
    const newState = await invoke('toggle_recording');
    isRecording = newState;
    updateRecordingUI();

    if (!isRecording) {
      // 录音停止，准备识别
      resultDiv.textContent = '正在识别...';
      copyBtn.style.display = 'none';

      // 等待识别完成
      setTimeout(async () => {
        await loadRecordings();
        // 识别最新的一条（触发自动输入）
        const recordings = await invoke('get_recordings');
        if (recordings.length > 0) {
          const latest = recordings[0];
          if (latest.text === '[识别中...]') {
            const text = await transcribeAudio(latest.id, true);
            lastTranscribedText = text;
          } else {
            lastTranscribedText = latest.text;
            resultDiv.textContent = latest.text.startsWith('[') ? '识别完成' : latest.text;
            if (!latest.text.startsWith('[')) {
              copyBtn.style.display = 'inline-block';
            }
          }
        }
      }, 300);
    } else {
      resultDiv.textContent = '正在聆听...';
      copyBtn.style.display = 'none';
    }
  } catch (err) {
    console.error('切换录音状态失败:', err);
    alert('录音失败: ' + err);
  }
}

// 绑定录音按钮点击事件
recordingIndicator?.addEventListener('click', toggleRecording);

// 转录音频
async function transcribeAudio(id, auto = false) {
  const btn = document.querySelector(`.transcribe-btn[data-id="${id}"]`);
  if (btn) {
    btn.textContent = '识别中...';
    btn.disabled = true;
  }

  try {
    const text = await invoke('transcribe_recording', { id });

    const itemDiv = historyList?.querySelector(`[data-id="${id}"]`);
    if (itemDiv) {
      itemDiv.querySelector('.history-text').textContent = text;
    }

    if (auto) {
      resultDiv.textContent = text.startsWith('[') ? '识别完成（无结果）' : text;
      if (!text.startsWith('[')) {
        copyBtn.style.display = 'inline-block';
      }
    }

    if (btn) {
      btn.textContent = text.startsWith('[') ? '识别' : '重新识别';
    }

    return text;
  } catch (err) {
    console.error('识别失败:', err);
    if (btn) btn.textContent = '识别失败';
    if (auto) resultDiv.textContent = '识别失败: ' + err;
    throw err;
  } finally {
    if (btn) btn.disabled = false;
  }
}

// 加载历史记录
async function loadRecordings() {
  try {
    console.log('正在加载历史记录...');
    const recordings = await invoke('get_recordings');
    console.log('获取到历史记录:', recordings.length, '条');
    renderHistory(recordings);
  } catch (err) {
    console.error('加载历史记录失败:', err);
    alert('加载历史记录失败: ' + err);
  }
}

// 渲染历史记录
function renderHistory(recordings) {
  console.log('renderHistory called with', recordings.length, 'items');
  // 调试：显示在页面上
  if (historyList) {
    historyList.setAttribute('data-debug-count', recordings.length);
  }

  if (!historyList) {
    console.error('historyList element not found!');
    alert('错误：找不到 historyList 元素');
    return;
  }

  if (recordings.length === 0) {
    console.log('No recordings, showing empty message');
    historyList.innerHTML = '<div class="empty">暂无录音记录 (debug: 0 items)</div>';
    return;
  }

  // 调试信息
  const debugHeader = `<div style="color: #3b82f6; padding: 8px; border-bottom: 1px solid #334155; margin-bottom: 8px;">找到 ${recordings.length} 条录音记录</div>`;

  historyList.innerHTML = debugHeader + recordings.map(item => `
    <div class="history-item" data-id="${item.id}">
      <div class="history-header">
        <span class="timestamp">${item.timestamp}</span>
        <div class="history-actions">
          <button class="transcribe-btn" data-id="${item.id}">
            ${item.text === '[识别中...]' || item.text === '[无转录结果]' ? '识别' : '重新识别'}
          </button>
          <button class="open-folder-btn" data-id="${item.id}" title="打开文件夹">📁</button>
          <button class="delete-btn" data-id="${item.id}" title="删除">🗑️</button>
        </div>
      </div>
      <div class="history-text">${item.text || '[无转录结果]'}</div>
      <div class="history-file">${item.audio_file.split('\\').pop()}</div>
    </div>
  `).join('');

  // 绑定识别按钮
  historyList.querySelectorAll('.transcribe-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      const id = e.target.dataset.id;
      await transcribeAudio(id, false);
    });
  });

  // 绑定打开文件夹按钮
  historyList.querySelectorAll('.open-folder-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      const id = e.target.dataset.id;
      try {
        await invoke('open_recording_folder', { id });
      } catch (err) {
        console.error('打开文件夹失败:', err);
        alert('打开文件夹失败: ' + err);
      }
    });
  });

  // 绑定删除按钮
  historyList.querySelectorAll('.delete-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      const id = e.target.dataset.id;
      if (confirm('确定要删除这条录音吗？')) {
        try {
          await invoke('delete_recording', { id });
          await loadRecordings();
        } catch (err) {
          alert('删除失败: ' + err);
        }
      }
    });
  });
}

// 复制文本
copyBtn?.addEventListener('click', async () => {
  if (lastTranscribedText && !lastTranscribedText.startsWith('[')) {
    try {
      await navigator.clipboard.writeText(lastTranscribedText);
      copyBtn.textContent = '已复制!';
      setTimeout(() => {
        copyBtn.textContent = '复制文本';
      }, 1500);
    } catch (err) {
      console.error('复制失败:', err);
    }
  }
});

// ========== 设置面板 ==========

// 打开设置
function openSettings() {
  settingsPanel.classList.add('open');
  overlay.classList.add('show');
  loadSettings();
}

// 关闭设置
function closeSettingsPanel() {
  settingsPanel.classList.remove('open');
  overlay.classList.remove('show');
}

// 加载设置
async function loadSettings() {
  try {
    currentConfig = await invoke('get_config');

    hotkeyInput.value = currentConfig.hotkey || 'F4';
    saveRecordingsCheck.checked = currentConfig.save_recordings !== false;
    autoTranscribeCheck.checked = currentConfig.auto_transcribe !== false;
    recordingsPathInput.value = currentConfig.recordings_dir || '';
  } catch (err) {
    console.error('加载设置失败:', err);
  }
}

// 保存设置
async function saveSettings() {
  try {
    const newConfig = {
      ...currentConfig,
      hotkey: hotkeyInput.value || 'F4',
      save_recordings: saveRecordingsCheck.checked,
      auto_transcribe: autoTranscribeCheck.checked,
      recordings_dir: recordingsPathInput.value,
    };

    await invoke('update_config', { newConfig });
    currentConfig = newConfig;

    // 显示保存成功提示
    const btn = saveSettingsBtn;
    const originalText = btn.textContent;
    btn.textContent = '已保存!';
    btn.disabled = true;
    setTimeout(() => {
      btn.textContent = originalText;
      btn.disabled = false;
    }, 1500);
  } catch (err) {
    alert('保存设置失败: ' + err);
  }
}

// 恢复默认设置
async function resetSettings() {
  if (confirm('确定要恢复默认设置吗？')) {
    try {
      const defaultConfig = {
        hotkey: 'F4',
        hold_to_record: true,
        save_recordings: true,
        recordings_dir: '',
        auto_transcribe: true,
      };

      // 使用系统默认路径
      const defaultDir = await invoke('get_recordings_dir');
      defaultConfig.recordings_dir = defaultDir;

      await invoke('update_config', { newConfig: defaultConfig });
      await loadSettings();
    } catch (err) {
      console.error('恢复默认设置失败:', err);
    }
  }
}

// 修改快捷键
let isCapturingHotkey = false;
changeHotkeyBtn?.addEventListener('click', () => {
  if (isCapturingHotkey) return;

  isCapturingHotkey = true;
  hotkeyInput.value = '请按快捷键...';
  hotkeyInput.focus();

  const captureKey = (e) => {
    e.preventDefault();

    const key = e.key;
    if (key && key !== 'Escape') {
      hotkeyInput.value = key.length === 1 ? key.toUpperCase() : key;
    }

    isCapturingHotkey = false;
    document.removeEventListener('keydown', captureKey);
  };

  document.addEventListener('keydown', captureKey);
});

// 浏览路径
browsePathBtn?.addEventListener('click', async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: recordingsPathInput.value,
    });

    if (selected) {
      recordingsPathInput.value = selected;
    }
  } catch (err) {
    console.error('选择路径失败:', err);
  }
});

// 检测音频设备
checkAudioBtn?.addEventListener('click', async () => {
  checkAudioBtn.textContent = '检测中...';
  checkAudioBtn.disabled = true;
  audioDevicesDiv.textContent = '正在检测音频设备...';

  try {
    const result = await invoke('check_audio_devices');
    audioDevicesDiv.textContent = result;
  } catch (err) {
    audioDevicesDiv.textContent = `检测失败: ${err}`;
  } finally {
    checkAudioBtn.textContent = '检测音频设备';
    checkAudioBtn.disabled = false;
  }
});

// 绑定设置面板事件
settingsBtn?.addEventListener('click', openSettings);
closeSettings?.addEventListener('click', closeSettingsPanel);
overlay?.addEventListener('click', closeSettingsPanel);
saveSettingsBtn?.addEventListener('click', saveSettings);
resetSettingsBtn?.addEventListener('click', resetSettings);

// 初始化
async function init() {
  console.log('AutoType 初始化...');

  // 设置事件监听
  await setupEventListeners();

  await loadSettings();
  await loadRecordings();

  // 定时刷新历史记录
  setInterval(loadRecordings, 5000);

  console.log('AutoType 初始化完成');
}

init();
