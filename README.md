# pipewire-equalizer

A fifteen band graphic equalizer for PipeWire, and a native port of the
abandoned [`pulseaudio-equalizer-ladspa`](https://github.com/pulseaudio-equalizer-ladspa/equalizer).

The upstream project stopped in 2019. Its author closed the [PipeWire support
request](https://github.com/pulseaudio-equalizer-ladspa/equalizer/issues/64)
with *"This project is currently abandoned too, so I'd recommend waiting for
another equalizer project for Pipewire to pop up."* This is that project.

It has no PulseAudio, no `pacmd` and no LADSPA: the filters are native
PipeWire biquads, they run inside the audio server, and the band gains are
changed while the audio plays.

```
┌──────────────┐   control socket   ┌──────────────┐   PipeWire graph   ┌─────────┐
│  pweq-gui    │◄─────────────────► │  pweq-daemon │ ─────────────────► │ speaker │
│  GTK4 / Adw  │   JSON over a      │              │  filter-chain     │         │
│  15 sliders  │   Unix socket      │              │  with 16 biquads  │         │
└──────────────┘                    └──────────────┘                    └─────────┘
```

## What it does

* Fifteen bands plus a preamp, adjustable while the audio is running.
* The 26 presets of the upstream project, converted to JSON.
* A GTK4 / libadwaita interface and a control socket for scripts.
* The filtering happens inside the PipeWire daemon, so it costs almost no
  CPU and no extra process sits in the audio path.

## Requirements

* PipeWire 1.0 or newer
* WirePlumber
* Rust 1.80 or newer
* The PipeWire development files (`libpipewire-0.3-dev`) and, for the
  interface, GTK4 (`libgtk-4-dev`) and libadwaita (`libadwaita-1-dev`)

On Debian and Ubuntu:

```sh
sudo apt install libpipewire-0.3-dev libspa-0.2-dev libgtk-4-dev libadwaita-1-dev
```

## Build

```sh
cargo build --release
```

The binaries land in `target/release/`:

| Binary | Role |
|---|---|
| `pweq-daemon` | Owns the equalizer node and answers the control socket |
| `pweq-gui` | The graphical interface |

## Install

```sh
./install.sh
```

It installs into `~/.local` and enables a systemd user service for the
daemon. Nothing is installed system wide and no root is needed.

To remove it:

```sh
./install.sh --uninstall
```

## Use

```sh
pweq-gui
```

The interface shows a slider per band. Moving one changes the filter while
the music plays; there is no restart and no gap.

The equalizer appears in the sound menu as **PipeWire Equalizer**. Select it
to route the audio through it, and select the device itself to bypass it.

The node plays into the output that was the default when the daemon
started. Set `PWEQ_TARGET` to a sink name to pin it to another one.

### It does not take over the sound by itself

The equalizer starts **switched off**, and the sink the user had chosen
stays the default. It only takes over when it is switched on from the
interface, and it gives the sink back when it is switched off.

This matters: moving the default to a filter that cannot start would leave
the session without sound, which happens when the audio device has no
driver yet. The equalizer refuses to switch on in that case and says so
in the status line.

### From the command line

The daemon speaks JSON over a Unix socket, one request per line:

```sh
# the current state
printf '{"cmd":"get_state"}\n' | socat - UNIX-CONNECT:/run/user/$UID/pweq.sock

# set a band
printf '{"cmd":"set_band","band":3,"gain":-6.0}\n' | socat - UNIX-CONNECT:/run/user/$UID/pweq.sock

# load a preset
printf '{"cmd":"load_preset","name":"Rock"}\n' | socat - UNIX-CONNECT:/run/user/$UID/pweq.sock
```

## Presets

The presets live in `~/.config/pweq/presets`. Two formats are read:

* **JSON**, the native format:

  ```json
  {
    "name": "My Preset",
    "preamp_db": 0.0,
    "gains_db": [0.0, 0.0, 3.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  }
  ```

* **The legacy format** of `pulseaudio-equalizer-ladspa`, so the old presets
  can be dropped in unchanged:

  ```text
  mbeq_1197
  mbeq
  Multiband EQ
  2.0
  Flat
  15
  0
  ... one gain per band ...
  50
  ... one frequency per band ...
  ```

The 26 presets of the upstream project are in [`presets/`](presets), converted
with `tools/convert_presets.py`.

## How faithful is it to the original?

The upstream equalizer was not a biquad equalizer. Its plugin, `mbeq_1197`
from SWH Plugins, works in the frequency domain: a 1024 point FFT with a
raised cosine window and 75% overlap, where the gain is interpolated linearly
between the band centres.

A cascade of biquads is a different construction: the response of every
filter adds up and the filters overlap. That is what a graphic equalizer is
expected to do, but it is not the same curve.

The band centres and the Q factors were fitted to get as close as possible.
`tools/compare_response.py` measures the difference, with the very
coefficients PipeWire computes:

```sh
$ python3 tools/compare_response.py --preset presets/rock.json
preset: Rock (presets/rock.json)
sample rate: 48000 Hz, 2000 points from 20.0 to 20000.0 Hz

  worst error: -4.48 dB at 439.2 Hz
  rms error:   1.19 dB
```

Two details of PipeWire's own implementation are worth knowing, because they
change what the sliders do:

* the shelving filters have a fixed slope, so their Q is fixed and the `Q`
  control has no effect on them;
* the shelving filters are computed with `A = 10^(dB/40)`, so they deliver
  half the gain the label says: 6 dB of shelf comes out as about 3 dB.

Both are in PipeWire's `filter-chain` module, not in this project.

## What was verified, and what was not

Verified on PipeWire 1.0.5 and WirePlumber 1.0.5:

* the daemon creates the equalizer node in the running PipeWire daemon, and
  the node shows up in the graph like any other sink;
* moving a band in the interface changes the gain of the live node without
  recreating the filter (`band_peaking3:Gain` went from 0 to -6 dB while the
  node kept running);
* loading a preset moves all fifteen gains of the node at once;
* the filter graph is built from native PipeWire biquads, so no LADSPA plugin
  and no extra process sit in the audio path.

Not verified: the audio actually coming out of the speakers with the filter
applied. In the session where this was developed the audio device never
started a driver, so every stream stayed suspended, including a plain
`pw-play` straight to the ALSA device. The routing was therefore verified at
the graph level (the node is created, the control ports are addressed and
changed) and not by listening.

## Layout

```
crates/pweq-core    Presets, band definitions, filter graph, control protocol
crates/pweq-daemon  The PipeWire node, the control socket
crates/pweq-gui     The GTK4 interface
presets             The 26 presets of the upstream project, in JSON
tools               Preset conversion and response measurement
```

## Licence

GPL-3.0-or-later, like the upstream project it is derived from. The presets
come from `pulseaudio-equalizer-ladspa` and keep that licence.
