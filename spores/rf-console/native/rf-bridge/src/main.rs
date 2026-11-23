//! Native RF bridge for the RF Console.
//!
//! This is the drop-in replacement for the Python scoutd prototype:
//! TinySA serial scan -> /scout.json, SoapySDR inventory -> /sdr/devices,
//! process-backed IQ -> rustfft rows -> /sdr/frame. The web surface keeps the
//! same JSON transport while the native owner owns devices and DSP.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use num_complex::Complex32;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serial2::SerialPort;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const IQ_SAMPLES: usize = 16_384;
const DEFAULT_BINS: usize = 512;

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    if values.is_empty() {
        0.0
    } else if values.len() % 2 == 0 {
        (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.0
    } else {
        values[values.len() / 2]
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PagePreset {
    name: String,
    freq: f64,
    span: f64,
}

fn load_presets(path: &Path) -> Result<Vec<PagePreset>, String> {
    let page =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let marker = "<script id=\"rf-state\" type=\"application/json\">";
    let start = page.find(marker).ok_or("no #rf-state in page")? + marker.len();
    let end = page[start..]
        .find("</script>")
        .ok_or("unterminated #rf-state")?
        + start;
    let state: Value = serde_json::from_str(&page[start..end])
        .map_err(|error| format!("parse #rf-state: {error}"))?;
    serde_json::from_value(
        state
            .get("presets")
            .cloned()
            .unwrap_or(Value::Array(vec![])),
    )
    .map_err(|error| format!("parse presets: {error}"))
}

#[derive(Debug, Clone, Serialize, Default)]
struct ScoutReading {
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_dbm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_hz: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    noise_dbm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snr_db: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    center_dbm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ScoutState {
    source: String,
    status: String,
    antenna: String,
    serial: Option<String>,
    scanned_at: Option<String>,
    scan_started_at: Option<String>,
    scan_count: u64,
    error: Option<String>,
    presets: BTreeMap<String, ScoutReading>,
}

impl ScoutState {
    fn new(antenna: &str) -> Self {
        Self {
            source: "tinySA4".into(),
            status: "starting".into(),
            antenna: antenna.into(),
            serial: None,
            scanned_at: None,
            scan_started_at: None,
            scan_count: 0,
            error: None,
            presets: BTreeMap::new(),
        }
    }
}

struct Scout {
    page: PathBuf,
    port: String,
    points: usize,
    retry: Duration,
    state: Arc<Mutex<ScoutState>>,
    stop: Arc<AtomicBool>,
}

impl Scout {
    fn snapshot(&self) -> ScoutState {
        self.state.lock().unwrap().clone()
    }

    fn update(&self, change: impl FnOnce(&mut ScoutState)) {
        change(&mut self.state.lock().unwrap());
    }

    fn run(self: Arc<Self>) {
        while !self.stop.load(Ordering::Relaxed) {
            let result = self.scan_once();
            if let Err(error) = result {
                let message = error.to_string();
                self.update(|state| {
                    state.status = "offline".into();
                    state.serial = Some(self.port.clone());
                    state.error = Some(message);
                });
                thread::sleep(self.retry);
            }
        }
    }

    fn scan_once(&self) -> Result<(), String> {
        let presets = load_presets(&self.page)?;
        let mut port = SerialPort::open(&self.port, 115_200)
            .map_err(|error| format!("open {}: {error}", self.port))?;
        port.set_read_timeout(Duration::from_millis(30))
            .map_err(|error| format!("set read timeout: {error}"))?;
        port.set_write_timeout(Duration::from_secs(3))
            .map_err(|error| format!("set write timeout: {error}"))?;
        port.set_dtr(false)
            .map_err(|error| format!("clear DTR: {error}"))?;
        port.set_rts(false)
            .map_err(|error| format!("clear RTS: {error}"))?;
        self.update(|state| {
            state.status = if state.scanned_at.is_none() {
                "scanning"
            } else {
                "live"
            }
            .into();
            state.serial = Some(self.port.clone());
            state.error = None;
            state.scan_started_at = Some(now_iso());
            state.scan_count = state.scan_count.saturating_add(1);
        });
        for preset in presets {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let reading = self.scan_preset(&port, &preset);
            self.update(|state| {
                state.presets.insert(preset.name.clone(), reading.clone());
                state.error = reading.error.clone();
            });
            println!(
                "{}: {:?} dBm, {:?} dB SNR",
                preset.name, reading.peak_dbm, reading.snr_db
            );
        }
        self.update(|state| {
            state.status = "live".into();
            state.scanned_at = Some(now_iso());
            state.scan_started_at = Some(now_iso());
        });
        Ok(())
    }

    fn scan_preset(&self, port: &SerialPort, preset: &PagePreset) -> ScoutReading {
        let center = preset.freq * 1e6;
        let span = preset.span * 1e6;
        let start = (center - span / 2.0).max(0.0);
        let stop = (center + span / 2.0).max(start + 1.0);
        let values = match read_amplitudes(port, start, stop, self.points) {
            Ok(values) => values,
            Err(error) => {
                return ScoutReading {
                    error: Some(error),
                    ..Default::default()
                }
            }
        };
        if values.len() < 20 {
            return ScoutReading {
                error: Some("no data".into()),
                ..Default::default()
            };
        }
        let mut sorted = values.clone();
        let noise = median(&mut sorted);
        let (peak_index, peak) = values
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, noise));
        let peak_hz = start + (peak_index as f64 / (values.len() - 1) as f64) * span;
        let center_dbm = values[values.len() / 2];
        ScoutReading {
            peak_dbm: Some(round1(peak)),
            peak_hz: Some(peak_hz.round() as u64),
            noise_dbm: Some(round1(noise)),
            snr_db: Some(round1(peak - noise)),
            center_dbm: Some(round1(center_dbm)),
            error: None,
        }
    }
}

fn drain_serial(port: &SerialPort, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    let mut last_data = Instant::now();
    let mut got_data = false;
    let mut buffer = [0_u8; 4096];
    while started.elapsed() < timeout {
        match port.read(&mut buffer) {
            Ok(0) => {
                if got_data && last_data.elapsed() > Duration::from_millis(50) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Ok(_) => {
                got_data = true;
                last_data = Instant::now();
            }
            Err(error)
                if error.kind() == ErrorKind::TimedOut || error.kind() == ErrorKind::WouldBlock =>
            {
                if got_data && last_data.elapsed() > Duration::from_millis(50) {
                    break;
                }
                if !got_data && started.elapsed() > timeout / 2 {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("drain serial: {error}")),
        }
    }
    Ok(())
}

fn send_command(port: &SerialPort, command: &str, wait: Duration) -> Result<(), String> {
    port.discard_input_buffer()
        .map_err(|error| format!("drain serial: {error}"))?;
    let mut line = command.as_bytes().to_vec();
    line.push(b'\r');
    port.write_all(&line)
        .map_err(|error| format!("write serial: {error}"))?;
    thread::sleep(wait);
    Ok(())
}

fn collect_command(
    port: &SerialPort,
    command: &str,
    wait: Duration,
    limit: usize,
) -> Result<String, String> {
    port.discard_input_buffer()
        .map_err(|error| format!("drain serial: {error}"))?;
    let mut line = command.as_bytes().to_vec();
    line.push(b'\r');
    port.write_all(&line)
        .map_err(|error| format!("write serial: {error}"))?;
    if !wait.is_zero() {
        thread::sleep(wait);
    }
    let started = Instant::now();
    let mut last_data = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    while started.elapsed() < Duration::from_secs(4) && bytes.len() < limit {
        match port.read(&mut buffer) {
            Ok(0) => {
                if !bytes.is_empty() && last_data.elapsed() > Duration::from_millis(100) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                last_data = Instant::now();
                if bytes.len() >= limit {
                    break;
                }
            }
            Err(error)
                if error.kind() == ErrorKind::TimedOut || error.kind() == ErrorKind::WouldBlock =>
            {
                if !bytes.is_empty() && last_data.elapsed() > Duration::from_millis(100) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("read serial: {error}")),
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_amplitudes(
    port: &SerialPort,
    start_hz: f64,
    stop_hz: f64,
    points: usize,
) -> Result<Vec<f64>, String> {
    send_command(
        port,
        &format!("sweep {:.0} {:.0} {points}", start_hz, stop_hz),
        Duration::from_millis(200),
    )?;
    drain_serial(port, Duration::from_millis(400))?;
    send_command(port, "scan", Duration::from_millis(500))?;
    drain_serial(port, Duration::from_millis(1500))?;
    let data = collect_command(port, "data 0", Duration::from_millis(200), points * 32)?;
    let mut values = Vec::with_capacity(points);
    for line in data.lines() {
        if let Ok(value) = line.trim().parse::<f64>() {
            values.push(value);
        }
    }
    Ok(values.into_iter().rev().take(points).rev().collect())
}

#[derive(Debug, Clone, Serialize)]
struct DeviceSpec {
    id: String,
    name: String,
    driver: String,
    serial: String,
    label: String,
    kind: String,
}

#[derive(Debug, Clone)]
struct SdrConfig {
    freq: u64,
    span: u64,
    bins: usize,
    bw: u64,
    gain: f32,
}

impl SdrConfig {
    fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.freq, self.span, self.bins, self.bw, self.gain
        )
    }
}

#[derive(Default)]
struct FrameState {
    frame: Option<Value>,
    version: u64,
    error: Option<String>,
}

struct Worker {
    spec: DeviceSpec,
    config: SdrConfig,
    key: String,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    frame: Arc<(Mutex<FrameState>, Condvar)>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Worker {
    fn new(spec: DeviceSpec, config: SdrConfig) -> Self {
        let key = format!("{}:{}", spec.id, config.key());
        Self {
            spec,
            config,
            key,
            stop: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            frame: Arc::new((Mutex::new(FrameState::default()), Condvar::new())),
            thread: Mutex::new(None),
        }
    }

    fn start(self: &Arc<Self>) {
        let worker = Arc::clone(self);
        let thread = thread::Builder::new()
            .name(format!("sdr-{}", self.spec.id))
            .spawn(move || worker.run())
            .expect("spawn SDR worker");
        *self.thread.lock().unwrap() = Some(thread);
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    fn latest(&self) -> Option<Value> {
        self.frame.0.lock().unwrap().frame.clone()
    }

    fn version(&self) -> u64 {
        self.frame.0.lock().unwrap().version
    }

    fn wait(&self, version: u64, timeout: Duration) -> (Option<Value>, u64) {
        let (lock, cvar) = &*self.frame;
        let mut state = lock.lock().unwrap();
        let deadline = Instant::now() + timeout;
        while state.version == version {
            let now = Instant::now();
            if now >= deadline || self.stop.load(Ordering::Relaxed) {
                break;
            }
            let (next, _) = cvar.wait_timeout(state, deadline - now).unwrap();
            state = next;
        }
        (state.frame.clone(), state.version)
    }

    fn publish(&self, frame: Value) {
        let (lock, cvar) = &*self.frame;
        let mut state = lock.lock().unwrap();
        state.frame = Some(frame);
        state.error = None;
        state.version = state.version.saturating_add(1);
        cvar.notify_all();
    }

    fn fail(&self, message: String) {
        let frame = json!({
            "device": self.spec.id,
            "error": message,
            "at": now_iso(),
        });
        let (lock, cvar) = &*self.frame;
        let mut state = lock.lock().unwrap();
        state.frame = Some(frame);
        state.error = Some(message);
        state.version = state.version.saturating_add(1);
        cvar.notify_all();
    }

    fn run(self: Arc<Self>) {
        while !self.stop.load(Ordering::Relaxed) {
            let child = match self.spawn() {
                Ok(child) => child,
                Err(error) => {
                    self.fail(error);
                    thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            let mut stdout = match child.stdout.as_ref() {
                Some(_) => {
                    let mut child = child;
                    let stdout = child.stdout.take().unwrap();
                    *self.child.lock().unwrap() = Some(child);
                    stdout
                }
                None => {
                    self.fail("SDR process has no stdout".into());
                    thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            let mut buffer = vec![0_u8; IQ_SAMPLES * 2];
            let mut ema: Option<Vec<f32>> = None;
            while !self.stop.load(Ordering::Relaxed) {
                if let Err(error) = stdout.read_exact(&mut buffer) {
                    self.fail(format!("read IQ: {error}"));
                    break;
                }
                let row = iq_to_dbm(&buffer, &self.spec.driver, self.config.bins);
                let row = match ema.take() {
                    Some(previous) => previous
                        .iter()
                        .zip(row.iter())
                        .map(|(old, new)| old * 0.72 + new * 0.28)
                        .collect(),
                    None => row,
                };
                let (peak_index, peak_dbm) = row
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap_or((0, -130.0));
                let span = actual_sample_rate(&self.spec.driver, self.config.span);
                let center = self.config.freq as f64;
                let peak_hz = center - span / 2.0
                    + (peak_index as f64 + 0.5) * span / self.config.bins as f64;
                self.publish(json!({
                    "device": self.spec.id,
                    "driver": self.spec.driver,
                    "serial": self.spec.serial,
                    "center_hz": center,
                    "span_hz": span,
                    "bin_hz": span / self.config.bins as f64,
                    "bins": self.config.bins,
                    "dbm": row,
                    "peak_hz": peak_hz,
                    "peak_dbm": round1(peak_dbm as f64),
                    "gain_db": self.config.gain,
                    "bandwidth_hz": self.config.bw,
                    "calibrated": false,
                    "at": now_iso(),
                }));
                ema = Some(row);
            }
            if let Some(mut child) = self.child.lock().unwrap().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if !self.stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(2));
            }
        }
    }

    fn spawn(&self) -> Result<Child, String> {
        let rate = actual_sample_rate(&self.spec.driver, self.config.span);
        let mut command = match self.spec.driver.as_str() {
            "hackrf" => {
                let mut command = Command::new("hackrf_transfer");
                let lna = ((self.config.gain * 0.35).round() as u32).clamp(0, 40);
                let vga = ((self.config.gain * 0.50).round() as u32).clamp(0, 62);
                command.args([
                    "-r",
                    "-",
                    "-f",
                    &self.config.freq.to_string(),
                    "-s",
                    &(rate as u64).to_string(),
                    "-l",
                    &lna.to_string(),
                    "-g",
                    &vga.to_string(),
                    "-b",
                    &(self.config.bw.max(1_750_000) as u64).to_string(),
                ]);
                command
            }
            "rtlsdr" => {
                let mut command = Command::new("rtl_sdr");
                command.args([
                    "-d",
                    &self.spec.serial,
                    "-f",
                    &self.config.freq.to_string(),
                    "-s",
                    &(rate as u64).to_string(),
                    "-g",
                    &self.config.gain.to_string(),
                    "-",
                ]);
                command
            }
            other => return Err(format!("unsupported driver: {other}")),
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("spawn {}: {error}", self.spec.driver))
    }
}

fn actual_sample_rate(driver: &str, span: u64) -> f64 {
    if driver == "hackrf" {
        let mhz = ((span as f64) / 1_000_000.0).ceil().max(2.0).min(20.0);
        mhz * 1_000_000.0
    } else if driver == "rtlsdr" {
        if span <= 300_000 {
            300_000.0
        } else if span <= 1_000_000 {
            1_200_000.0
        } else if span <= 2_400_000 {
            2_400_000.0
        } else {
            3_200_000.0
        }
    } else {
        ((span as f64) / 1_000_000.0).ceil().max(1.0) * 1_000_000.0
    }
}

fn iq_to_dbm(bytes: &[u8], driver: &str, bins: usize) -> Vec<f32> {
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let (i, q) = if driver == "rtlsdr" {
            (
                (pair[0] as f32 - 127.5) / 128.0,
                (pair[1] as f32 - 127.5) / 128.0,
            )
        } else {
            (
                (pair[0] as i8) as f32 / 128.0,
                (pair[1] as i8) as f32 / 128.0,
            )
        };
        samples.push(Complex32::new(i, q));
    }
    let n = samples.len();
    if n == 0 {
        return vec![-130.0; bins];
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buffer: Vec<Complex32> = samples
        .into_iter()
        .enumerate()
        .map(|(index, sample)| {
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / (n - 1) as f32).cos();
            sample * window
        })
        .collect();
    fft.process(&mut buffer);
    let scale = 2.0 / n as f32;
    let mut power = Vec::with_capacity(n);
    for index in 0..n {
        let value = buffer[(index + n / 2) % n].norm() * scale;
        power.push((20.0 * value.max(1.0e-6).log10() - 30.0).clamp(-130.0, -20.0));
    }
    let mut row = vec![-130.0_f32; bins];
    for (index, slot) in row.iter_mut().enumerate() {
        let position = index as f32 * (n - 1) as f32 / (bins - 1) as f32;
        let left = position.floor() as usize;
        let right = (left + 1).min(n - 1);
        let mix = position - left as f32;
        *slot = power[left] + (power[right] - power[left]) * mix;
    }
    row
}

struct SdrManager {
    current: Mutex<Option<Arc<Worker>>>,
    devices: Mutex<(Instant, Vec<DeviceSpec>)>,
}

impl SdrManager {
    fn new() -> Self {
        Self {
            current: Mutex::new(None),
            devices: Mutex::new((Instant::now() - Duration::from_secs(10), Vec::new())),
        }
    }

    fn devices(&self) -> Vec<DeviceSpec> {
        {
            let cached = self.devices.lock().unwrap();
            if cached.0.elapsed() < Duration::from_secs(5) {
                return cached.1.clone();
            }
        }
        let found = enumerate_devices();
        *self.devices.lock().unwrap() = (Instant::now(), found.clone());
        found
    }

    fn frame(&self, device: &str, config: SdrConfig) -> Result<Value, String> {
        let spec = self
            .devices()
            .into_iter()
            .find(|item| item.id == device)
            .ok_or_else(|| format!("device not found: {device}"))?;
        let worker = {
            let mut current = self.current.lock().unwrap();
            let wanted_key = format!("{}:{}", spec.id, config.key());
            if let Some(existing) = current.as_ref() {
                if existing.key == wanted_key {
                    Arc::clone(existing)
                } else {
                    let old = current.take();
                    if let Some(old) = old {
                        old.stop();
                    }
                    let worker = Arc::new(Worker::new(spec, config));
                    worker.start();
                    *current = Some(Arc::clone(&worker));
                    worker
                }
            } else {
                let worker = Arc::new(Worker::new(spec, config));
                worker.start();
                *current = Some(Arc::clone(&worker));
                worker
            }
        };
        if let Some(frame) = worker.latest() {
            return Ok(frame);
        }
        let version = worker.version();
        let (frame, _) = worker.wait(version, Duration::from_secs(4));
        frame.ok_or_else(|| "no frame yet".into())
    }
}

fn enumerate_devices() -> Vec<DeviceSpec> {
    let output = Command::new("SoapySDRUtil").arg("--find").output().ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut found = Vec::new();
    let mut current: HashMap<String, String> = HashMap::new();
    let flush = |current: &mut HashMap<String, String>, found: &mut Vec<DeviceSpec>| {
        let driver = current.get("driver").cloned().unwrap_or_default();
        let id = match driver.as_str() {
            "hackrf" => "hackrf",
            "rtlsdr" => "rtl",
            _ => return,
        };
        let name = if id == "hackrf" {
            "HackRF One"
        } else {
            "RTL-SDR Blog V4"
        };
        found.push(DeviceSpec {
            id: id.into(),
            name: name.into(),
            driver: driver.clone(),
            serial: current.get("serial").cloned().unwrap_or_default(),
            label: current.get("label").cloned().unwrap_or_else(|| name.into()),
            kind: "sdr".into(),
        });
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Found device") {
            flush(&mut current, &mut found);
            current.clear();
        } else if let Some((key, value)) = line.split_once('=') {
            current.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    flush(&mut current, &mut found);
    found
}

struct App {
    scout: Arc<Scout>,
    sdrs: Arc<SdrManager>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut listen = "127.0.0.1:8798".to_string();
    let mut port = "/dev/ttyACM1".to_string();
    let mut page = PathBuf::from("/opt/rf-console/index.html");
    let mut antenna = "AR-771".to_string();
    let mut points = 200usize;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--listen" => {
                index += 1;
                listen = args.get(index).cloned().unwrap_or(listen);
            }
            "--port" => {
                index += 1;
                port = args.get(index).cloned().unwrap_or(port);
            }
            "--page" => {
                index += 1;
                page = PathBuf::from(args.get(index).cloned().unwrap_or_default());
            }
            "--antenna" => {
                index += 1;
                antenna = args.get(index).cloned().unwrap_or(antenna);
            }
            "--points" => {
                index += 1;
                points = args
                    .get(index)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(points);
            }
            other => eprintln!("unknown argument: {other}"),
        }
        index += 1;
    }

    let scout = Arc::new(Scout {
        page,
        port,
        points,
        retry: Duration::from_secs(6),
        state: Arc::new(Mutex::new(ScoutState::new(&antenna))),
        stop: Arc::new(AtomicBool::new(false)),
    });
    let scout_thread = Arc::clone(&scout);
    thread::Builder::new()
        .name("tinysa-scout".into())
        .spawn(move || scout_thread.run())?;
    let app = Arc::new(App {
        scout,
        sdrs: Arc::new(SdrManager::new()),
    });
    let server = Server::http(&listen).map_err(|error| format!("listen {listen}: {error}"))?;
    println!("rf-bridge on http://{listen}");
    for request in server.incoming_requests() {
        let app = Arc::clone(&app);
        thread::spawn(move || handle(request, &app));
    }
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn respond(request: Request, status: u16, body: String, content_type: &str) {
    let mut response = Response::from_string(body).with_status_code(StatusCode(status));
    response.add_header(header("Content-Type", content_type));
    response.add_header(header("Cache-Control", "no-store"));
    response.add_header(header("Access-Control-Allow-Origin", "*"));
    response.add_header(header("Access-Control-Allow-Methods", "GET, OPTIONS"));
    response.add_header(header("Access-Control-Allow-Headers", "*"));
    let _ = request.respond(response);
}

fn json_response(request: Request, value: Value) {
    respond(
        request,
        200,
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".into()),
        "application/json; charset=utf-8",
    );
}

fn query(url: &str) -> HashMap<String, String> {
    let Some((_, query)) = url.split_once('?') else {
        return HashMap::new();
    };
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn handle(request: Request, app: &App) {
    if request.method() == &Method::Options {
        respond(request, 204, String::new(), "text/plain");
        return;
    }
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url);
    match path {
        "/scout.json" => {
            json_response(request, serde_json::to_value(app.scout.snapshot()).unwrap())
        }
        "/healthz" => {
            let state = app.scout.snapshot();
            json_response(
                request,
                json!({"ok": state.status == "live", "status": state.status, "serial": state.serial}),
            );
        }
        "/sdr/devices" => {
            let devices = app.sdrs.devices();
            json_response(
                request,
                json!({"source": "soapy", "devices": devices, "error": if devices.is_empty() { Some("no SoapySDR devices found") } else { None }}),
            );
        }
        "/sdr/frame" => {
            let query = query(&url);
            let device = query.get("device").map(String::as_str).unwrap_or("");
            let device = if device == "adsb" || device == "ais" {
                "rtl"
            } else {
                device
            };
            let config = SdrConfig {
                freq: query
                    .get("freq")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(100_000_000),
                span: query
                    .get("span")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2_000_000)
                    .max(100_000),
                bins: query
                    .get("bins")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_BINS)
                    .clamp(128, 2048),
                bw: query.get("bw").and_then(|v| v.parse().ok()).unwrap_or(0),
                gain: query
                    .get("gain")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(20.0),
            };
            match app.sdrs.frame(device, config) {
                Ok(frame) => json_response(request, frame),
                Err(error) => json_response(request, json!({"error": error})),
            }
        }
        _ => respond(request, 404, "not found".into(), "text/plain"),
    }
}
