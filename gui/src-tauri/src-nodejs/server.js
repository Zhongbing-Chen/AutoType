#!/usr/bin/env node
/**
 * VocoType ASR Service - Node.js 代理服务
 * 调用 Python FunASR 进行语音识别
 */

const express = require('express');
const cors = require('cors');
const multer = require('multer');
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const util = require('util');

const app = express();
const PORT = process.env.ASR_PORT || 10095;

// 启用 CORS 和 JSON 解析
app.use(cors());
app.use(express.json({ limit: '50mb' }));

// 确保上传目录存在
const UPLOAD_DIR = path.join(__dirname, 'uploads');
if (!fs.existsSync(UPLOAD_DIR)) {
  fs.mkdirSync(UPLOAD_DIR, { recursive: true });
}

// Python 服务配置
const PYTHON_SCRIPT = path.join(__dirname, 'asr_worker.py');
const VOCTYPE_CLI_DIR = path.join(__dirname, '..', '..', '..', 'cli');

// 检查 Python 环境
let pythonAvailable = false;
let pythonCmd = 'python';

async function checkPython() {
  // 首先尝试虚拟环境 Python
  const venvPython = path.join(VOCTYPE_CLI_DIR, '.venv', 'Scripts', 'python.exe');
  const cmds = [venvPython, 'python', 'python3', 'py'];

  for (const cmd of cmds) {
    try {
      const result = spawn(cmd, ['--version']);
      await new Promise((resolve, reject) => {
        result.on('close', (code) => {
          if (code === 0) {
            pythonCmd = cmd;
            pythonAvailable = true;
            resolve();
          } else {
            reject();
          }
        });
        result.on('error', reject);
      });
      break;
    } catch (e) {
      continue;
    }
  }

  console.log(`[INFO] Python: ${pythonAvailable ? pythonCmd : '未找到'}`);
  if (pythonCmd === venvPython) {
    console.log(`[INFO] 使用虚拟环境 Python`);
  }
  return pythonAvailable;
}

/**
 * 调用 Python FunASR 进行转录
 */
function transcribeWithPython(audioPath) {
  return new Promise((resolve, reject) => {
    const pythonCode = `
import sys
import json
import os

# 添加 vocotype-cli 到路径
sys.path.insert(0, r'${VOCTYPE_CLI_DIR}')
os.chdir(r'${VOCTYPE_CLI_DIR}')

try:
    from app.funasr_server import FunASRServer

    server = FunASRServer()
    init_result = server.initialize()

    if not init_result.get('success'):
        print(json.dumps({
            'success': False,
            'error': init_result.get('error', '初始化失败')
        }))
        sys.exit(1)

    result = server.transcribe_audio(r'${audioPath}')
    print(json.dumps(result))

except Exception as e:
    import traceback
    print(json.dumps({
        'success': False,
        'error': str(e),
        'traceback': traceback.format_exc()
    }))
    sys.exit(1)
`;

    const python = spawn(pythonCmd, ['-c', pythonCode], {
      env: {
        ...process.env,
        PYTHONIOENCODING: 'utf-8'
      }
    });

    let stdout = '';
    let stderr = '';

    python.stdout.on('data', (data) => {
      stdout += data.toString();
    });

    python.stderr.on('data', (data) => {
      stderr += data.toString();
    });

    python.on('close', (code) => {
      if (stderr) {
        console.log('[PYTHON STDERR]', stderr);
      }

      if (code !== 0) {
        reject(new Error(`Python 进程退出码 ${code}: ${stderr}`));
        return;
      }

      try {
        // 找到 JSON 输出（可能在多行输出的最后一行）
        const lines = stdout.trim().split('\n');
        let result = null;

        for (let i = lines.length - 1; i >= 0; i--) {
          try {
            result = JSON.parse(lines[i]);
            break;
          } catch (e) {
            continue;
          }
        }

        if (!result) {
          throw new Error('无法解析 Python 输出');
        }

        resolve(result);
      } catch (err) {
        reject(new Error(`解析结果失败: ${err.message}\n原始输出: ${stdout}`));
      }
    });

    python.on('error', (err) => {
      reject(new Error(`启动 Python 失败: ${err.message}`));
    });
  });
}

// Multer 配置
const storage = multer.diskStorage({
  destination: (req, file, cb) => {
    cb(null, UPLOAD_DIR);
  },
  filename: (req, file, cb) => {
    const uniqueSuffix = Date.now() + '-' + Math.round(Math.random() * 1E9);
    cb(null, uniqueSuffix + path.extname(file.originalname));
  }
});

const upload = multer({
  storage,
  limits: { fileSize: 50 * 1024 * 1024 },
  fileFilter: (req, file, cb) => {
    const allowed = ['.wav', '.mp3', '.ogg', '.m4a', '.webm'];
    const ext = path.extname(file.originalname).toLowerCase();
    if (allowed.includes(ext)) {
      cb(null, true);
    } else {
      cb(new Error('不支持的音频格式'));
    }
  }
});

// 健康检查端点
app.get('/', (req, res) => {
  res.json({
    status: 'ok',
    service: 'VocoType ASR Service (Node.js + Python FunASR)',
    version: '1.0.0',
    python: {
      available: pythonAvailable,
      command: pythonCmd
    }
  });
});

// 上传转录端点
app.post('/api/transcribe', upload.single('audio'), async (req, res) => {
  if (!pythonAvailable) {
    return res.status(503).json({
      success: false,
      error: 'Python 环境未就绪'
    });
  }

  if (!req.file) {
    return res.status(400).json({
      success: false,
      error: '未上传音频文件'
    });
  }

  const audioPath = req.file.path;
  console.log('[INFO] 处理音频:', audioPath);

  try {
    const result = await transcribeWithPython(audioPath);

    // 清理临时文件
    try {
      fs.unlinkSync(audioPath);
    } catch (e) {
      console.warn('[WARN] 清理文件失败:', e.message);
    }

    res.json(result);
  } catch (err) {
    console.error('[ERROR] 转录失败:', err);

    // 清理临时文件
    try {
      fs.unlinkSync(audioPath);
    } catch (e) {}

    res.status(500).json({
      success: false,
      error: err.message
    });
  }
});

// 本地文件转录（Tauri 调用）
app.post('/api/transcribe/local', async (req, res) => {
  if (!pythonAvailable) {
    return res.status(503).json({
      success: false,
      error: 'Python 环境未就绪'
    });
  }

  const { audioPath } = req.body;

  if (!audioPath || !fs.existsSync(audioPath)) {
    return res.status(400).json({
      success: false,
      error: '音频文件不存在'
    });
  }

  console.log('[INFO] 处理本地音频:', audioPath);

  try {
    const result = await transcribeWithPython(audioPath);
    res.json(result);
  } catch (err) {
    console.error('[ERROR] 本地转录失败:', err);
    res.status(500).json({
      success: false,
      error: err.message
    });
  }
});

// 错误处理
app.use((err, req, res, next) => {
  console.error('[ERROR]', err.message);
  res.status(500).json({
    success: false,
    error: err.message
  });
});

// 启动服务
async function startServer() {
  console.log('='.repeat(50));
  console.log('VocoType ASR Service');
  console.log('='.repeat(50));

  await checkPython();

  if (!pythonAvailable) {
    console.warn('[WARN] Python 未找到，服务将无法进行语音识别');
  } else {
    console.log('[INFO] FunASR 将通过 Python 调用');
  }

  app.listen(PORT, () => {
    console.log(`[INFO] 服务地址: http://localhost:${PORT}`);
    console.log('='.repeat(50));
  });
}

startServer().catch(console.error);
