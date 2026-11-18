#!/usr/bin/env python3
"""Convert the upstream `pulseaudio-equalizer-ladspa` presets to JSON.

The upstream format is a plain-text file with one value per line:

    mbeq_1197        LADSPA plugin id
    mbeq             LADSPA label
    Multiband EQ     plugin name
    2.0              preamp (linear multiplier)
    Flat             preset name
    15               number of bands
    0 ...            one gain per band, in dB
    50 ...           one centre frequency per band, in Hz

We convert it to JSON, keeping the gains in dB and turning the preamp into dB
as well, so every value in the project is in the same unit.

Usage:
    convert_presets.py <upstream-preset-dir> <output-dir>
"""

from __future__ import annotations

import json
import math
import pathlib
import sys

NUM_BANDS = 15
EXPECTED_FREQS = [
    50.0, 100.0, 156.0, 220.0, 311.0, 440.0, 622.0, 880.0, 1250.0, 1750.0,
    2500.0, 3500.0, 5000.0, 10000.0, 20000.0,
]


def linear_to_db(linear: float) -> float:
    if linear <= 0:
        return -30.0
    return round(20.0 * math.log10(linear), 4)


def parse_preset(path: pathlib.Path) -> dict:
    lines = [ln for ln in path.read_text().splitlines() if ln.strip()]
    if len(lines) < 6:
        raise ValueError(f"{path}: expected at least 6 lines, got {len(lines)}")

    preamp_linear = float(lines[3])
    num_bands = int(lines[5])
    if num_bands != NUM_BANDS:
        raise ValueError(f"{path}: expected {NUM_BANDS} bands, got {num_bands}")

    needed = 6 + num_bands * 2
    if len(lines) < needed:
        raise ValueError(f"{path}: expected {needed} lines, got {len(lines)}")

    gains = [round(float(lines[6 + i]), 4) for i in range(num_bands)]
    freqs = [float(lines[6 + num_bands + i]) for i in range(num_bands)]

    for i, (got, want) in enumerate(zip(freqs, EXPECTED_FREQS)):
        if abs(got - want) > 0.5:
            raise ValueError(f"{path}: band {i} at {got} Hz, expected {want} Hz")

    # The upstream UI listed presets by file name, not by the name stored
    # inside the file (which is truncated in a couple of them). Use the file
    # name so the names match what users are used to.
    name = path.stem

    return {
        "name": name,
        "preamp_db": linear_to_db(preamp_linear),
        "gains_db": gains,
    }


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2

    src = pathlib.Path(sys.argv[1])
    dst = pathlib.Path(sys.argv[2])
    dst.mkdir(parents=True, exist_ok=True)

    converted = []
    for preset_path in sorted(src.glob("*.preset")):
        try:
            preset = parse_preset(preset_path)
        except ValueError as exc:
            print(f"skipping {preset_path.name}: {exc}", file=sys.stderr)
            continue

        slug = preset_path.stem.lower().replace(" ", "-").replace("&", "and")
        out = dst / f"{slug}.json"
        out.write_text(json.dumps(preset, indent=2) + "\n")
        converted.append(preset["name"])

    print(f"converted {len(converted)} presets into {dst}")
    for name in converted:
        print(f"  - {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
