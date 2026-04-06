import { invoke } from '@tauri-apps/api/tauri';

const recordBtn = document.getElementById('recordBtn');
const statusDiv = document.getElementById('status');
const resultDiv = document.getElementById('result');
const historyList = document.getElementById('historyList');
const recordingsDirDiv = document.getElementById('recordingsDir');

let isRecording = false;
let lastRecordingId = null;

recordBtn?.addEventListener('click', async () => {
  try {
    isRecording = await invoke('toggle_recording');
    updateUI();

    if (!isRecording) {
      // 录音停止后，先刷新历史记录获取新录音ID
      await new Promise(r => setTimeout(r, 500));
      const recordings = await invoke('get_recordings');
      renderHistory(recordings);

      // 自动转录最新的录音
      if (recordings.length > 0) {
        const latest = recordings[0];
        if (latest.text === '[识别中...]' || latest.text === '[无转录结果]') {
          console.log('自动转录:', latest.id);
          lastRecordingId = latest.id;
          resultDiv.textContent = '正在转录音频...';
          await transcribeAudio(latest.id, true);
        }
      }
    }
  } catch (err) {
    console.error('Error:', err);
    statusDiv.textContent = '错误: ' + err;
  }
});

// 转录音频
async function transcribeAudio(id, auto = false) {
  const btn = document.querySelector(`.transcribe-btn[data-id="${id}"]`);
  if (btn) {
    btn.textContent = '识别中...';
    btn.disabled = true;
  }

  try {
    const text = await invoke('transcribe_recording', { id });

    // 更新显示
    const itemDiv = historyList?.querySelector(`[data-id="${id}"]`);
    if (itemDiv) {
      itemDiv.querySelector('.history-text').textContent = text;
    }

    if (auto) {
      resultDiv.textContent = text.startsWith('[') ? '转录完成（无识别结果）' : text;
    }

    if (btn) {
      btn.textContent = text.startsWith('[') ? '识别' : '重新识别';
    }

    return text;
  } catch (err) {
    console.error('识别失败:', err);
    if (btn) btn.textContent = '识别失败';
    if (auto) resultDiv.textContent = '转录失败: ' + err;
    throw err;
  } finally {
    if (btn) btn.disabled = false;
  }
}

function updateUI() {
  if (isRecording) {
    recordBtn.classList.add('recording');
    recordBtn.querySelector('.text').textContent = '停止录音';
    statusDiv.textContent = '正在录音... (按 F2 停止)';
    resultDiv.textContent = '正在聆听您的声音...';
  } else {
    recordBtn.classList.remove('recording');
    recordBtn.querySelector('.text').textContent = '开始录音';
    statusDiv.textContent = '就绪 (按 F2 开始)';
    resultDiv.textContent = '点击按钮或按 F2 开始录音';
  }
}

// 加载历史记录
async function loadRecordings() {
  try {
    const recordings = await invoke('get_recordings');
    renderHistory(recordings);
  } catch (err) {
    console.error('加载历史记录失败:', err);
  }
}

// 渲染历史记录
function renderHistory(recordings) {
  if (!historyList) return;

  if (recordings.length === 0) {
    historyList.innerHTML = '<div class="empty">暂无录音记录</div>';
    return;
  }

  historyList.innerHTML = recordings.map(item => `
    <div class="history-item" data-id="${item.id}">
      <div class="history-header">
        <span class="timestamp">${item.timestamp}</span>
        <button class="transcribe-btn" data-id="${item.id}">
          ${item.text === '[识别中...]' || item.text === '[无转录结果]' ? '识别' : '重新识别'}
        </button>
      </div>
      <div class="history-text">${item.text || '[无转录结果]'}</div>
      <div class="history-file">${item.audio_file.split('\\').pop()}</div>
    </div>
  `).join('');

  // 绑定识别按钮事件
  historyList.querySelectorAll('.transcribe-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      const id = e.target.dataset.id;
      await transcribeAudio(id, false);
    });
  });
}

// 获取保存目录
async function loadRecordingsDir() {
  try {
    const dir = await invoke('get_recordings_dir');
    if (recordingsDirDiv) {
      recordingsDirDiv.textContent = `录音保存位置: ${dir}`;
    }
  } catch (err) {
    console.error('获取目录失败:', err);
  }
}

// 检测音频设备
const checkAudioBtn = document.getElementById('checkAudioBtn');
const audioDevicesDiv = document.getElementById('audioDevices');

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

// 初始化
async function init() {
  console.log('VocoType 初始化...');

  // 加载历史记录
  await loadRecordings();
  await loadRecordingsDir();

  // 定时刷新历史记录
  setInterval(loadRecordings, 5000);
}

init();
