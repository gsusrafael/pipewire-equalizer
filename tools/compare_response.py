#!/usr/bin/env python3
"""Measure how close the biquad chain is to the curve of the original plugin.

The upstream `mbeq_1197` plugin is not a biquad equalizer: it works in the
frequency domain (1024 point FFT, raised cosine window, 75% overlap) and
interpolates the gain linearly between band centres.

A cascade of biquads works differently: the response of every filter adds up,
and the filters overlap. The result is what a graphic equaliser is expected to
do, and it is not the same curve as the one of the original plugin. This
script measures the difference, so the claim can be checked instead of
assumed.

The coefficients follow the implementation of PipeWire's `filter-chain`
module (`src/modules/module-filter-chain/biquad.c`), so what is measured here
is what the equalizer really does. Two details of that implementation are
worth knowing:

* the shelving filters have a fixed Q (the slope is hard coded to 1, which
  gives an effective Q of 1/sqrt(2)), so the `Q` control has no effect on
  them;
* the shelving filters are computed with `A = 10^(dB/40)`, so they deliver
  half the gain the label says: 6 dB of shelf comes out as about 3 dB.

Usage:
    compare_response.py [--preset PRESETS/rock.json] [--points 2000]
"""

from __future__ import annotations

import argparse
import cmath
import json
import math
import pathlib
import sys

# The band centres, the Q values and the filter kind of every band.
BANDS = [
    (50.00, 0.71, "lowshelf"),
    (100.00, 0.30, "peaking"),
    (155.56, 2.55, "peaking"),
    (220.00, 2.80, "peaking"),
    (311.13, 3.30, "peaking"),
    (440.00, 4.40, "peaking"),
    (622.25, 4.05, "peaking"),
    (880.00, 3.50, "peaking"),
    (1244.51, 5.65, "peaking"),
    (1760.00, 4.50, "peaking"),
    (2489.02, 2.10, "peaking"),
    (3519.95, 7.35, "peaking"),
    (4978.04, 4.10, "peaking"),
    (9956.08, 0.75, "peaking"),
    (19912.16, 0.71, "highshelf"),
]

CENTRES = [b[0] for b in BANDS]


def db(value: float) -> float:
    return 20 * math.log10(max(value, 1e-12))


def biquad_response_db(
    kind: str, freq: float, centre: float, q: float, gain_db: float, rate: float
) -> float:
    """Response in dB of one biquad at `freq`.

    The coefficients follow the implementation of PipeWire's `filter-chain`
    module, so the measurement matches what the equalizer really does.
    """
    w0 = math.pi * (centre / rate)
    k = math.cos(w0)
    a = 10 ** (gain_db / 40)

    if kind == "peaking":
        alpha = math.sin(w0) / (2 * q)
        b0 = 1 + alpha * a
        b1 = -2 * k
        b2 = 1 - alpha * a
        a0 = 1 + alpha / a
        a1 = -2 * k
        a2 = 1 - alpha / a
    elif kind in ("lowshelf", "highshelf"):
        # PipeWire fixes the slope of the shelving filters at 1, so their Q is
        # fixed as well and the `q` argument is ignored.
        slope = 1.0
        alpha = 0.5 * math.sin(w0) * math.sqrt((a + 1 / a) * (1 / slope - 1) + 2)
        k2 = 2 * math.sqrt(a) * alpha
        a_plus_one = a + 1
        a_minus_one = a - 1
        if kind == "lowshelf":
            b0 = a * (a_plus_one - a_minus_one * k + k2)
            b1 = 2 * a * (a_minus_one - a_plus_one * k)
            b2 = a * (a_plus_one - a_minus_one * k - k2)
            a0 = a_plus_one + a_minus_one * k + k2
            a1 = -2 * (a_minus_one + a_plus_one * k)
            a2 = a_plus_one + a_minus_one * k - k2
        else:
            b0 = a * (a_plus_one + a_minus_one * k + k2)
            b1 = -2 * a * (a_minus_one + a_plus_one * k)
            b2 = a * (a_plus_one + a_minus_one * k - k2)
            a0 = a_plus_one - a_minus_one * k + k2
            a1 = 2 * (a_minus_one - a_plus_one * k)
            a2 = a_plus_one - a_minus_one * k - k2
    else:
        raise ValueError(f"unknown filter {kind}")

    wf = math.pi * (freq / rate)
    z = cmath.exp(1j * wf)
    num = b0 + b1 / z + b2 / (z * z)
    den = a0 + a1 / z + a2 / (z * z)
    return db(abs(num / den))


def chain_response_db(freq: float, gains: list[float], rate: float) -> float:
    """Response in dB of the whole chain at a given frequency."""
    total = 0.0
    for (centre, q, kind), gain in zip(BANDS, gains):
        total += biquad_response_db(kind, freq, centre, q, gain, rate)
    return total


def target_response_db(freq: float, gains: list[float]) -> float:
    """Response in dB of the curve the original plugin produces."""
    if freq <= CENTRES[0]:
        return gains[0]
    if freq >= CENTRES[-1]:
        return gains[-1]
    for i in range(len(CENTRES) - 1):
        if CENTRES[i] <= freq <= CENTRES[i + 1]:
            delta = (freq - CENTRES[i]) / (CENTRES[i + 1] - CENTRES[i])
            return (1 - delta) * gains[i] + delta * gains[i + 1]
    return gains[-1]


def interpolate(centres: list[float], gains: list[float], freq: float) -> float:
    """Linear interpolation in dB between the band centres."""
    if freq <= centres[0]:
        return gains[0]
    if freq >= centres[-1]:
        return gains[-1]
    for i in range(len(centres) - 1):
        if centres[i] <= freq <= centres[i + 1]:
            delta = (freq - centres[i]) / (centres[i + 1] - centres[i])
            return (1 - delta) * gains[i] + delta * gains[i + 1]
    return gains[-1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preset", default="presets/rock.json")
    parser.add_argument("--rate", type=float, default=48000.0)
    parser.add_argument("--points", type=int, default=2000)
    parser.add_argument("--fmin", type=float, default=20.0)
    parser.add_argument("--fmax", type=float, default=20000.0)
    parser.add_argument(
        "--bands-only",
        action="store_true",
        help="compare only the thirteen peaking filters, without the shelves",
    )
    args = parser.parse_args()

    preset_path = pathlib.Path(args.preset)
    if not preset_path.exists():
        print(f"no such preset: {preset_path}", file=sys.stderr)
        return 2

    preset = json.loads(preset_path.read_text())
    gains = preset["gains_db"]
    name = preset["name"]

    ratio = (args.fmax / args.fmin) ** (1 / (args.points - 1))
    errors = []
    for i in range(args.points):
        freq = args.fmin * ratio**i
        if args.bands_only:
            # Without the shelving filters, the comparison is against the
            # interpolation of the thirteen inner bands only.
            inner_gains = gains[1:-1]
            inner_centres = CENTRES[1:-1]
            got = 0.0
            for (centre, q, kind), gain in zip(BANDS[1:-1], inner_gains):
                got += biquad_response_db(kind, freq, centre, q, gain, args.rate)
            want = interpolate(inner_centres, inner_gains, freq)
        else:
            got = chain_response_db(freq, gains, args.rate)
            want = target_response_db(freq, gains)
        errors.append((freq, got - want))

    worst = max(errors, key=lambda e: abs(e[1]))
    rms = math.sqrt(sum(e[1] ** 2 for e in errors) / len(errors))

    print(f"preset: {name} ({preset_path})")
    print(
        f"sample rate: {args.rate:.0f} Hz, {args.points} points "
        f"from {args.fmin} to {args.fmax} Hz"
    )
    if args.bands_only:
        print("comparison: the thirteen peaking filters only")
    print()
    print(f"  worst error: {worst[1]:+.2f} dB at {worst[0]:.1f} Hz")
    print(f"  rms error:   {rms:.2f} dB")
    print()

    print("per band (response at the band centre):")
    for (centre, q, kind), gain in zip(BANDS, gains):
        got = chain_response_db(centre, gains, args.rate)
        want = target_response_db(centre, gains)
        print(
            f"  {centre:9.2f} Hz  {kind:10s} Q={q:4.2f}  "
            f"target {want:+6.2f} dB   chain {got:+6.2f} dB   "
            f"diff {got - want:+5.2f} dB"
        )

    return 0


if __name__ == "__main__":
    sys.exit(main())
