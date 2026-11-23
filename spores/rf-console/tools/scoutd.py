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

from scout_core import TinySA, find_port, presets_from_page

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
                self.update(
                    status="scanning",
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

    def do_GET(self) -> None:
        path = self.path.split("?", 1)[0]
        if path == "/scout.json":
            body = json.dumps(self.scout.snapshot()).encode()
            content_type = "application/json; charset=utf-8"
        elif path in ("/", "/healthz"):
            state = self.scout.snapshot()
            body = json.dumps(
                {"ok": state["status"] == "live", "status": state["status"], "serial": state["serial"]}
            ).encode()
            content_type = "application/json; charset=utf-8"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self) -> None:
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.end_headers()

    def log_message(self, fmt: str, *args) -> None:
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
