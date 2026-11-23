#!/usr/bin/env python3
"""Scan the RF Deck presets with a tinySA over USB and write web/scout.json.

Run on the Pi as the radio user, for example:

    sudo -u radioman ./tools/scout.py

The output is a point-in-time presence snapshot, not a live feed. The UI reads
it at load time and labels the source and antenna.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import statistics
from pathlib import Path

import serial


def presets_from_page(path: Path) -> list[dict]:
    text = path.read_text()
    match = re.search(r'<script id="rf-state" type="application/json">(.*?)</script>', text, re.S)
    if not match:
        raise SystemExit(f"no #rf-state in {path}")
    state = json.loads(match.group(1))
    return [
        {key: preset[key] for key in ("name", "freq", "span", "device", "category")}
        for preset in state["presets"]
    ]


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", default="/dev/ttyACM1")
    parser.add_argument("--antenna", default="AR-771")
    parser.add_argument("--points", type=int, default=200)
    parser.add_argument("--out", type=Path, default=root / "web/scout.json")
    args = parser.parse_args()

    presets = presets_from_page(root / "web/index.html")
    port = serial.Serial(args.port, 115200, timeout=2)
    port.setDTR(False)
    port.setRTS(False)
    import time

    time.sleep(0.3)

    def command(text: str, wait: float, limit: int = 20000) -> str:
        port.reset_input_buffer()
        port.write(text.encode() + b"\r")
        time.sleep(wait)
        return port.read(limit).decode("ascii", "ignore")

    def amplitudes(start_hz: float, stop_hz: float) -> list[float]:
        start = max(0, int(start_hz))
        stop = max(start + 1, int(stop_hz))
        command(f"sweep {start} {stop} {args.points}", 0.35, 2000)
        command("scan", 0.45, 2000)
        data = command("data 0", 0.55, 20000)
        values = []
        for line in data.splitlines():
            try:
                values.append(float(line.strip()))
            except ValueError:
                pass
        return values[-args.points :]

    result = {
        "source": "tinySA4",
        "antenna": args.antenna,
        "scanned_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "points": args.points,
        "presets": {},
    }
    for index, preset in enumerate(presets, 1):
        center = float(preset["freq"]) * 1e6
        span = float(preset["span"]) * 1e6
        start = center - span / 2
        values = amplitudes(start, center + span / 2)
        if len(values) < 20:
            result["presets"][preset["name"]] = {"error": "no data"}
            continue
        noise = statistics.median(values)
        peak = max(values)
        peak_index = values.index(peak)
        peak_hz = start + (peak_index / (len(values) - 1)) * span
        result["presets"][preset["name"]] = {
            "peak_dbm": round(peak, 1),
            "peak_hz": round(peak_hz),
            "noise_dbm": round(noise, 1),
            "snr_db": round(peak - noise, 1),
            "center_dbm": round(values[len(values) // 2], 1),
        }
        print(
            f"{index:02d}/{len(presets)} {preset['name']}: "
            f"{peak:.1f} dBm @ {peak_hz / 1e6:.4f} MHz, noise {noise:.1f}, "
            f"SNR {peak - noise:.1f}",
            flush=True,
        )

    command("sweep 0 1000000000 200", 0.35, 2000)
    port.close()
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
