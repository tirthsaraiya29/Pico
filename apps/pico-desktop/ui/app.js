// PICO Real-Time 3D Intelligence Engine Frontend

let scene, camera, renderer, coreMesh, particleSystem, wireframeMesh;
let coreState = 'IDLE'; // 'IDLE', 'LISTENING', 'THINKING', 'SPEAKING', 'TOOL'
let targetScale = 1.0;
let targetColor = new THREE.Color(0x00f0ff);
let socket = null;
let activeTurnText = null;

// Initialize 3D Engine
function initThree() {
  const container = document.getElementById('canvas-container');
  const width = container.clientWidth;
  const height = container.clientHeight;

  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(45, width / height, 0.1, 1000);
  camera.position.z = 5;

  renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setSize(width, height);
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  container.appendChild(renderer.domElement);

  // 1. Central Geodesic Sphere
  const geometry = new THREE.IcosahedronGeometry(1.2, 3);
  const material = new THREE.MeshBasicMaterial({
    color: 0x00f0ff,
    wireframe: true,
    transparent: true,
    opacity: 0.35,
  });
  wireframeMesh = new THREE.Mesh(geometry, material);
  scene.add(wireframeMesh);

  // 2. Inner Glow Core
  const innerGeo = new THREE.IcosahedronGeometry(0.85, 2);
  const innerMat = new THREE.MeshBasicMaterial({
    color: 0x00e5ff,
    wireframe: false,
    transparent: true,
    opacity: 0.2,
  });
  coreMesh = new THREE.Mesh(innerGeo, innerMat);
  scene.add(coreMesh);

  // 3. Ambient Particle Halo
  const particleGeo = new THREE.BufferGeometry();
  const particleCount = 450;
  const posArray = new Float32Array(particleCount * 3);

  for (let i = 0; i < particleCount * 3; i += 3) {
    const u = Math.random();
    const v = Math.random();
    const theta = u * 2.0 * Math.PI;
    const phi = Math.acos(2.0 * v - 1.0);
    const r = 1.5 + Math.random() * 0.9;
    posArray[i] = r * Math.sin(phi) * Math.cos(theta);
    posArray[i + 1] = r * Math.sin(phi) * Math.sin(theta);
    posArray[i + 2] = r * Math.cos(phi);
  }

  particleGeo.setAttribute('position', new THREE.BufferAttribute(posArray, 3));
  const particleMat = new THREE.PointsMaterial({
    size: 0.035,
    color: 0x00f0ff,
    transparent: true,
    opacity: 0.7,
  });

  particleSystem = new THREE.Points(particleGeo, particleMat);
  scene.add(particleSystem);

  // Window resize handler
  window.addEventListener('resize', onWindowResize);

  // Mouse tilt tracking
  document.addEventListener('mousemove', (e) => {
    const mouseX = (e.clientX / window.innerWidth) * 2 - 1;
    const mouseY = -(e.clientY / window.innerHeight) * 2 + 1;
    camera.position.x += (mouseX * 0.4 - camera.position.x) * 0.05;
    camera.position.y += (mouseY * 0.4 - camera.position.y) * 0.05;
    camera.lookAt(scene.position);
  });

  animate();
}

function onWindowResize() {
  const container = document.getElementById('canvas-container');
  if (!container) return;
  const width = container.clientWidth;
  const height = container.clientHeight;
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
  renderer.setSize(width, height);
}

let clock = new THREE.Clock();
function animate() {
  requestAnimationFrame(animate);
  const elapsedTime = clock.getElapsedTime();

  // Rotations
  let rotSpeed = 0.4;
  if (coreState === 'THINKING') rotSpeed = 2.2;
  else if (coreState === 'TOOL') rotSpeed = 1.6;
  else if (coreState === 'SPEAKING') rotSpeed = 0.8;

  wireframeMesh.rotation.x += 0.005 * rotSpeed;
  wireframeMesh.rotation.y += 0.01 * rotSpeed;
  coreMesh.rotation.x -= 0.007 * rotSpeed;
  coreMesh.rotation.y -= 0.009 * rotSpeed;
  particleSystem.rotation.y += 0.003 * rotSpeed;

  // State transitions: Pulse & Color Interpolation
  let pulse = Math.sin(elapsedTime * 2.5) * 0.08;
  if (coreState === 'LISTENING') pulse = Math.sin(elapsedTime * 6.0) * 0.15;
  if (coreState === 'SPEAKING') pulse = Math.sin(elapsedTime * 8.0) * 0.22;
  if (coreState === 'THINKING') pulse = Math.sin(elapsedTime * 12.0) * 0.05;

  const currentScale = targetScale + pulse;
  wireframeMesh.scale.set(currentScale, currentScale, currentScale);
  coreMesh.scale.set(currentScale * 0.85, currentScale * 0.85, currentScale * 0.85);

  wireframeMesh.material.color.lerp(targetColor, 0.08);
  coreMesh.material.color.lerp(targetColor, 0.08);
  particleSystem.material.color.lerp(targetColor, 0.08);

  renderer.render(scene, camera);
}

// Update 3D Core State
function setCoreState(state) {
  coreState = state;
  const badge = document.getElementById('core-state-label');

  switch (state) {
    case 'LISTENING':
      targetColor = new THREE.Color(0x3b82f6); // Energetic Blue
      targetScale = 1.25;
      badge.textContent = 'LISTENING • VAD ACTIVE';
      break;
    case 'THINKING':
      targetColor = new THREE.Color(0xf59e0b); // Amber
      targetScale = 0.95;
      badge.textContent = 'REASONING • LLM STREAM';
      break;
    case 'TOOL':
      targetColor = new THREE.Color(0x10b981); // Emerald Matrix
      targetScale = 1.15;
      badge.textContent = 'EXECUTING OS TOOL';
      break;
    case 'SPEAKING':
      targetColor = new THREE.Color(0x8b5cf6); // Violet Pulse
      targetScale = 1.35;
      badge.textContent = 'SPEAKING • AUDIO OUT';
      break;
    default:
      targetColor = new THREE.Color(0x00f0ff); // Cyan Idle
      targetScale = 1.0;
      badge.textContent = 'IDLE • READY';
      break;
  }
}

// Audio Oscilloscope Waveform Mock
function initAudioWaveform() {
  const canvas = document.getElementById('waveform-canvas');
  const ctx = canvas.getContext('2d');
  let phase = 0;

  function drawWave() {
    requestAnimationFrame(drawWave);
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    let amp = 3;
    if (coreState === 'SPEAKING') amp = 16;
    else if (coreState === 'LISTENING') amp = 10;
    else if (coreState === 'THINKING') amp = 5;

    ctx.beginPath();
    ctx.lineWidth = 2;
    ctx.strokeStyle = coreState === 'SPEAKING' ? '#8b5cf6' : (coreState === 'TOOL' ? '#10b981' : '#00f0ff');

    for (let x = 0; x < canvas.width; x++) {
      const y = canvas.height / 2 + Math.sin(x * 0.04 + phase) * Math.cos(x * 0.02 + phase * 0.5) * amp;
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    phase += 0.08;

    // VU meter bar
    const vuBar = document.getElementById('vu-bar');
    if (vuBar) {
      const pct = Math.min(100, (amp / 18) * 100);
      vuBar.style.width = pct + '%';
    }
  }
  drawWave();
}

// WebSocket Communication
function initWebSocket() {
  const host = window.location.host;
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = window.PICO_AUTH_TOKEN || (document.querySelector('meta[name="pico-auth-token"]')?.getAttribute('content') || '');
  const tokenQuery = token ? `?token=${encodeURIComponent(token)}` : '';
  socket = new WebSocket(`${protocol}//${host}/ws${tokenQuery}`);

  socket.onopen = () => {
    document.getElementById('runtime-status-text').textContent = 'CORE ONLINE';
  };

  socket.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      handleServerEvent(data);
    } catch (e) {
      console.error('Failed to parse WebSocket message:', e);
    }
  };

  socket.onclose = () => {
    document.getElementById('runtime-status-text').textContent = 'RECONNECTING...';
    setTimeout(initWebSocket, 2000);
  };
}

function handleServerEvent(event) {
  const type = event.type;
  const payload = event.payload;

  switch (type) {
    case 'VoiceActivityStarted':
      setCoreState('LISTENING');
      break;

    case 'VoiceActivityStopped':
      setCoreState('THINKING');
      break;

    case 'TranscriptionFinalized':
      appendUserMessage(payload.text);
      setCoreState('THINKING');
      break;

    case 'InferenceStarted':
      setCoreState('THINKING');
      showCancelButton(true);
      break;

    case 'TokenChunk':
      appendStreamingAssistantChunk(payload.token);
      break;

    case 'InferenceFinished':
      document.getElementById('telemetry-latency').textContent = payload.latency_ms + ' ms';
      showCancelButton(false);
      if (coreState !== 'SPEAKING') setCoreState('IDLE');
      break;

    case 'ToolCallRequested':
      setCoreState('TOOL');
      appendToolCallBadge(payload.call);
      break;

    case 'ToolExecutionFinished':
      appendToolResultBadge(payload.result);
      setCoreState('THINKING');
      break;

    case 'SpeechSynthesisStarted':
    case 'PlaybackStarted':
      setCoreState('SPEAKING');
      showCancelButton(true);
      break;

    case 'PlaybackFinished':
    case 'PlaybackInterrupted':
      showCancelButton(false);
      setCoreState('IDLE');
      break;

    case 'Error':
      appendSystemError(payload.component, payload.message);
      setCoreState('IDLE');
      showCancelButton(false);
      break;
  }
}

// Message Rendering
const messagesContainer = document.getElementById('messages-container');

function appendUserMessage(text) {
  const el = document.createElement('div');
  el.className = 'msg-bubble user';
  el.innerHTML = `
    <div class="msg-avatar">U</div>
    <div class="msg-body">
      <div class="msg-author">USER</div>
      <div class="msg-text">${escapeHtml(text)}</div>
    </div>
  `;
  messagesContainer.appendChild(el);
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
  activeTurnText = null;
}

function appendStreamingAssistantChunk(token) {
  if (!activeTurnText) {
    const el = document.createElement('div');
    el.className = 'msg-bubble assistant';
    el.innerHTML = `
      <div class="msg-avatar">P</div>
      <div class="msg-body">
        <div class="msg-author">PICO CORE</div>
        <div class="msg-text"></div>
      </div>
    `;
    messagesContainer.appendChild(el);
    activeTurnText = el.querySelector('.msg-text');
  }
  activeTurnText.textContent += token;
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

function appendToolCallBadge(call) {
  const el = document.createElement('div');
  el.className = 'tool-card';
  el.innerHTML = `
    <div class="tool-card-header">⚙ RUNNING TOOL: ${escapeHtml(call.name)}</div>
    <div class="tool-card-output">Parameters: ${escapeHtml(JSON.stringify(call.arguments))}</div>
  `;
  messagesContainer.appendChild(el);
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

function appendToolResultBadge(result) {
  const el = document.createElement('div');
  el.className = 'tool-card';
  const statusColor = result.success ? '#10b981' : '#ef4444';
  el.style.borderColor = statusColor;
  el.innerHTML = `
    <div class="tool-card-header" style="color: ${statusColor}">
      <span>✔ RESULT: ${escapeHtml(result.name)}</span>
      <span>${result.execution_duration_ms}ms</span>
    </div>
    <div class="tool-card-output">${escapeHtml(JSON.stringify(result.content, null, 2))}</div>
  `;
  messagesContainer.appendChild(el);
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

function appendSystemError(comp, msg) {
  const el = document.createElement('div');
  el.className = 'msg-bubble assistant';
  el.innerHTML = `
    <div class="msg-avatar" style="color:#ef4444">!</div>
    <div class="msg-body" style="border-color:#ef4444">
      <div class="msg-author" style="color:#ef4444">ERROR • ${escapeHtml(comp)}</div>
      <div class="msg-text">${escapeHtml(msg)}</div>
    </div>
  `;
  messagesContainer.appendChild(el);
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

function escapeHtml(str) {
  if (!str) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function showCancelButton(show) {
  const btn = document.getElementById('btn-cancel');
  if (btn) btn.style.display = show ? 'inline-flex' : 'none';
}

function sendUserTurn(text) {
  if (!text || !socket || socket.readyState !== WebSocket.OPEN) return;
  appendUserMessage(text);
  const speak = document.getElementById('voice-speak-toggle').checked;
  socket.send(JSON.stringify({
    action: 'chat',
    text: text,
    speak: speak
  }));
}

function sendQuickPrompt(prompt) {
  sendUserTurn(prompt);
}

// UI Event Bindings
document.addEventListener('DOMContentLoaded', () => {
  initThree();
  initAudioWaveform();
  initWebSocket();

  const textInput = document.getElementById('text-input');
  const btnSend = document.getElementById('btn-send');
  const btnCancel = document.getElementById('btn-cancel');
  const btnReset = document.getElementById('btn-reset');
  const btnSettings = document.getElementById('btn-settings');
  const btnCloseSettings = document.getElementById('btn-close-settings');
  const settingsDrawer = document.getElementById('settings-drawer');
  const btnMic = document.getElementById('btn-mic');

  btnSend.addEventListener('click', () => {
    const text = textInput.value.trim();
    if (text) {
      sendUserTurn(text);
      textInput.value = '';
    }
  });

  textInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      const text = textInput.value.trim();
      if (text) {
        sendUserTurn(text);
        textInput.value = '';
      }
    }
  });

  btnCancel.addEventListener('click', () => {
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ action: 'cancel' }));
      showCancelButton(false);
      setCoreState('IDLE');
    }
  });

  btnReset.addEventListener('click', () => {
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ action: 'reset_session' }));
      messagesContainer.innerHTML = '';
      appendStreamingAssistantChunk('Session cleared. New conversation initialized.');
    }
  });

  btnSettings.addEventListener('click', () => {
    settingsDrawer.classList.toggle('open');
  });

  btnCloseSettings.addEventListener('click', () => {
    settingsDrawer.classList.remove('open');
  });

  btnMic.addEventListener('click', () => {
    btnMic.classList.toggle('active');
    const isActive = btnMic.classList.contains('active');
    setCoreState(isActive ? 'LISTENING' : 'IDLE');
  });

  // Query diagnostic system status with local auth token
  const token = window.PICO_AUTH_TOKEN || (document.querySelector('meta[name="pico-auth-token"]')?.getAttribute('content') || '');
  const authHeaders = token ? { 'Authorization': `Bearer ${token}` } : {};
  fetch('/api/status', { headers: authHeaders })
    .then(r => r.json())
    .then(data => {
      if (data.system) {
        document.getElementById('device-list-container').innerHTML = `
          <div>Host: ${data.system.hostname} (${data.system.os})</div>
          <div>CPU Cores: ${data.system.cpu_cores}</div>
          <div>Total RAM: ${data.system.total_memory_mb} MB (Free: ${data.system.available_memory_mb} MB)</div>
        `;
      }
    })
    .catch(() => {});
});
