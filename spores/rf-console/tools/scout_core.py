"""Shared tinySA scanning for the one-shot tool and the live scout service."""
from __future__ import annotations

import glob
import json
import re
import statistics
import time
from pathlib import Path

try:
    import serial
    from serial.tools import list_ports
except ImportError as exc:  # pragma: no cover - host dependency
    serial = None
    list_ports = None
    SERIAL_IMPORT_ERROR = exc
else:
    SERIAL_IMPORT_ERROR = None


PRESET_KEYS = ("name", "freq", "span", "device", "category")


def presets_from_page(path: Path) -> list[dict]:
    """Read the Soil-generated page state without duplicating the preset list."""
    text = path.read_text()
    match = re.search(r'<script id="rf-state" type="application/json">(.*?)</script>', text, re.S)
    if not match:
        raise SystemExit(f"no #rf-state in {path}")
    state = json.loads(match.group(1))
    return [
        {key: preset[key] for key in PRESET_KEYS if key in preset}
        for preset in state["presets"]
    ]


def serial_candidates() -> list[str]:
    candidates: list[str] = []
    if list_ports is not None:
        for info in list_ports.comports():
            device = info.device or ""
            text = " ".join(
                str(value or "")
                for value in (device, info.description, info.manufacturer, info.product)
            ).lower()
            if "tinysa" in text or "tiny sa" in text:
                candidates.insert(0, device)
            elif re.search(r"/dev/(ttyACM|ttyUSB|cu\.usbmodem|cu\.usbserial|serial/by-id)", device):
                if device not in candidates:
                    candidates.append(device)
    for pattern in (
        "/dev/ttyACM*",
        "/dev/ttyUSB*",
        "/dev/cu.usbmodem*",
        "/dev/cu.usbserial*",
        "/dev/serial/by-id/*",
    ):
        candidates.extend(sorted(glob.glob(pattern)))
    seen = set()
    return [path for path in candidates if not (path in seen or seen.add(path))]


def find_port(explicit: str | None = None) -> tuple[str | None, str | None]:
    if SERIAL_IMPORT_ERROR is not None:
        return None, f"pyserial is not installed: {SERIAL_IMPORT_ERROR}"
    if explicit:
        return explicit, None
    candidates = serial_candidates()
    if not candidates:
        return None, "no USB serial device found"
    return candidates[0], None


class TinySA:
    """One serial session. The caller owns reconnect policy."""

    def __init__(self, port: str, points: int = 200):
        if serial is None:
            raise RuntimeError(f"pyserial is not installed: {SERIAL_IMPORT_ERROR}")
        self.points = points
        self.port = serial.Serial(port, 115200, timeout=2)
        self.port.setDTR(False)
        self.port.setRTS(False)
        time.sleep(0.3)

    def command(self, text: str, wait: float, limit: int = 20000) -> str:
        self.port.reset_input_buffer()
        self.port.write(text.encode() + b"\r")
        time.sleep(wait)
        return self.port.read(limit).decode("ascii", "ignore")

    def amplitudes(self, start_hz: float, stop_hz: float) -> list[float]:
        start = max(0, int(start_hz))
        stop = max(start + 1, int(stop_hz))
        self.command(f"sweep {start} {stop} {self.points}", 0.3, 2000)
        self.command("scan", 0.4, 2000)
        data = self.command("data 0", 0.5, 20000)
        values: list[float] = []
        for line in data.splitlines():
            try:
                values.append(float(line.strip()))
            except ValueError:
                pass
        return values[-self.points :]

    def scan_preset(self, preset: dict) -> dict:
        center = float(preset["freq"]) * 1e6
        span = float(preset["span"]) * 1e6
        start = center - span / 2
        values = self.amplitudes(start, center + span / 2)
        if len(values) < 20:
            return {"error": "no data"}
        noise = statistics.median(values)
        peak = max(values)
        peak_index = values.index(peak)
        peak_hz = start + (peak_index / (len(values) - 1)) * span
        return {
            "peak_dbm": round(peak, 1),
            "peak_hz": round(peak_hz),
            "noise_dbm": round(noise, 1),
            "snr_db": round(peak - noise, 1),
            "center_dbm": round(values[len(values) // 2], 1),
        }

    def restore(self) -> None:
        self.command("sweep 0 1000000000 200", 0.3, 2000)

    def close(self) -> None:
        try:
            self.restore()
        finally:
            self.port.close()
