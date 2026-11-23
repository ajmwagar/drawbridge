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

const DATA = JSON.parse(document.getElementById("rf-state").textContent);
const DEVICES = DATA.devices;
const PRESETS = DATA.presets;
const CATEGORIES = DATA.categories || [];
const GPS = DATA.gps || { fix: false, region: "us", label: "Unknown region" };

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
let bandmapCategory = "folders";
let bandmapRegion = "auto";
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
  bandPicker: $("#band-picker"),
  bandmap: $("#bandmap"),
  bandmapClose: $("#bandmap-close"),
  bandmapBreadcrumb: $("#bandmap-breadcrumb"),
  bandmapCrumb: $("#bandmap-crumb"),
  bandmapFolders: $("#bandmap-folders"),
  bandmapGroups: $("#bandmap-groups"),
  bandmapContent: $("#bandmap-content"),
  bandmapRegion: $("#bandmap-region"),
  bandmapGps: $("#bandmap-gps"),
  bandmapCount: $("#bandmap-count"),
  bandmapPrev: $("#bandmap-prev"),
  bandmapNext: $("#bandmap-next"),
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

function renderDevices() {
  el.receiverCount.textContent = String(DEVICES.length).padStart(2, "0");
  el.deviceCount.textContent = `${DEVICES.length} devices`;
  for (const item of DEVICES) {
    const button = el.deviceList.querySelector(`[data-device="${item.id}"]`);
    if (!button) continue;
    const state = runtime.get(item.id);
    const active = item.id === activeId;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-selected", String(active));
    const stateEl = button.querySelector(".device-state");
    if (stateEl) {
      stateEl.className = `device-state ${state.running ? "is-rx" : "is-idle"}`;
      stateEl.innerHTML = `<span class="status-dot" aria-hidden="true"></span>${state.running ? "RX" : "IDLE"}`;
    }
  }
}

function pushEvent(text) {
  const item = document.createElement("li");
  const time = document.createElement("time");
  time.textContent = nowUtc();
  const span = document.createElement("span");
  span.textContent = text;
  item.append(time, span);
  el.events.prepend(item);
  while (el.events.children.length > 5) el.events.lastElementChild.remove();
}

function selectedRegion() {
  if (bandmapRegion === "auto") return GPS.region || "us";
  if (bandmapRegion === "all") return null;
  return bandmapRegion;
}

function matchesRegion(preset, region) {
  if (!region) return true;
  const regions = preset.regions || [];
  return regions.includes("global") || regions.includes(region);
}

function bandmapPool() {
  const region = selectedRegion();
  return PRESETS.filter(
    (preset) =>
      (bandmapCategory === "folders" ||
        bandmapCategory === "all" ||
        preset.category === bandmapCategory) &&
      matchesRegion(preset, region),
  );
}

function visibleBandmapPresets() {
  return bandmapPool();
}

function categoryLabel(id) {
  if (id === "folders") return "Folders";
  return CATEGORIES.find((category) => category.id === id)?.label || "All bands";
}

function renderBandmap() {
  const state = settings();
  const folders = bandmapCategory === "folders";
  const region = selectedRegion();
  const visible = bandmapPool();
  const names = new Set(visible.map((preset) => preset.name));

  el.bandmapFolders.hidden = !folders;
  el.bandmapGroups.hidden = folders;
  el.bandmapPrev.hidden = folders;
  el.bandmapNext.hidden = folders;
  el.bandmapCrumb.textContent = categoryLabel(bandmapCategory);

  el.bandmapContent.querySelectorAll(".folder-card").forEach((card) => {
    const id = card.dataset.bandmapCategory;
    const count = PRESETS.filter(
      (preset) => preset.category === id && matchesRegion(preset, region),
    ).length;
    card.hidden = count === 0;
    const countEl = card.querySelector(".folder-count");
    if (countEl) countEl.textContent = `${count} place${count === 1 ? "" : "s"}`;
  });

  el.bandmapContent.querySelectorAll("[data-bandmap-group]").forEach((group) => {
    group.hidden =
      folders || (bandmapCategory !== "all" && group.dataset.bandmapGroup !== bandmapCategory);
  });

  el.bandmapContent.querySelectorAll(".band-card").forEach((card) => {
    const button = card.querySelector("[data-scenario]");
    const preset = PRESETS.find((item) => item.name === button?.dataset.scenario);
    card.hidden = folders || !preset || !names.has(preset.name);
    card.classList.toggle(
      "is-current",
      Boolean(preset && preset.device === activeId && preset.freq === state.freq),
    );
  });

  el.bandmapContent.querySelectorAll("[data-bandmap-group]").forEach((group) => {
    if (!group.hidden && !group.querySelector(".band-card:not([hidden])")) {
      group.hidden = true;
    }
  });

  if (folders) {
    const visibleFolders = [...el.bandmapContent.querySelectorAll(".folder-card")].filter(
      (card) => !card.hidden,
    ).length;
    el.bandmapCount.textContent = `${visibleFolders} folder${visibleFolders === 1 ? "" : "s"}`;
  } else {
    el.bandmapCount.textContent = `${visible.length} place${visible.length === 1 ? "" : "s"}`;
  }

  el.bandmapGps.textContent = GPS.fix
    ? `GPS fix · ${GPS.label}`
    : `No GPS fix · ${GPS.label}`;
}

function stepBandmap(direction) {
  const list = visibleBandmapPresets();
  if (!list.length) return;
  const index = list.findIndex(
    (preset) => preset.device === activeId && preset.freq === settings().freq,
  );
  const next = index === -1
    ? (direction > 0 ? 0 : list.length - 1)
    : (index + direction + list.length) % list.length;
  applyPreset(list[next].name);
  renderBandmap();
}

function openBandmap() {
  el.bandmap.hidden = false;
  document.body.classList.add("bandmap-open");
  renderBandmap();
  el.bandmapClose.focus();
}

function closeBandmap() {
  if (el.bandmap.hidden) return;
  el.bandmap.hidden = true;
  document.body.classList.remove("bandmap-open");
  el.bandPicker.focus();
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
  renderBandmap();
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
const bandHash = location.hash.match(/^#bands(?:\/([a-z-]+))?$/);
if (bandHash) {
  if (bandHash[1] && CATEGORIES.some((category) => category.id === bandHash[1])) {
    bandmapCategory = bandHash[1];
  }
  openBandmap();
}
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

el.bandPicker.addEventListener("click", openBandmap);
el.bandmapClose.addEventListener("click", closeBandmap);
el.bandmap.addEventListener("click", (event) => {
  if (event.target.closest("[data-bandmap-close]")) closeBandmap();
});
el.bandmapBreadcrumb.addEventListener("click", (event) => {
  const button = event.target.closest("[data-bandmap-category]");
  if (!button) return;
  bandmapCategory = button.dataset.bandmapCategory;
  renderBandmap();
});
el.bandmapContent.addEventListener("click", (event) => {
  const folder = event.target.closest(".folder-card[data-bandmap-category]");
  if (folder) {
    bandmapCategory = folder.dataset.bandmapCategory;
    renderBandmap();
    return;
  }
  const button = event.target.closest("[data-scenario]");
  if (!button) return;
  applyPreset(button.dataset.scenario);
  closeBandmap();
});
el.bandmapRegion.addEventListener("change", () => {
  bandmapRegion = el.bandmapRegion.value;
  renderBandmap();
});
el.bandmapPrev.addEventListener("click", () => stepBandmap(-1));
el.bandmapNext.addEventListener("click", () => stepBandmap(1));

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
  if (!el.bandmap.hidden) {
    if (event.code === "Escape") {
      event.preventDefault();
      closeBandmap();
    }
    return;
  }
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
renderBandmap();
resizeSpectrum();
resizeWaterfall();
syncControls();
el.clock.textContent = `${nowUtc()} UTC`;
lastSpectrum = makeSpectrum(performance.now());
drawSpectrum(lastSpectrum, performance.now());
drawWaterfall(lastSpectrum);
lastWaterfallDraw = performance.now();
const bandHash = location.hash.match(/^#bands(?:\/([a-z-]+))?$/);
if (bandHash) {
  if (bandHash[1] && CATEGORIES.some((category) => category.id === bandHash[1])) {
    bandmapCategory = bandHash[1];
  }
  openBandmap();
}
requestAnimationFrame(frame);
