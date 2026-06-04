# Leap Motion (Ultraleap) demo

A small visual playground for an Ultraleap Hand Tracking device (formerly Leap
Motion Controller / Controller 2 / 3Di / IR 170). The repo contains two
implementations of the same demo:

| Folder              | Language          | Purpose                                                 |
| ------------------- | ----------------- | ------------------------------------------------------- |
| `leap_demo_rs/`     | Rust + macroquad  | Visualiser — recommended starting point                 |
| `leap_demo.py`      | Python + pygame   | The same visualiser, first cut, fine for hacking        |
| `laser_harp_rs/`    | Rust + cpal + midir | **Jean-Michel-Jarre-style laser harp** with synth + MIDI |

Both draw a live skeleton for each tracked hand, fingertip trails, a pinch
glow between thumb and index, a palm circle that fills with grab strength,
a soft shadow on an imaginary table, depth cues, and a HUD with pinch/grab
meters and frame rates.

## Prerequisites

### 1. The Ultraleap device

Any Ultraleap-supported device works — Leap Motion Controller (1), Controller 2,
3Di, IR 170, SIR 170, Hyperion. Plug it in over USB. The notch / dots side
faces up, fingers tracked above.

### 2. Ultraleap Hand Tracking Software

This installer bundles the system service the device talks to **and** the
LeapSDK we link against. The SDK itself is not redistributable, so this repo
does not bundle it — but everything we need ships inside that installer.

Download from Ultraleap (free, account required):

  - macOS / Windows: <https://leap2.ultraleap.com/downloads/>
  - Linux (Ubuntu / Debian / Arch): same page, scroll to "Linux".

After install, the SDK lives at:

| OS      | Path                                                                            |
| ------- | ------------------------------------------------------------------------------- |
| macOS   | `/Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK/`                   |
| Linux   | `/usr/share/doc/ultraleap-hand-tracking-service/samples/` (with libs in `/usr/lib`) |
| Windows | `C:\Program Files\Ultraleap\LeapSDK\`                                           |

Verify the service is running by opening the **Ultraleap Control Panel** app
(macOS / Windows) or `sudo systemctl status ultraleap-hand-tracking-service`
(Linux). You should see your device listed under "Devices" and a tracking
preview when you wave a hand.

### 3. Toolchains

- **Rust demo:** stable Rust ≥ 1.75 (`rustup` recommended).
- **Python demo:** Python 3.12 (the SDK ships a precompiled cffi binding for
  3.12 only on macOS — see "How the Python version works" below).

## Build & run — Rust version

```sh
cd leap_demo_rs
./run.sh                  # cargo run --release
# or, after the first build:
./target/release/leap_demo_rs
```

The build script (`build.rs`) auto-detects the SDK location on macOS /
Linux / Windows. If you installed the SDK somewhere non-standard, point at
it explicitly:

```sh
LEAP_SDK_DIR=/opt/my-leap/lib cargo build --release
```

The path is baked into the binary as an absolute rpath, so no
`DYLD_LIBRARY_PATH` / `LD_LIBRARY_PATH` is required at runtime.

Keys: **Esc / Q** quits.

## Build & run — Laser harp

```sh
cd laser_harp_rs
./run.sh                  # cargo run --release
```

Same SDK lookup as the visualiser. First build pulls a few extra crates
(`cpal`, `midir`, `crossbeam-channel`) and takes ~30 s.

What you'll see and hear:

- **Five horizontal strings** stacked at increasing heights, coloured across
  the rainbow. Labels on the left show each string's note: C, D, E, G, A
  (C-major pentatonic — no wrong notes). To play, sweep a fingertip
  vertically THROUGH a string.
- **Hand X = octave.** Five octave zones across the play area span ±2
  octaves around middle C. The active column lights up in the top bar
  (−2 / −1 / 0 / +1 / +2). To shift a note up or down an octave, slide
  your hand sideways before strumming.
- **Fingers are independent voices.** Spread your fingers and sweep =
  arpeggio / chord. **Only EXTENDED fingers play** — curling a finger
  mutes it (we use the Leap SDK's `is_extended` flag). This is the same
  idea as fretting on a guitar: choose which fingers will speak.
- **Palm height drives brightness.** A global low-pass filter opens as you
  raise the palm — warm low, shimmer high.
- **Pinch ≥ 0.7 = sustain pedal.** Notes ring on through subsequent strums.
- **Five instruments per hand** — the left and right hand can play
  different sounds at the same time. `1`/`2`/`3`/`4`/`5` selects the right
  hand's instrument; **Shift+1..5** selects the left hand's. Defaults are
  Pluck (right) and FM Bell (left). Choices: `1` Pluck (Karplus-Strong,
  harp-like), `2` FM Bell, `3` Sine, `4` Saw, `5` Square.
- **MIDI out on a virtual port** named "Leap Laser Harp" (CoreMIDI on
  macOS, ALSA seq on Linux). The two hands send on **different MIDI
  channels** — right = channel 1, left = channel 2 — so a DAW can route
  them to different instruments natively. On macOS, any DAW (Logic,
  Ableton, GarageBand, Bitwig…) will see "Leap Laser Harp" as an input
  source automatically — no IAC bus setup required. Add two software-
  instrument tracks, set one to MIDI input channel 1 and the other to
  channel 2, and play both hands through completely different patches.
  The built-in synth keeps playing too — mute the built-in output via
  system volume if you only want the DAW sound.

Other keys: `[ / ]` transpose ± semitone, `space` panic (all notes off),
`esc / q` quit.

### Tweaking the harp

All tunables live at the top of `laser_harp_rs/src/main.rs`:

- `SCALE: [u8; N_STRINGS]` — MIDI notes for the five strings. Want a minor
  pentatonic? `[57, 60, 62, 64, 67]`. Whole tone? Hirajoshi? Change one
  array. If you change `N_STRINGS`, update `STRING_Y_MM` too.
- `STRING_Y_MM` — vertical position of each string above the device, in mm.
  Spread them further apart for slower, more deliberate playing; pack them
  closer for fast runs.
- `STRING_HALF_HEIGHT_MM` — Y tolerance for "fingertip is touching the
  string". Bigger = easier to play, more risk of accidental triggers.
- `OCT_ZONE_HALF_MM` / `N_OCTAVES` / `OCT_OFFSETS` — width and number of
  the X octave columns and which octave each one represents.
- `PINCH_SUSTAIN_THRESHOLD` — pinch strength that engages sustain.

Synth voices live in `src/synth.rs`; the ADSR envelopes and the FM-bell
modulator ratio are right there if you want to tune the bell brighter or
make the pluck ring longer.

## Build & run — Python version

```sh
cd /Users/chris/code/claude_world/leap_motion
python3.12 -m venv .venv
source .venv/bin/activate
pip install pygame cffi
python leap_demo.py
```

The script appends the bundled SDK directory
(`/Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK`) to `sys.path`
so the `leapc_cffi` module is found.

> Python 3.12 specifically: the SDK ships a precompiled CFFI binary named
> `_leapc_cffi.cpython-312-darwin.so`. Other Python versions need a rebuild,
> which is non-trivial. If you need 3.11 / 3.13, look at the
> [`leapc-python-bindings`](https://github.com/ultraleap/leapc-python-bindings)
> source.

## How it works

### Big picture

```
   ┌──────────────────────┐    USB     ┌──────────────────┐
   │ Ultraleap device     │ ─────────► │ tracking service │
   └──────────────────────┘            │ (system daemon)  │
                                       └──────┬───────────┘
                                              │ local IPC
                                              ▼
                                       ┌──────────────────┐
                                       │ libLeapC.dylib   │
                                       └──────┬───────────┘
                                              │ C ABI
                                              ▼
                                       ┌──────────────────┐
                                       │ this demo        │
                                       └──────────────────┘
```

You never talk to the device directly. The Ultraleap service owns the USB
connection and does the heavy lifting (CV → bone-tracked hands). Apps link
against `libLeapC` and call `LeapPollConnection()` to receive a stream of
events; the tracking events carry an array of `LEAP_HAND` structs.

### Rust demo architecture

- `src/leap.rs` — hand-rolled minimal FFI: just the structs and functions we
  use. **All structs are `#[repr(C, packed)]`** because the SDK header is
  wrapped in `#pragma pack(1)`. Field reads go through a `read_unaligned`
  helper (`get!` macro in `main.rs`) since taking references to fields of a
  packed struct is undefined behaviour in Rust.

  Compile-time `size_of` assertions check every struct against the values
  reported by the SDK header. If a future SDK changes the layout, the build
  fails loudly instead of crashing at runtime.

- `src/main.rs` —
  - A dedicated **poll thread** calls `LeapPollConnection(timeout=200ms)` in a
    loop, copies each tracking frame into a plain `HandSnap` (no FFI pointers
    captured), and stores the result in `Arc<Mutex<FrameSnap>>`.
  - The macroquad render thread (`#[macroquad::main]`) clones the snapshot
    every frame, then issues GPU draw calls: lines for bones, circles for
    joints, an ellipse for the shadow, alpha-faded line strips for trails.
  - Coordinate projection is in `project()` — a simple orthographic 2D
    mapping with a depth multiplier for size/thickness.

- `build.rs` — finds the SDK lib dir (env var or default), wires up
  `cargo:rustc-link-*` and bakes an absolute rpath.

### Python demo architecture

Same shape, smaller engine. A background `threading.Thread` polls via the
cffi binding and pushes hand snapshots into a `FrameState` guarded by a lock.
The main loop runs pygame at 60 fps, draws using `pygame.draw.line` /
`circle`, and uses `SRCALPHA` surfaces for the glow / trail effects. This is
the source of the "glitchy" feel in pygame — software blending of full-window
alpha surfaces per frame is slow.

### Coordinate system

Leap returns positions in **millimetres** relative to the centre of the
device:

- `+X` to the right (when looking at the device's USB-cable side)
- `+Y` straight up, away from the table
- `+Z` toward the user

A hand held ~25 cm above the device gives `palm.y ≈ 250`. The `project()`
helper centres `y = 250 mm` on screen so a hand at a comfortable height sits
in the middle of the window.

### Key data per hand

Each `LEAP_HAND` we read carries:

- `id`, `hand_type` (left / right), `confidence`, `visible_time`
- `pinch_distance` (mm between thumb and any finger tip),
  `pinch_strength` (0…1), `grab_strength` (0…1), `grab_angle`
- `palm` — position, stabilised position, velocity, normal, direction,
  orientation quaternion
- `digits[5]` — thumb, index, middle, ring, pinky. Each has 4 `bones`:
  metacarpal, proximal, intermediate, distal. Bones store `prev_joint`,
  `next_joint`, `width`, `rotation`. The thumb's metacarpal is zero-length by
  convention (see the SDK comments).
- `arm` — a single bone, `prev_joint` is the elbow, `next_joint` is the
  wrist.

That's the entire skeleton: 5 fingers × 4 bones + arm + palm. Everything you
see in the demo is rendered from these fields.

## Extending it — fun ideas

These are ordered roughly by effort.

### Quick (an afternoon)

1. **Air drawing pad.** Persist fingertip positions when a hand is pinching
   (`pinch_strength > 0.7`), into a `Vec<(Vec2, Color)>`. Render as a line
   strip on a second layer. Clear with a fist (high `grab_strength`). Use
   index-finger trails to "draw", thumb pinch to pick colour.

2. **Theremin.** Map right-hand Y to pitch (e.g. 200–1200 Hz) and left-hand Y
   to amplitude. Use `cpal` (Rust) or `pyaudio` to stream a sine wave. The
   reference is the classic Leap theremin demo — really pleasant feedback.

3. **OSC bridge.** Send `/leap/{left,right}/palm` (x, y, z, pinch, grab) as
   OSC messages to localhost. Suddenly you can drive Ableton Live, Reaper,
   TouchDesigner, Resolume, Max/MSP, or any visual patching tool from your
   hands. Crate: [`rosc`](https://crates.io/crates/rosc).

4. **Particle attractor.** Spawn particles at random screen edges; each
   frame, apply a force toward the projected palm position. On pinch, fire
   a burst of particles outward from the pinch midpoint. Trivially fun and
   the perf headroom makes it look great.

5. **Sign-language alphabet (subset).** The `is_extended` flag on each digit
   already gives you 32 finger combinations. Hard-code rules for A–E (fist,
   index out, peace sign, etc.) and display the recognised letter. Add
   timing (hold 500 ms to confirm) and you have a basic gesture recogniser.

### Medium (a weekend)

6. **3D rendering.** Swap the orthographic `project()` for a true 3D camera
   using `glam::Mat4` and a perspective matrix; render bones as capsules /
   cylinders. macroquad has a basic 3D mode, or move up to `wgpu` / `bevy`.
   This makes depth and pronation/supination of the hand actually readable.

7. **Pick-and-place physics.** Add a few cuboids in 3D. When the hand
   pinches near a cube (distance from pinch midpoint < threshold), parent
   the cube to the pinch. Release on un-pinch. Use `rapier3d` for gravity
   and collisions, hand becomes a kinematic body.

8. **Mouse / cursor replacement.** Compute a stable cursor position from the
   index fingertip, smoothing with an exponential moving average. Move the
   system cursor with [`enigo`](https://crates.io/crates/enigo) (cross-
   platform). Fast pinch = click, slow pinch = drag.

9. **Gesture-driven keyboard shortcuts.** Detect swipe-left / swipe-right /
   pinch-and-twist. Emit `Cmd+]` / `Cmd+[` / `Cmd+Space` via `enigo`. Great
   for browser tab control or slideshow remote.

10. **Volumetric sculpting.** Show a grid of cells; cells inside any
    fingertip sphere are "filled". Visualise as cubes; you build shapes by
    waving your hand through 3D space. Add undo (open palm wipe), save to
    `.obj`.

### Bigger (multi-week)

11. **Multiplayer hands.** Stream hand snapshots over UDP / WebSocket;
    receive on another instance and render both sets. Add a simple
    rate-limit / interpolation step and you've made a tiny CSCW toy. The
    `HandSnap` struct is already plain data — `bincode` or `postcard` will
    serialise it directly.

12. **In-app text input.** Air-keyboard layout floating in front of the
    user; recognise pinch-tap on each key. Combine with a language model
    for autocorrect. (Disclaimer: typing in mid-air is hard. But fun.)

13. **MIDI controller surface.** Render a virtual mixer; the palm normal,
    pinch and grab become 3 continuous CC outputs per hand. Plays well with
    your DAW. Use [`midir`](https://crates.io/crates/midir).

14. **Two-hand sculpting.** Two-handed pinch = stretch / compress / rotate a
    mesh. Subdivision surface that you mould with palm normals. Export
    `.glb` / `.obj`.

### Things I'd reach for from `leap.rs`

A few SDK features the demo doesn't yet expose but would unlock the above:

- **Tracking modes.** `LeapSetTrackingMode(conn, mode)` — `HMD` mode if the
  device is mounted to a VR headset facing forward; `ScreenTop` if mounted
  above the monitor; `Desktop` (default) for the device lying flat.
- **Policies.** `LeapSetPolicyFlags` lets you turn on background-app
  tracking, image streaming (the raw IR camera frames — great for
  fiducial / passthrough overlays), and HMD mode.
- **Images.** `eLeapEventType_Image` events give the two IR camera frames.
  You can blit them as a textured quad for a Blade-Runner-y passthrough
  view. The SDK ships an `ImageSample.c` example.
- **Multi-device.** `MultiDeviceSample.c` shows how to fuse two devices for
  wider coverage.

## File map

```
leap_motion/
├── README.md                       (this file)
├── leap_demo.py                    (Python + pygame visualiser)
├── run.sh                          (launcher for the Python visualiser)
├── .venv/                          (Python virtualenv, gitignored)
├── leap_demo_rs/                   (Rust + macroquad visualiser)
│   ├── Cargo.toml
│   ├── build.rs                    (finds LeapSDK, sets rpath)
│   ├── run.sh                      (cargo run --release wrapper)
│   ├── src/
│   │   ├── leap.rs                 (FFI bindings, packed structs)
│   │   └── main.rs                 (poll thread + render loop)
│   └── target/                     (cargo build output, gitignored)
└── laser_harp_rs/                  (Rust laser harp: synth + MIDI)
    ├── Cargo.toml
    ├── build.rs                    (same SDK lookup as the visualiser)
    ├── run.sh
    └── src/
        ├── leap.rs                 (copy of the FFI bindings)
        ├── synth.rs                (cpal polyphonic synth, 5 instruments)
        ├── midi.rs                 (virtual MIDI port via midir)
        └── main.rs                 (beam logic, render, key handling)
```

> `laser_harp_rs/src/leap.rs` is a verbatim copy of the file from
> `leap_demo_rs/`. The two demos stay independent for now; if a third app
> arrives, lift it into a shared `leap_ffi/` crate in a workspace.

## Troubleshooting

- **Empty window / no hands detected.** Open the Ultraleap Control Panel,
  confirm the device shows up. Wave a hand 10–40 cm above it.
- **Build can't find `libLeapC`.** Set `LEAP_SDK_DIR` to the directory
  containing the library (see "Build & run").
- **SIGSEGV when a hand enters frame.** This was a bug in earlier versions
  of this repo — the FFI structs were `#[repr(C)]` instead of
  `#[repr(C, packed)]`, so `LEAP_TRACKING_EVENT.pHands` was read from the
  wrong offset. Fixed in current `leap.rs`. If you see this again after
  modifying `leap.rs`, double-check your size assertions.
- **macroquad window flicker on first launch.** Known macOS quirk when the
  build runs in a non-TTY context; running from a normal Terminal /
  Warp / iTerm tab works fine.

## License notes

This project is yours to do whatever you like with. **The Ultraleap SDK is
not included** — it's covered by the Ultraleap Tracking SDK Agreement
shipped at `/Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK/LICENSE.md`.
The agreement permits use of `libLeapC` in your applications; redistribution
of the library itself has its own terms — read them if you plan to ship a
binary that bundles it.
