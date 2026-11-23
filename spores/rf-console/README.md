# RF Deck — RF console prototype

A simulated multi-radio console: receiver rail, spectrum, waterfall, tuning,
demodulation controls, presets, signal meter, and capture state.

This is a UI prototype. Every reading is generated locally and labelled
`SIMULATED FEED`. No SDR is opened and no network or radio transport is used.

## Preview

- **Genre:** atmospheric
- **Macrostructure:** instrument console — rail / scope / inspector, not a
  marketing page
- **Theme:** field monitor — ink-green panels, bone text, amber carrier,
  cyan trace
- **Typography:** SF Mono / Cascadia / ui-monospace for data; Avenir Next /
  system sans for supporting copy
- **Slop test:** pass
- **Key decisions:** one receiver selection model for SDR, packet, and
  broadcast devices; draggable tuning; canvas waterfall; mock data explicitly
  marked; no fake browser chrome, invented metrics, or rounded card grid

## Run the UI

```sh
python3 -m http.server 8099 --directory web
# open http://127.0.0.1:8099
```

Or install the package into a Loam root once web surfaces are enabled:

```sh
loam install . --root ~/.mycelium/spores
loamd serve --web-listen 127.0.0.1:8799 --root ~/.mycelium/spores
```

## Direction

The eventual data plane should stay outside the Spore:

```text
Spore web surface  -> control capability -> native radio host
browser / wry      -> loopback FFT + PCM  -> native radio host
```

The Spore owns layout, policy, presets, and acknowledgement ordering. The
native host owns devices, tuning locks, DSP, recordings, credentials, and the
high-rate transport. This prototype is the UI shell that will sit on top of
that boundary.

## Interactions

- Click a receiver to switch devices.
- Drag the spectrum or use the mouse wheel to tune.
- Arrow keys nudge; space starts/stops RX.
- Presets switch device and tuning.
- Mode, bandwidth, gain, squelch, peak hold, and record controls are live.
