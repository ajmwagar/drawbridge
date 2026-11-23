//! Native audio for the RF bridge: demodulate IQ, pace 48k stereo packets,
//! send AES67-style RTP to Isochrone, and optionally write a WAV recording.

use std::collections::VecDeque;
use std::fs;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hound::{SampleFormat, WavSpec, WavWriter};
use num_complex::Complex32;

const AUDIO_RATE: u32 = 48_000;
const FRAMES_PER_PACKET: usize = 48;
const MAX_QUEUED_SAMPLES: usize = AUDIO_RATE as usize * 2;
const RTP_PAYLOAD_TYPE: u8 = 97;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    Am,
    Nfm,
    Wfm,
    Usb,
    Lsb,
}

impl DemodMode {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "am" => Self::Am,
            "wfm" => Self::Wfm,
            "usb" => Self::Usb,
            "lsb" => Self::Lsb,
            _ => Self::Nfm,
        }
    }
}

#[derive(Debug, Clone)]
struct DemodConfig {
    mode: DemodMode,
    input_rate_hz: f64,
    output_rate_hz: f64,
    bandwidth_hz: f64,
    deviation_hz: f64,
}

impl DemodConfig {
    fn new(mode: DemodMode, input_rate_hz: f64) -> Self {
        let (bandwidth_hz, deviation_hz) = match mode {
            DemodMode::Am => (6_000.0, 0.0),
            DemodMode::Nfm => (12_500.0, 2_500.0),
            DemodMode::Wfm => (200_000.0, 75_000.0),
            DemodMode::Usb | DemodMode::Lsb => (3_000.0, 0.0),
        };
        Self {
            mode,
            input_rate_hz,
            output_rate_hz: f64::from(AUDIO_RATE),
            bandwidth_hz,
            deviation_hz,
        }
    }
}

struct Demodulator {
    config: DemodConfig,
    last_iq: Option<Complex32>,
    deemphasis: f32,
    dc_block: f32,
    lowpass_1: f32,
    lowpass_2: f32,
    resample_phase: f64,
}

impl Demodulator {
    fn new(config: DemodConfig) -> Self {
        Self {
            config,
            last_iq: None,
            deemphasis: 0.0,
            dc_block: 0.0,
            lowpass_1: 0.0,
            lowpass_2: 0.0,
            resample_phase: 0.0,
        }
    }

    fn process(&mut self, iq: &[Complex32]) -> Vec<f32> {
        if iq.is_empty() {
            return Vec::new();
        }
        let mut detected = Vec::with_capacity(iq.len());
        match self.config.mode {
            DemodMode::Nfm | DemodMode::Wfm => {
                let mut previous = self.last_iq.unwrap_or(iq[0]);
                for sample in iq {
                    let product = *sample * previous.conj();
                    let phase = product.arg();
                    detected.push(
                        phase
                            * (self.config.input_rate_hz
                                / (std::f64::consts::TAU * self.config.deviation_hz))
                                as f32,
                    );
                    previous = *sample;
                }
                self.last_iq = Some(previous);
            }
            DemodMode::Am => {
                let rate = self.config.input_rate_hz as f32;
                for sample in iq {
                    let envelope = sample.norm();
                    self.dc_block += (envelope - self.dc_block) * (20.0 / rate).min(0.25);
                    detected.push(envelope - self.dc_block);
                }
            }
            DemodMode::Usb => detected.extend(iq.iter().map(|sample| sample.re)),
            DemodMode::Lsb => detected.extend(iq.iter().map(|sample| sample.im)),
        }

        if matches!(self.config.mode, DemodMode::Nfm | DemodMode::Wfm) {
            let dt = 1.0 / self.config.input_rate_hz;
            let tau = 75.0e-6;
            let alpha = (dt / (tau + dt)) as f32;
            for value in &mut detected {
                self.deemphasis += alpha * (*value - self.deemphasis);
                *value = self.deemphasis;
            }
        }

        let cutoff = (self.config.bandwidth_hz / 2.0)
            .min(self.config.output_rate_hz * 0.45)
            .max(1.0);
        let alpha =
            (1.0 - (-std::f64::consts::TAU * cutoff / self.config.input_rate_hz).exp()) as f32;
        for value in &mut detected {
            self.lowpass_1 += alpha * (*value - self.lowpass_1);
            self.lowpass_2 += alpha * (self.lowpass_1 - self.lowpass_2);
            *value = self.lowpass_2;
        }

        self.resample(&detected)
    }

    fn resample(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        let step = self.config.input_rate_hz / self.config.output_rate_hz;
        if step <= 1.0 {
            return input.to_vec();
        }
        let mut output = Vec::with_capacity(((input.len() as f64) / step).ceil() as usize + 2);
        let mut position = self.resample_phase;
        while position < (input.len() - 1) as f64 {
            let index = position.floor() as usize;
            let fraction = (position - index as f64) as f32;
            let first = input[index];
            let second = input[index + 1];
            output.push(first + (second - first) * fraction);
            position += step;
        }
        self.resample_phase = (position - input.len() as f64).max(0.0);
        output
    }
}

pub struct AudioPipeline {
    queue: Arc<(Mutex<VecDeque<f32>>, Condvar)>,
    demod: Mutex<Demodulator>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    recording: Arc<Mutex<Option<PathBuf>>>,
}

impl AudioPipeline {
    pub fn start(
        input_rate_hz: f64,
        mode: DemodMode,
        target: SocketAddr,
        record: bool,
        recordings_dir: &Path,
        device_id: &str,
    ) -> Result<Arc<Self>, String> {
        let queue = Arc::new((Mutex::new(VecDeque::<f32>::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recording = Arc::new(Mutex::new(None::<PathBuf>));
        let pipeline = Arc::new(Self {
            queue: Arc::clone(&queue),
            demod: Mutex::new(Demodulator::new(DemodConfig::new(mode, input_rate_hz))),
            stop: Arc::clone(&stop),
            thread: Mutex::new(None),
            recording: Arc::clone(&recording),
        });

        let record_path = if record {
            fs::create_dir_all(recordings_dir)
                .map_err(|error| format!("create recordings dir: {error}"))?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            let path = recordings_dir.join(format!("rf-{device_id}-{stamp}.wav"));
            *recording.lock().unwrap() = Some(path.clone());
            Some(path)
        } else {
            None
        };

        let thread = thread::Builder::new()
            .name(format!("audio-{device_id}"))
            .spawn(move || audio_loop(queue, stop, target, record_path))
            .map_err(|error| format!("spawn audio: {error}"))?;
        *pipeline.thread.lock().unwrap() = Some(thread);
        Ok(pipeline)
    }

    pub fn recording_path(&self) -> Option<PathBuf> {
        self.recording.lock().unwrap().clone()
    }

    pub fn push_iq(&self, iq: &[Complex32]) {
        let samples = self.demod.lock().unwrap().process(iq);
        if samples.is_empty() {
            return;
        }
        let (lock, cvar) = &*self.queue;
        let mut queue = lock.lock().unwrap();
        if queue.len() + samples.len() > MAX_QUEUED_SAMPLES {
            let excess = queue.len() + samples.len() - MAX_QUEUED_SAMPLES;
            let discard = excess.min(queue.len());
            queue.drain(..discard);
        }
        queue.extend(samples);
        cvar.notify_all();
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let (_lock, cvar) = &*self.queue;
        cvar.notify_all();
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

fn audio_loop(
    queue: Arc<(Mutex<VecDeque<f32>>, Condvar)>,
    stop: Arc<AtomicBool>,
    target: SocketAddr,
    record_path: Option<PathBuf>,
) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("audio: UDP bind failed: {error}");
            return;
        }
    };
    if let Err(error) = socket.connect(target) {
        eprintln!("audio: UDP connect {target}: {error}");
        return;
    }
    let mut writer = record_path.and_then(|path| {
        let spec = WavSpec {
            channels: 2,
            sample_rate: AUDIO_RATE,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        match WavWriter::create(&path, spec) {
            Ok(writer) => {
                eprintln!("audio: recording {}", path.display());
                Some((path, writer))
            }
            Err(error) => {
                eprintln!("audio: create {}: {error}", path.display());
                None
            }
        }
    });
    let mut sequence = 0_u16;
    let mut timestamp = 0_u32;
    let mut first = true;
    let ssrc = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u32)
        .unwrap_or(1))
        ^ std::process::id();
    let mut next = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now.duration_since(next) > Duration::from_millis(5) {
            // Do not burst to catch up; the receiver's playout buffer owns the
            // clock relationship and a burst would drain it.
            next = now;
        }
        let (lock, _) = &*queue;
        let mut mono = [0.0_f32; FRAMES_PER_PACKET];
        let mut queue = lock.lock().unwrap();
        let available = queue.len().min(FRAMES_PER_PACKET);
        for sample in mono.iter_mut().take(available) {
            *sample = queue.pop_front().unwrap_or(0.0);
        }
        drop(queue);
        if available == 0 && stop.load(Ordering::Relaxed) {
            break;
        }

        let mut stereo = [0.0_f32; FRAMES_PER_PACKET * 2];
        for (index, sample) in mono.iter().enumerate() {
            stereo[index * 2] = *sample;
            stereo[index * 2 + 1] = *sample;
        }
        if let Some((_, writer)) = writer.as_mut() {
            for sample in stereo {
                let _ = writer.write_sample(sample);
            }
        }
        let packet = encode_rtp(sequence, timestamp, first, ssrc, &stereo);
        let _ = socket.send(&packet);
        sequence = sequence.wrapping_add(1);
        timestamp = timestamp.wrapping_add(FRAMES_PER_PACKET as u32);
        first = false;
        next += Duration::from_millis(1);
    }
    if let Some((path, writer)) = writer.take() {
        if let Err(error) = writer.finalize() {
            eprintln!("audio: finalize {}: {error}", path.display());
        } else {
            eprintln!("audio: finalized {}", path.display());
        }
    }
}

fn encode_rtp(sequence: u16, timestamp: u32, marker: bool, ssrc: u32, stereo: &[f32]) -> Vec<u8> {
    const HEADER_BYTES: usize = 12;
    let mut packet = Vec::with_capacity(HEADER_BYTES + stereo.len() * 3);
    packet.push(0x80);
    packet.push((u8::from(marker) << 7) | RTP_PAYLOAD_TYPE);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    for sample in stereo {
        let value = (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
        let bytes = value.to_be_bytes();
        packet.extend_from_slice(&bytes[1..4]);
    }
    packet
}
