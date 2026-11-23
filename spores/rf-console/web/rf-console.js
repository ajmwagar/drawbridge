const $ = (selector) => document.querySelector(selector);

const rootStyle = getComputedStyle(document.documentElement);
const rgbToken = (name) => rootStyle.getPropertyValue(name).trim().split(/[\s,]+/).join(",");
const COLORS = {
  accent: rgbToken("--accent-rgb"),
  accent2: rgbToken("--accent-2-rgb"),
  ink: rgbToken("--ink-rgb"),
  line: rgbToken("--line-rgb"),
  faint: rgbToken("--faint-rgb"),
  scope: rgbToken("--scope-bg-rgb"),
  good: rgbToken("--good-rgb"),
};
const rgba = (name, alpha) => `rgba(${COLORS[name]},${alpha})`;
const MONO_FONT = rootStyle.getPropertyValue("--font-display").trim();

const BINS = 512;
const WATER = [
  [8, 14, 15],
  [10, 24, 24],
  [12, 38, 37],
  [19, 58, 54],
  [38, 86, 77],
  [85, 107, 74],
  [160, 123, 58],
  [232, 169, 75],
];

const ICONS = {
  sdr: `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.35"><path d="M12 20V9"/><path d="M8 20h8"/><path d="M12 9c-2.8 0-5-2.2-5-5"/><path d="M12 9c2.8 0 5-2.2 5-5"/><path d="M12 9c-1.1 0-2-.9-2-2"/><path d="M12 9c1.1 0 2-.9 2-2"/></svg>`,
  packet: `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.35"><path d="m12 3 7 4v10l-7 4-7-4V7l7-4Z"/><path d="m5 7 7 4 7-4"/><path d="M12 11v10"/></svg>`,
  atsc: `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.35"><rect x="4" y="5" width="16" height="12" rx="1"/><path d="M9 21h6"/><path d="M12 17v4"/><path d="m8 9 3 3 3-3"/></svg>`,
};

const DEVICES = [
  {
    id: "hackrf",
    name: "HackRF One",
    code: "hackrf/78d0…",
    kind: "sdr",
    meta: "wideband · 2.0 MS/s",
    noise: -104,
    signals: [
      { freq: 162.55, width: 0.0125, power: -41, label: "NOAA 7" },
      { freq: 162.425, width: 0.0125, power: -63, label: "WX 2" },
      { freq: 162.475, width: 0.0125, power: -71 },
      { freq: 144.39, width: 0.012, power: -58, label: "APRS" },
    ],
    initial: { freq: 162.55, mode: "NFM", bw: 12.5, gain: 22, squelch: -92, span: 2, running: true, recording: false },
  },
  {
    id: "adsb",
    name: "RTL-SDR V4",
    code: "rtl/00000001",
    kind: "sdr",
    meta: "1090 MHz · 2.4 MS/s",
    noise: -102,
    signals: [
      { freq: 1090, width: 0.08, power: -48, label: "MODE-S" },
      { freq: 1090.46, width: 0.12, power: -72 },
    ],
    initial: { freq: 1090, mode: "AM", bw: 200, gain: 38.6, squelch: -80, span: 5, running: true, recording: false },
  },
  {
    id: "ais",
    name: "RTL-SDR V3",
    code: "rtl/00000002",
    kind: "sdr",
    meta: "marine · 2.4 MS/s",
    noise: -106,
    signals: [
      { freq: 161.975, width: 0.025, power: -57, label: "AIS 1" },
      { freq: 162.025, width: 0.025, power: -64, label: "AIS 2" },
    ],
    initial: { freq: 161.975, mode: "NFM", bw: 25, gain: 32, squelch: -96, span: 0.5, running: false, recording: false },
  },
  {
    id: "meshcore",
    name: "MeshCore",
    code: "meshcore/69f5…",
    kind: "packet",
    meta: "packet radio · 350 contacts",
    noise: -110,
    signals: [
      { freq: 915, width: 0.5, power: -62, label: "LORA" },
      { freq: 915.52, width: 0.35, power: -73 },
      { freq: 914.5, width: 0.3, power: -78 },
    ],
    initial: { freq: 915, mode: "NFM", bw: 125, gain: 18, squelch: -100, span: 2, running: true, recording: false },
  },
  {
    id: "hdhr",
    name: "HDHomeRun",
    code: "hdhr/10626E55",
    kind: "atsc",
    meta: "broadcast · 2 tuners",
    noise: -101,
    signals: [
      { freq: 533, width: 5.8, power: -57, label: "ATSC 533" },
      { freq: 539, width: 5.8, power: -64 },
      { freq: 527, width: 5.8, power: -70 },
    ],
    initial: { freq: 533, mode: "AM", bw: 200, gain: 0, squelch: -84, span: 10, running: false, recording: false },
  },
];

const PRESETS = [
  { name: "NOAA WX", device: "hackrf", freq: 162.55, mode: "NFM", bw: 12.5, span: 2, note: "Weather band · carrier centered" },
  { name: "APRS", device: "hackrf", freq: 144.39, mode: "NFM", bw: 12.5, span: 0.5, note: "APRS · 1200 baud AFSK" },
  { name: "AIS", device: "ais", freq: 161.975, mode: "NFM", bw: 25, span: 0.5, note: "Marine AIS · channel 1" },
  { name: "ADS-B", device: "adsb", freq: 1090, mode: "AM", bw: 200, span: 5, note: "1090 MHz · Mode-S / ADS-B" },
  { name: "MeshCore", device: "meshcore", freq: 915, mode: "NFM", bw: 125, span: 2, note: "LoRa packet · 915 MHz" },
];

const runtime = new Map(
  DEVICES.map((device) => [
    device.id,
    {
      ...device.initial,
      note: PRESETS[0].note,
    },
  ]),
);

let activeId = "hackrf";
let hoveredFreq = null;
let dragging = false;
let sessionSeconds = 42 * 60 + 17;

const el = {
  deviceList: $("#device-list"),
  receiverCount: $("#receiver-count"),
  deviceCount: $("#device-count"),
  events: $("#events"),
  receiverTitle: $("#receiver-title"),
  receiverMeta: $("#receiver-meta"),
  span: $("#span"),
  peakHold: $("#peak-hold"),
  rxToggle: $("#rx-toggle"),
  scopeCenter: $("#scope-center"),
  axisA: $("#axis-a"),
  axisB: $("#axis-b"),
  axisC: $("#axis-c"),
  axisD: $("#axis-d"),
  axisE: $("#axis-e"),
  freqMajor: $("#freq-major"),
  tuneNote: $("#tune-note"),
  modeNote: $("#mode-note"),
  modeGrid: $("#mode-grid"),
  bandwidth: $("#bandwidth"),
  bwValue: $("#bw-value"),
  gain: $("#gain"),
  gainValue: $("#gain-value"),
  squelch: $("#squelch"),
  squelchValue: $("#squelch-value"),
  signalValue: $("#signal-value"),
  meterFill: $("#meter-fill"),
  presetList: $("#preset-list"),
  recordToggle: $("#record-toggle"),
  recordNote: $("#record-note"),
  statusLabel: $("#status-label"),
  statusDevice: $("#status-device"),
  statusSession: $("#status-session"),
  clock: $("#utc-clock"),
  spectrum: $("#spectrum"),
  waterfall: $("#waterfall"),
  scopeCard: $(".scope-card"),
  waterfallWrap: $(".waterfall-wrap"),
};

let events = [
  { time: "14:02:11", text: "NOAA 7 carrier acquired on HackRF One" },
  { time: "14:01:48", text: "MeshCore advert heard · 3 hops" },
  { time: "13:59:22", text: "ADS-B track 0x4CA2B1 resolved" },
  { time: "13:57:05", text: "RTL-SDR V3 returned to idle" },
];

const device = () => DEVICES.find((item) => item.id === activeId);
const settings = () => runtime.get(activeId);

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function nowUtc() {
  const date = new Date();
  return [date.getUTCHours(), date.getUTCMinutes(), date.getUTCSeconds()]
    .map((value) => String(value).padStart(2, "0"))
    .join(":");
}

function formatFreq(freq, digits = 3) {
  return Number(freq).toFixed(digits);
}

function formatBandwidth(khz) {
  if (khz >= 1000) return `${(khz / 1000).toFixed(1)} MHz`;
  return `${khz.toFixed(khz % 1 ? 1 : 0)} kHz`;
}

function setSlider(input, value) {
  const min = Number(input.min);
  const max = Number(input.max);
  const ratio = clamp((Number(value) - min) / (max - min), 0, 1);
  input.closest(".slider")?.style.setProperty("--value", String(ratio));
}

function formatAxis(freq) {
  const span = settings().span;
  return formatFreq(freq, span >= 1 ? 2 : 3);
}

function makeDeviceRow(deviceItem) {
  const state = runtime.get(deviceItem.id);
  const active = deviceItem.id === activeId;
  const stateClass = state.running ? "is-rx" : "is-idle";
  const stateText = state.running ? "RX" : "IDLE";
  return `
    <button type="button" class="device-row${active ? " is-active" : ""}" data-device="${deviceItem.id}" role="option" aria-selected="${active}">
      <span class="device-icon">${ICONS[deviceItem.kind]}</span>
      <span class="device-copy">
        <span class="device-name">${deviceItem.name}</span>
        <span class="device-meta">${deviceItem.code}</span>
      </span>
      <span class="device-state ${stateClass}"><span class="status-dot" aria-hidden="true"></span>${stateText}</span>
    </button>
  `;
}

function renderDevices() {
  el.deviceList.innerHTML = DEVICES.map(makeDeviceRow).join("");
  el.receiverCount.textContent = String(DEVICES.length).padStart(2, "0");
  el.deviceCount.textContent = `${DEVICES.length} devices`;
}

function renderEvents() {
  el.events.innerHTML = events
    .map((event) => `<li><time>${event.time}</time><span>${event.text}</span></li>`)
    .join("");
}

function pushEvent(text) {
  events.unshift({ time: nowUtc(), text });
  events = events.slice(0, 5);
  renderEvents();
}

function renderPresets() {
  el.presetList.innerHTML = PRESETS.map(
    (preset) => `
      <button type="button" class="preset" data-preset="${preset.name}">
        <span class="preset-name">${preset.name}</span>
        <span class="preset-freq">${formatFreq(preset.freq)}</span>
      </button>
    `,
  ).join("");
}

function resetSpectrumHistory() {
  peakHoldValues.fill(-130);
  waterfallRows.length = 0;
}

function syncControls() {
  const state = settings();
  const item = device();
  el.receiverTitle.textContent = item.name;
  el.receiverMeta.textContent = `${item.code} · ${item.meta}`;
  el.freqMajor.textContent = formatFreq(state.freq);
  el.span.value = String(state.span);
  el.modeNote.textContent = state.mode;
  el.bandwidth.value = String(state.bw);
  el.bwValue.textContent = formatBandwidth(state.bw);
  el.gain.value = String(state.gain);
  el.gainValue.textContent = `${state.gain.toFixed(state.gain % 1 ? 1 : 0)} dB`;
  el.squelch.value = String(state.squelch);
  el.squelchValue.textContent = `${state.squelch} dBFS`;
  setSlider(el.bandwidth, state.bw);
  setSlider(el.gain, state.gain);
  setSlider(el.squelch, state.squelch);
  el.tuneNote.textContent = state.note || "Simulated receiver · drag to tune";
  el.rxToggle.textContent = state.running ? "Stop RX" : "Start RX";
  el.peakHold.setAttribute("aria-pressed", String(peakHoldEnabled));
  el.recordToggle.setAttribute("aria-pressed", String(state.recording));
  el.recordToggle.textContent = state.recording ? "Stop" : "Record";
  el.recordNote.textContent = state.recording
    ? "SigMF capture · 2.0 MS/s"
    : "No active recording";
  el.statusLabel.textContent = state.running ? "RECEIVING" : "IDLE";
  el.statusDevice.textContent = item.code.split("/")[0].toUpperCase();
  el.scopeCenter.textContent = `${formatFreq(state.freq)} MHz`;
  el.axisA.textContent = formatAxis(state.freq - state.span / 2);
  el.axisB.textContent = formatAxis(state.freq - state.span / 4);
  el.axisC.textContent = formatAxis(state.freq);
  el.axisD.textContent = formatAxis(state.freq + state.span / 4);
  el.axisE.textContent = formatAxis(state.freq + state.span / 2);

  for (const button of el.modeGrid.querySelectorAll("button")) {
    button.classList.toggle("is-active", button.dataset.mode === state.mode);
  }

  renderDevices();
  el.presetList.querySelectorAll("[data-preset]").forEach((button) => {
    const preset = PRESETS.find((item) => item.name === button.dataset.preset);
    button.classList.toggle("is-current", preset?.device === activeId && preset?.freq === state.freq);
  });
}

function setActiveDevice(id, announce = true) {
  if (!runtime.has(id) || id === activeId) return;
  activeId = id;
  resetSpectrumHistory();
  hoveredFreq = null;
  syncControls();
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
  if (announce) {
    pushEvent(`${device().name} selected · ${formatFreq(settings().freq)} MHz`);
  }
}

function setFrequency(freq, announce = false) {
  const state = settings();
  state.freq = clamp(Math.round(freq * 1000) / 1000, 0.1, 6000);
  state.note = "Manual tune · simulated carrier";
  resetSpectrumHistory();
  syncControls();
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
  if (announce) {
    pushEvent(`Tuned ${device().name} to ${formatFreq(state.freq)} MHz`);
  }
}

function applyPreset(name) {
  const preset = PRESETS.find((item) => item.name === name);
  if (!preset) return;
  activeId = preset.device;
  const state = settings();
  state.freq = preset.freq;
  state.mode = preset.mode;
  state.bw = preset.bw;
  state.span = preset.span;
  state.note = preset.note;
  resetSpectrumHistory();
  hoveredFreq = null;
  syncControls();
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
  pushEvent(`${preset.name} preset · ${formatFreq(preset.freq)} MHz`);
}

function toggleReceiver() {
  const state = settings();
  state.running = !state.running;
  syncControls();
  pushEvent(`${device().name} ${state.running ? "started" : "stopped"}`);
  if (state.running) {
    lastWaterfallDraw = 0;
  }
}

function toggleRecording() {
  const state = settings();
  state.recording = !state.recording;
  syncControls();
  pushEvent(`${state.recording ? "Recording started" : "Recording stopped"} · ${device().name}`);
}

/* Canvases */

const spectrumCanvas = el.spectrum;
const spectrumCtx = spectrumCanvas.getContext("2d");
const waterfallCanvas = el.waterfall;
const waterfallCtx = waterfallCanvas.getContext("2d");

let spectrumSize = { w: 0, h: 0, dpr: 1 };
let waterfallSize = { w: 0, h: 0, dpr: 1 };
let lastSpectrum = new Float32Array(BINS).fill(-104);
const peakHoldValues = new Float32Array(BINS).fill(-130);
let peakHoldEnabled = true;
let waterfallRows = [];
let lastWaterfallDraw = 0;

function resizeSpectrum() {
  const rect = el.scopeCard.getBoundingClientRect();
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  spectrumSize = { w: Math.max(1, rect.width), h: Math.max(1, rect.height), dpr };
  spectrumCanvas.width = Math.floor(spectrumSize.w * dpr);
  spectrumCanvas.height = Math.floor(spectrumSize.h * dpr);
  spectrumCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
}

function resizeWaterfall() {
  const rect = el.waterfallWrap.getBoundingClientRect();
  const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
  waterfallSize = {
    w: Math.max(1, Math.floor(rect.width * dpr)),
    h: Math.max(1, Math.floor(rect.height * dpr)),
    dpr,
  };
  waterfallCanvas.width = waterfallSize.w;
  waterfallCanvas.height = waterfallSize.h;
  waterfallRows.length = 0;
}

function freqToX(freq, width) {
  const state = settings();
  return ((freq - (state.freq - state.span / 2)) / state.span) * width;
}

function dbToY(value, height) {
  const top = 8;
  const bottom = height - 8;
  return bottom - ((value + 120) / 100) * (bottom - top);
}

function makeSpectrum(now) {
  const item = device();
  const state = settings();
  const row = new Float32Array(BINS);
  const t = now * 0.001;
  for (let index = 0; index < BINS; index += 1) {
    const freq = state.freq - state.span / 2 + (index / (BINS - 1)) * state.span;
    let value =
      item.noise +
      Math.sin(index * 0.41 + t * 1.7) * 1.4 +
      (Math.random() - 0.5) * 2.4;
    for (const signal of item.signals) {
      const distance = Math.abs(freq - signal.freq);
      const halfWidth = signal.width / 2;
      if (distance < halfWidth * 4) {
        const drop = (distance / halfWidth) ** 2;
        const shimmer = Math.sin(t * 2.4 + index * 0.27) * 0.7;
        value = Math.max(value, signal.power - drop * 18 + shimmer);
      }
    }
    row[index] = clamp(value, -125, -20);
  }
  return row;
}

function drawSpectrum(row, now) {
  if (!spectrumSize.w || !row) return;
  const { w, h } = spectrumSize;
  const state = settings();
  const item = device();
  const ctx = spectrumCtx;

  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = rgba("scope", 1);
  ctx.fillRect(0, 0, w, h);

  ctx.strokeStyle = rgba("line", 0.52);
  ctx.lineWidth = 1;
  for (let index = 0; index <= 10; index += 1) {
    const x = Math.round((index / 10) * w) + 0.5;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
  }
  for (let index = 0; index <= 4; index += 1) {
    const y = Math.round(8 + (index / 4) * (h - 16)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }

  const passband = state.bw / 1000;
  const passLeft = freqToX(state.freq - passband / 2, w);
  const passRight = freqToX(state.freq + passband / 2, w);
  ctx.fillStyle = rgba("accent", 0.07);
  ctx.fillRect(passLeft, 0, Math.max(1, passRight - passLeft), h);
  ctx.strokeStyle = rgba("accent", 0.46);
  ctx.setLineDash([3, 4]);
  for (const x of [passLeft, passRight]) {
    ctx.beginPath();
    ctx.moveTo(x + 0.5, 0);
    ctx.lineTo(x + 0.5, h);
    ctx.stroke();
  }
  ctx.setLineDash([]);

  if (peakHoldEnabled) {
    for (let index = 0; index < BINS; index += 1) {
      peakHoldValues[index] = Math.max(row[index], peakHoldValues[index] - 0.55);
    }
    ctx.strokeStyle = rgba("faint", 0.55);
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let index = 0; index < BINS; index += 1) {
      const x = (index / (BINS - 1)) * w;
      const y = dbToY(peakHoldValues[index], h);
      if (index === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
  }

  const gradient = ctx.createLinearGradient(0, 0, 0, h);
  gradient.addColorStop(0, rgba("accent2", 0.28));
  gradient.addColorStop(0.55, rgba("accent2", 0.10));
  gradient.addColorStop(1, rgba("accent2", 0));
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let index = 0; index < BINS; index += 1) {
    const x = (index / (BINS - 1)) * w;
    const y = dbToY(row[index], h);
    ctx.lineTo(x, y);
  }
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fillStyle = gradient;
  ctx.fill();

  ctx.beginPath();
  for (let index = 0; index < BINS; index += 1) {
    const x = (index / (BINS - 1)) * w;
    const y = dbToY(row[index], h);
    if (index === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = rgba("accent2", 0.95);
  ctx.lineWidth = 1.25;
  ctx.stroke();

  const centerX = freqToX(state.freq, w);
  ctx.strokeStyle = rgba("accent", 0.9);
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(centerX + 0.5, 0);
  ctx.lineTo(centerX + 0.5, h);
  ctx.stroke();

  if (hoveredFreq !== null && !dragging) {
    const hoverX = freqToX(hoveredFreq, w);
    ctx.strokeStyle = rgba("ink", 0.45);
    ctx.setLineDash([2, 5]);
    ctx.beginPath();
    ctx.moveTo(hoverX + 0.5, 0);
    ctx.lineTo(hoverX + 0.5, h);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = rgba("ink", 0.9);
    ctx.font = `8px ${MONO_FONT}`;
    ctx.textAlign = hoverX > w - 55 ? "right" : "left";
    ctx.fillText(`${formatFreq(hoveredFreq)}`, hoverX + (hoverX > w - 55 ? -5 : 5), 17);
  }

  ctx.font = `8px ${MONO_FONT}`;
  ctx.textAlign = "left";
  for (const signal of item.signals) {
    if (!signal.label) continue;
    const x = freqToX(signal.freq, w);
    if (x < 10 || x > w - 10) continue;
    const y = dbToY(signal.power, h) - 7;
    ctx.fillStyle = rgba("ink", 0.7);
    ctx.fillText(signal.label, x + 5, Math.max(13, y));
  }

  const peak = Math.max(...row);
  ctx.fillStyle = rgba("faint", 0.85);
  ctx.textAlign = "right";
  ctx.fillText(`${peak.toFixed(0)} dBFS`, w - 9, h - 8);
}

function waterColor(value) {
  const normalized = clamp((value + 118) / 78, 0, 1) ** 0.68;
  const position = normalized * (WATER.length - 1);
  const low = Math.floor(position);
  const high = Math.min(WATER.length - 1, low + 1);
  const mix = position - low;
  const a = WATER[low];
  const b = WATER[high];
  return [
    Math.round(a[0] + (b[0] - a[0]) * mix),
    Math.round(a[1] + (b[1] - a[1]) * mix),
    Math.round(a[2] + (b[2] - a[2]) * mix),
  ];
}

function drawWaterfall(newRow = null) {
  if (!waterfallSize.w) return;
  const { w, h } = waterfallSize;
  const rowHeight = Math.max(2, Math.round(2 * waterfallSize.dpr));
  const maxRows = Math.ceil(h / rowHeight);

  if (newRow && settings().running) {
    waterfallRows.unshift(newRow);
    if (waterfallRows.length > maxRows) waterfallRows.length = maxRows;
  }

  if (!waterfallRows.length) {
    const seedAt = performance.now();
    for (let index = 0; index < maxRows; index += 1) {
      waterfallRows.push(makeSpectrum(seedAt - index * 65));
    }
  }

  const image = waterfallCtx.createImageData(w, h);
  const data = image.data;
  const empty = WATER[0];
  for (let y = 0; y < h; y += 1) {
    const row = waterfallRows[Math.floor(y / rowHeight)];
    for (let x = 0; x < w; x += 1) {
      const offset = (y * w + x) * 4;
      if (!row) {
        data[offset] = empty[0];
        data[offset + 1] = empty[1];
        data[offset + 2] = empty[2];
        data[offset + 3] = 255;
        continue;
      }
      const bin = Math.min(BINS - 1, Math.floor((x / (w - 1)) * (BINS - 1)));
      const color = waterColor(row[bin]);
      data[offset] = color[0];
      data[offset + 1] = color[1];
      data[offset + 2] = color[2];
      data[offset + 3] = 255;
    }
  }
  waterfallCtx.putImageData(image, 0, 0);

  const state = settings();
  const centerX = freqToX(state.freq, w);
  const passband = state.bw / 1000;
  const left = freqToX(state.freq - passband / 2, w);
  const right = freqToX(state.freq + passband / 2, w);

  waterfallCtx.fillStyle = rgba("accent", 0.09);
  waterfallCtx.fillRect(left, 0, Math.max(1, right - left), h);
  waterfallCtx.strokeStyle = rgba("accent", 0.76);
  waterfallCtx.lineWidth = 1;
  waterfallCtx.beginPath();
  waterfallCtx.moveTo(centerX + 0.5, 0);
  waterfallCtx.lineTo(centerX + 0.5, h);
  waterfallCtx.stroke();

  for (const x of [left, right]) {
    waterfallCtx.strokeStyle = rgba("accent", 0.32);
    waterfallCtx.setLineDash([2, 5]);
    waterfallCtx.beginPath();
    waterfallCtx.moveTo(x + 0.5, 0);
    waterfallCtx.lineTo(x + 0.5, h);
    waterfallCtx.stroke();
    waterfallCtx.setLineDash([]);
  }
}

function updateMeter(row) {
  const peak = Math.max(...row);
  const percent = clamp(((peak + 120) / 80) * 100, 3, 100);
  el.meterFill.style.width = `${percent.toFixed(0)}%`;
  el.signalValue.textContent = `${peak.toFixed(0)} dBFS`;
}

function frame(now) {
  if (now - lastFrameAt > 45) {
    lastSpectrum = makeSpectrum(now);
    if (settings().running) {
      drawSpectrum(lastSpectrum, now);
      updateMeter(lastSpectrum);
    }
    if (now - lastWaterfallDraw > 65) {
      drawWaterfall(settings().running ? lastSpectrum : null);
      lastWaterfallDraw = now;
    }
    lastFrameAt = now;
  }
  lastSpectrum = makeSpectrum(performance.now());
drawSpectrum(lastSpectrum, performance.now());
drawWaterfall(lastSpectrum);
lastWaterfallDraw = performance.now();
requestAnimationFrame(frame);
}

let lastFrameAt = 0;

/* Events */

function pointerFrequency(event) {
  const rect = spectrumCanvas.getBoundingClientRect();
  const x = clamp(event.clientX - rect.left, 0, rect.width);
  const state = settings();
  return state.freq - state.span / 2 + (x / rect.width) * state.span;
}

function tuneFromPointer(event, announce) {
  const frequency = pointerFrequency(event);
  const step = settings().span > 2 ? 0.025 : settings().span > 0.7 ? 0.005 : 0.001;
  setFrequency(Math.round(frequency / step) * step, announce);
}

el.deviceList.addEventListener("click", (event) => {
  const button = event.target.closest("[data-device]");
  if (button) setActiveDevice(button.dataset.device);
});

el.presetList.addEventListener("click", (event) => {
  const button = event.target.closest("[data-preset]");
  if (button) applyPreset(button.dataset.preset);
});

el.modeGrid.addEventListener("click", (event) => {
  const button = event.target.closest("[data-mode]");
  if (!button) return;
  const state = settings();
  state.mode = button.dataset.mode;
  state.note = `${state.mode} demodulation · simulated`;
  syncControls();
  pushEvent(`Mode ${state.mode} · ${device().name}`);
});

el.span.addEventListener("change", () => {
  const state = settings();
  state.span = Number(el.span.value);
  resetSpectrumHistory();
  syncControls();
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
});

el.bandwidth.addEventListener("input", () => {
  const state = settings();
  state.bw = Number(el.bandwidth.value);
  el.bwValue.textContent = formatBandwidth(state.bw);
  setSlider(el.bandwidth, state.bw);
  state.note = `Bandwidth ${formatBandwidth(state.bw)} · simulated`;
  el.tuneNote.textContent = state.note;
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
});

el.gain.addEventListener("input", () => {
  const state = settings();
  state.gain = Number(el.gain.value);
  el.gainValue.textContent = `${state.gain.toFixed(state.gain % 1 ? 1 : 0)} dB`;
  setSlider(el.gain, state.gain);
});

el.squelch.addEventListener("input", () => {
  const state = settings();
  state.squelch = Number(el.squelch.value);
  el.squelchValue.textContent = `${state.squelch} dBFS`;
  setSlider(el.squelch, state.squelch);
});

el.peakHold.addEventListener("click", () => {
  peakHoldEnabled = !peakHoldEnabled;
  el.peakHold.setAttribute("aria-pressed", String(peakHoldEnabled));
  resetSpectrumHistory();
  drawSpectrum(lastSpectrum, performance.now());
});

el.rxToggle.addEventListener("click", toggleReceiver);
el.recordToggle.addEventListener("click", toggleRecording);

for (const button of document.querySelectorAll("[data-step]")) {
  button.addEventListener("click", () => {
    setFrequency(settings().freq + Number(button.dataset.step), true);
  });
}

spectrumCanvas.addEventListener("pointerdown", (event) => {
  dragging = true;
  spectrumCanvas.setPointerCapture(event.pointerId);
  tuneFromPointer(event, false);
});

spectrumCanvas.addEventListener("pointermove", (event) => {
  hoveredFreq = pointerFrequency(event);
  if (dragging) tuneFromPointer(event, false);
  else drawSpectrum(lastSpectrum, performance.now());
});

spectrumCanvas.addEventListener("pointerup", (event) => {
  if (!dragging) return;
  dragging = false;
  tuneFromPointer(event, true);
});

spectrumCanvas.addEventListener("pointerleave", () => {
  hoveredFreq = null;
  drawSpectrum(lastSpectrum, performance.now());
});

spectrumCanvas.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    const direction = Math.sign(event.deltaY);
    const step = settings().span / 200;
    setFrequency(settings().freq - direction * step, false);
  },
  { passive: false },
);

window.addEventListener("keydown", (event) => {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
  if (event.code === "Space") {
    event.preventDefault();
    toggleReceiver();
  }
  if (event.code === "ArrowLeft" || event.code === "ArrowRight") {
    event.preventDefault();
    const step = event.shiftKey ? 0.025 : 0.001;
    setFrequency(settings().freq + (event.code === "ArrowRight" ? step : -step), false);
  }
});

const observer = new ResizeObserver(() => {
  resizeSpectrum();
  resizeWaterfall();
  drawSpectrum(lastSpectrum, performance.now());
  drawWaterfall();
});
observer.observe(el.scopeCard);
observer.observe(el.waterfallWrap);

setInterval(() => {
  el.clock.textContent = `${nowUtc()} UTC`;
  if (settings().running) sessionSeconds += 1;
  const hours = Math.floor(sessionSeconds / 3600);
  const minutes = Math.floor((sessionSeconds % 3600) / 60);
  const seconds = sessionSeconds % 60;
  el.statusSession.textContent = [hours, minutes, seconds]
    .map((value) => String(value).padStart(2, "0"))
    .join(":");
}, 1000);

renderDevices();
renderEvents();
renderPresets();
resizeSpectrum();
resizeWaterfall();
syncControls();
el.clock.textContent = `${nowUtc()} UTC`;
lastSpectrum = makeSpectrum(performance.now());
drawSpectrum(lastSpectrum, performance.now());
drawWaterfall(lastSpectrum);
lastWaterfallDraw = performance.now();
requestAnimationFrame(frame);
