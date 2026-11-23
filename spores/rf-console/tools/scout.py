#!/usr/bin/env python3
"""Scan the RF Deck presets once with a tinySA and write web/scout.json.

Prefer `tools/scoutd.py` for a live feed. This one-shot remains useful for a
snapshot checked into the Spore or for testing the radio.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path

from scout_core import TinySA, find_port, presets_from_page


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", default=None, help="serial device; autodetect when omitted")
    parser.add_argument("--antenna", default="AR-771")
    parser.add_argument("--points", type=int, default=200)
    parser.add_argument("--page", type=Path, default=root / "web/index.html")
    parser.add_argument("--out", type=Path, default=root / "web/scout.json")
    args = parser.parse_args()

    port, error = find_port(args.port)
    if not port:
        raise SystemExit(error)
    presets = presets_from_page(args.page)
    radio = TinySA(port, points=args.points)
    result = {
        "source": "tinySA4",
        "serial": port,
        "antenna": args.antenna,
        "scanned_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "points": args.points,
        "presets": {},
    }
    try:
        for index, preset in enumerate(presets, 1):
            reading = radio.scan_preset(preset)
            result["presets"][preset["name"]] = reading
            print(
                f"{index:02d}/{len(presets)} {preset['name']}: "
                f"{reading.get('peak_dbm')} dBm @ "
                f"{reading.get('peak_hz', 0) / 1e6:.4f} MHz, noise "
                f"{reading.get('noise_dbm')}, SNR {reading.get('snr_db')}",
                flush=True,
            )
    finally:
        radio.close()
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
