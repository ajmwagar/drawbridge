#!/usr/bin/env python3
"""Live tinySA presence service for the RF Console band map.

The native host owns the USB device, the serial session, and the scan cache.
The Soil web surface only polls `GET /scout.json`; it never opens serial.

Run on the machine with the tinySA attached:

    ./tools/scoutd.py --listen 127.0.0.1:8798 --antenna AR-771

For a remote host, bind the tailnet interface and point frontdoor at it:

    ./tools/scoutd.py --listen 100.74.130.83:8798
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import signal
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from scout_core import TinySA, find_port, presets_from_page

try:
    from sdr_core import SDRManager
except Exception as exc:  # SoapySDR/numpy are optional for a scout-only host
    SDRManager = None
    SDR_IMPORT_ERROR = exc
else:
    SDR_IMPORT_ERROR = None

UTC = dt.timezone.utc


def now() -> str:
    return dt.datetime.now(UTC).isoformat()


class Scout:
    def __init__(self, page: Path, explicit_port: str | None, antenna: str, points: int, retry: float):
        self.page = page
        self.explicit_port = explicit_port
        self.antenna = antenna
        self.points = points
        self.retry = retry
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.state = {
            "source": "tinySA4",
            "status": "starting",
            "antenna": antenna,
            "serial": None,
            "scanned_at": None,
            "scan_started_at": None,
            "scan_count": 0,
            "error": None,
            "presets": {},
        }

    def snapshot(self) -> dict:
        with self.lock:
            return json.loads(json.dumps(self.state))

    def update(self, **values) -> None:
        with self.lock:
            self.state.update(values)

    def set_reading(self, name: str, reading: dict) -> None:
        with self.lock:
            self.state["presets"][name] = reading
            self.state["error"] = None

    def run(self) -> None:
        while not self.stop.is_set():
            port, error = find_port(self.explicit_port)
            if not port:
                self.update(status="offline", serial=None, error=error)
                self.stop.wait(self.retry)
                continue
            radio = None
            try:
                radio = TinySA(port, points=self.points)
                # Keep the last complete pass labeled live while a later pass
                # is running. Only the very first pass is "scanning".
                self.update(
                    status="live" if self.state["scanned_at"] else "scanning",
                    serial=port,
                    error=None,
                    scan_started_at=now(),
                    scan_count=self.state["scan_count"] + 1,
                )
                presets = presets_from_page(self.page)
                for preset in presets:
                    if self.stop.is_set():
                        break
                    reading = radio.scan_preset(preset)
                    self.set_reading(preset["name"], reading)
                    print(
                        f"{preset['name']}: {reading.get('peak_dbm')} dBm, "
                        f"{reading.get('snr_db')} dB SNR",
                        flush=True,
                    )
                # A complete pass is a useful boundary; a live UI can show age.
                # Do not call a partial pass live: the band map needs every card.
                self.update(status="live", scanned_at=now(), scan_started_at=now())
            except Exception as exc:  # serial unplugged, permissions, protocol reset
                self.update(status="offline", error=str(exc), serial=port)
                print(f"scout offline: {exc}", flush=True)
                self.stop.wait(self.retry)
            finally:
                if radio is not None:
                    try:
                        radio.close()
                    except Exception:
                        pass


class Handler(BaseHTTPRequestHandler):
    scout: Scout
    sdrs: object | None = None
    protocol_version = "HTTP/1.1"

    def send_json(self, body: object, status: int = 200) -> None:
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/scout.json":
            self.send_json(self.scout.snapshot())
        elif path in ("/", "/healthz"):
            state = self.scout.snapshot()
            self.send_json(
                {"ok": state["status"] == "live", "status": state["status"], "serial": state["serial"]}
            )
        elif path == "/sdr/devices":
            if self.sdrs is None:
                self.send_json({"error": f"SoapySDR unavailable: {SDR_IMPORT_ERROR}"}, 503)
                return
            found = self.sdrs.devices()
            self.send_json(
                {
                    "source": "soapy",
                    "devices": found,
                    "error": None if found else "no SoapySDR devices found",
                }
            )
        elif path == "/sdr/frame":
            self.frame_sdr(parse_qs(parsed.query))
        elif path == "/sdr/stream":
            self.stream_sdr(parse_qs(parsed.query))
        else:
            self.send_error(404)

    def _sdr_request(self, query: dict[str, list[str]]):
        if self.sdrs is None:
            return None, None, f"SoapySDR unavailable: {SDR_IMPORT_ERROR}"
        device = (query.get("device") or [""])[0].strip().lower()
        if device in ("adsb", "ais"):
            device = "rtl"
        if not device:
            return None, None, "missing device"
        try:
            config = {
                "freq": float((query.get("freq") or ["100000000"])[0]),
                "span": max(1e5, float((query.get("span") or ["2000000"])[0])),
                "bins": max(128, min(2048, int((query.get("bins") or ["512"])[0]))),
                "bw": max(0.0, float((query.get("bw") or ["0"])[0])),
                "gain": float((query.get("gain") or ["20"])[0]),
            }
        except ValueError:
            return None, None, "invalid stream parameters"
        worker = self.sdrs.stream(device, config)
        if worker is None:
            return None, None, f"device not found: {device}"
        return worker, config, None

    def frame_sdr(self, query: dict[str, list[str]]) -> None:
        worker, _config, error = self._sdr_request(query)
        if error:
            self.send_json({"error": error}, 404 if "not found" in error else 400)
            return
        frame, _version = worker.wait(0, timeout=4.0)
        self.send_json(frame or {"error": "no frame yet"})

    def stream_sdr(self, query: dict[str, list[str]]) -> None:
        worker, _config, error = self._sdr_request(query)
        if error:
            self.send_json({"error": error}, 404 if "not found" in error else 400)
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        version = 0
        try:
            while not worker.stop_event.is_set():
                frame, version = worker.wait(version, timeout=2.0)
                if frame is None:
                    self.wfile.write(b": waiting\n\n")
                else:
                    self.wfile.write(b"data: " + json.dumps(frame).encode() + b"\n\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_OPTIONS(self) -> None:
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, fmt: str, *args) -> None:
        if "/sdr/frame" in self.path:
            return
        print(f"{self.address_string()} {fmt % args}", flush=True)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="127.0.0.1:8798")
    parser.add_argument("--port", default=None)
    parser.add_argument("--antenna", default="AR-771")
    parser.add_argument("--points", type=int, default=200)
    parser.add_argument("--retry", type=float, default=6.0)
    parser.add_argument("--page", type=Path, default=root / "web/index.html")
    args = parser.parse_args()

    host, _, port_text = args.listen.rpartition(":")
    if not host:
        host, port_text = "127.0.0.1", args.listen
    scout = Scout(args.page, args.port, args.antenna, args.points, args.retry)
    Handler.scout = scout
    Handler.sdrs = SDRManager() if SDRManager is not None else None
    thread = threading.Thread(target=scout.run, name="scout", daemon=True)
    thread.start()
    server = ThreadingHTTPServer((host, int(port_text)), Handler)

    def shutdown(*_args) -> None:
        scout.stop.set()
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    print(f"rf scout on http://{host}:{port_text}/scout.json", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
