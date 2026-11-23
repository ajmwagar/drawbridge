"""SoapySDR spectrum workers for the RF Console native bridge.

The native side owns the USB devices, sample clocks, gain, and FFT. The web
surface only subscribes to bounded JSON frames over SSE. This is intentionally
a spectrum bridge, not the final Soil radio capability.
"""
from __future__ import annotations

import json
import math
import threading
import time
import datetime as dt

import numpy as np

try:
    import SoapySDR
    from SoapySDR import SOAPY_SDR_RX, SOAPY_SDR_CF32, SOAPY_SDR_TIMEOUT, SOAPY_SDR_OVERFLOW
except ImportError as exc:  # pragma: no cover - host dependency
    SoapySDR = None
    SOAPY_SDR_RX = SOAPY_SDR_CF32 = SOAPY_SDR_TIMEOUT = None
    SOAPY_IMPORT_ERROR = exc
else:
    SOAPY_IMPORT_ERROR = None

UTC = dt.timezone.utc
IQ_SAMPLES = 16384


def now() -> str:
    return dt.datetime.now(UTC).isoformat()


def _text(value) -> str:
    return str(value or "").strip()


def enumerate_devices() -> list[dict]:
    if SoapySDR is None:
        return []
    found = []
    try:
        infos = SoapySDR.Device.enumerate()
    except Exception:
        return []
    for info in infos:
        info = dict(info)
        driver = _text(info.get("driver")).lower()
        if driver == "hackrf":
            device_id = "hackrf"
            name = "HackRF One"
        elif driver == "rtlsdr":
            device_id = "rtl"
            name = "RTL-SDR Blog V4"
        else:
            continue
        serial = _text(info.get("serial"))
        found.append(
            {
                "id": device_id,
                "name": name,
                "driver": driver,
                "serial": serial,
                "label": _text(info.get("label")) or name,
                "kind": "sdr",
                "arg": f"driver={driver}" + (f",serial={serial}" if serial else ""),
            }
        )
    # Preserve a stable order in the UI.
    order = {"hackrf": 0, "rtl": 1}
    found.sort(key=lambda item: order.get(item["id"], 9))
    return found


def sample_rate_for(driver: str, span_hz: float) -> float:
    if driver == "hackrf":
        # HackRF accepts integer MS/s. Leave headroom over the displayed span.
        return float(max(1, min(20, math.ceil(span_hz / 1e6))) * 1e6)
    if driver == "rtlsdr":
        if span_hz <= 0.3e6:
            return 0.3e6
        if span_hz <= 1.0e6:
            return 1.2e6
        if span_hz <= 2.4e6:
            return 2.4e6
        return 3.2e6
    return max(1e6, min(20e6, math.ceil(span_hz / 1e6) * 1e6))


def bandwidth_for(driver: str, bw_hz: float) -> float:
    if driver == "hackrf":
        return max(1.75e6, min(28e6, bw_hz or 1.75e6))
    if driver == "rtlsdr":
        return max(0.0, min(8e6, bw_hz or 0.0))
    return bw_hz


def fft_row(iq: np.ndarray, bins: int) -> np.ndarray:
    window = np.hanning(len(iq)).astype(np.float32)
    spectrum = np.fft.fftshift(np.fft.fft(iq * window))
    magnitude = np.abs(spectrum) / max(1.0, float(np.sum(window)) / 2.0)
    dbfs = 20.0 * np.log10(np.maximum(magnitude, 1e-12))
    # Display-space interpolation keeps the wire frame bounded.
    x = np.linspace(0, len(dbfs) - 1, bins)
    row = np.interp(x, np.arange(len(dbfs)), dbfs)
    # Uncalibrated but useful dBm-like display. The broker says calibrated:false.
    return np.clip(row - 30.0, -130.0, -20.0).astype(np.float32)


class DeviceWorker:
    def __init__(self, spec: dict, config: dict):
        self.spec = spec
        self.config = config
        self.key = json.dumps(config, sort_keys=True, separators=(",", ":"))
        self.stop_event = threading.Event()
        self.cond = threading.Condition()
        self.frame = None
        self.version = 0
        self.error = None
        self.thread = threading.Thread(target=self.run, name=f"sdr-{spec['id']}", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()
        if self.thread.is_alive():
            self.thread.join(timeout=3)

    def wait(self, version: int, timeout: float = 2.0):
        with self.cond:
            self.cond.wait_for(lambda: self.version != version or self.stop_event.is_set(), timeout)
            return self.frame, self.version

    def publish(self, frame: dict) -> None:
        with self.cond:
            self.frame = frame
            self.version += 1
            self.cond.notify_all()

    def fail(self, message: str) -> None:
        with self.cond:
            self.frame = {"device": self.spec["id"], "error": message, "at": now()}
            self.version += 1
            self.cond.notify_all()

    def run(self) -> None:
        span = float(self.config.get("span", 2e6))
        freq = float(self.config.get("freq", 100e6))
        bins = int(self.config.get("bins", 512))
        rate = sample_rate_for(self.spec["driver"], span)
        bw = bandwidth_for(self.spec["driver"], float(self.config.get("bw", 0)))
        gain = float(self.config.get("gain", 20))
        retry = 2.0
        while not self.stop_event.is_set():
            device = None
            stream = None
            try:
                if SoapySDR is None:
                    raise RuntimeError(f"SoapySDR is not installed: {SOAPY_IMPORT_ERROR}")
                device = SoapySDR.Device(self.spec["arg"])
                device.setSampleRate(SOAPY_SDR_RX, 0, rate)
                device.setFrequency(SOAPY_SDR_RX, 0, freq)
                if bw:
                    try:
                        device.setBandwidth(SOAPY_SDR_RX, 0, bw)
                    except Exception:
                        pass
                try:
                    device.setGain(SOAPY_SDR_RX, 0, gain)
                except Exception:
                    pass
                try:
                    device.setAntenna(SOAPY_SDR_RX, 0, "RX")
                except Exception:
                    pass
                stream = device.setupStream(SOAPY_SDR_RX, SOAPY_SDR_CF32, [0])
                device.activateStream(stream)
                actual_rate = float(device.getSampleRate(SOAPY_SDR_RX, 0))
                actual_freq = float(device.getFrequency(SOAPY_SDR_RX, 0))
                buffer = np.empty(IQ_SAMPLES, np.complex64)
                ema = None
                while not self.stop_event.is_set():
                    result = device.readStream(stream, [buffer], len(buffer), timeoutUs=300000)
                    if result.ret in (SOAPY_SDR_TIMEOUT, SOAPY_SDR_OVERFLOW):
                        continue
                    if result.ret < 0:
                        raise RuntimeError(f"Soapy readStream failed: {result.ret}")
                    if result.ret == 0:
                        continue
                    row = fft_row(buffer[: result.ret], bins)
                    ema = row if ema is None else ema * 0.72 + row * 0.28
                    peak_index = int(np.argmax(ema))
                    self.publish(
                        {
                            "device": self.spec["id"],
                            "driver": self.spec["driver"],
                            "serial": self.spec["serial"],
                            "center_hz": actual_freq,
                            "span_hz": actual_rate,
                            "bin_hz": actual_rate / bins,
                            "bins": bins,
                            "dbm": [round(float(value), 1) for value in ema],
                            "peak_hz": actual_freq - actual_rate / 2 + (peak_index + 0.5) * actual_rate / bins,
                            "peak_dbm": round(float(np.max(ema)), 1),
                            "gain_db": gain,
                            "bandwidth_hz": bw,
                            "calibrated": False,
                            "at": now(),
                        }
                    )
            except Exception as exc:
                self.fail(str(exc))
                self.stop_event.wait(retry)
            finally:
                if device is not None and stream is not None:
                    try:
                        device.deactivateStream(stream)
                    except Exception:
                        pass
                    try:
                        device.closeStream(stream)
                    except Exception:
                        pass


class SDRManager:
    def __init__(self):
        self.lock = threading.Lock()
        self.current: DeviceWorker | None = None
        self._devices_cache: tuple[float, list[dict]] | None = None

    def devices(self) -> list[dict]:
        with self.lock:
            if self._devices_cache and time.monotonic() - self._devices_cache[0] < 5:
                return self._devices_cache[1]
        found = enumerate_devices()
        with self.lock:
            self._devices_cache = (time.monotonic(), found)
        return found

    def spec(self, device_id: str) -> dict | None:
        for item in self.devices():
            if item["id"] == device_id:
                return item
        return None

    def stream(self, device_id: str, config: dict) -> DeviceWorker | None:
        spec = self.spec(device_id)
        if spec is None:
            return None
        with self.lock:
            wanted = DeviceWorker(spec, config)
            if self.current and self.current.spec["id"] == device_id and self.current.key == wanted.key:
                return self.current
            old = self.current
            self.current = wanted
            wanted.start()
        if old is not None:
            old.stop()
        return wanted
