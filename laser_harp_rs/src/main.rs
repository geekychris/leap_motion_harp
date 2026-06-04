#![allow(non_upper_case_globals)]

//! Leap Motion 2D laser harp.
//!
//! Five horizontal "strings" are stacked at increasing heights above the
//! device, tuned to a C-major pentatonic scale. To play a note you sweep a
//! fingertip vertically through a string. The note's octave is determined by
//! your hand's horizontal position: five octave zones across the play area
//! cover ±2 octaves around middle C.
//!
//! Each of the five fingers triggers independently, so spreading your
//! fingers and strumming produces chords / arpeggios. Curl a finger and it
//! goes quiet — only **extended** fingers can pluck strings (we use the
//! Leap SDK's `is_extended` flag). This makes the hand work a little like
//! fretting on a guitar: choose which fingers will speak before you strum.
//!
//! Lifting the palm opens a global low-pass filter (brightness). Pinching
//! either hand engages a sustain pedal.
//!
//! Audio: an in-process polyphonic synth (`synth.rs`) via cpal.
//! MIDI:  a virtual port (`midi.rs`) the rest of the machine can subscribe to.
//!
//! Keys:
//!   1..5         pick the RIGHT-hand instrument
//!   Shift+1..5   pick the LEFT-hand instrument
//!   H            swap left/right chirality (depends on device orientation)
//!   [ / ]        transpose down / up by a semitone
//!   space        panic (all notes off)
//!   esc / q      quit

mod leap;
mod midi;
mod synth;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use macroquad::prelude::*;

use leap::*;
use midi::MidiOut;
use synth::{Instrument, SynthEvent};

/// Read a field from a `#[repr(C, packed)]` FFI struct safely.
macro_rules! get {
    ($ptr:expr, $($field:tt)*) => {{
        ::std::ptr::read_unaligned(::std::ptr::addr_of!((*$ptr) $($field)*))
    }};
}

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 800;
const PROJ_SCALE: f32 = 2.2;
const CENTER_Y_MM: f32 = 280.0;

// --- Strings (Y axis = pitch class) -----------------------------------------
const N_STRINGS: usize = 5;
/// C-major pentatonic, MIDI note numbers at octave 0 (middle C).
/// Lowest string sits lowest in the play volume.
const SCALE: [u8; N_STRINGS] = [60, 62, 64, 67, 69]; // C4 D4 E4 G4 A4
const STRING_Y_MM: [f32; N_STRINGS] = [120.0, 200.0, 280.0, 360.0, 440.0];
const STRING_HALF_HEIGHT_MM: f32 = 18.0;

// --- Octaves (X axis) -------------------------------------------------------
const N_OCTAVES: usize = 5;
/// Half-width of one octave zone, mm. Zones are centred at -2x..+2x times
/// this value, with smooth transitions so finger X picks the nearest centre.
const OCT_ZONE_HALF_MM: f32 = 50.0;
const OCT_OFFSETS: [i32; N_OCTAVES] = [-2, -1, 0, 1, 2];

const PINCH_SUSTAIN_THRESHOLD: f32 = 0.7;

const BG: Color = color_u8!(8, 8, 18, 255);

#[derive(Clone, Default)]
struct FingerSnap {
    tip: [f32; 3],
    is_extended: bool,
}

#[derive(Clone, Default)]
struct HandSnap {
    id: u32,
    is_left: bool,
    palm: [f32; 3],
    pinch: f32,
    fingers: [FingerSnap; 5],
}

#[derive(Clone)]
struct SharedState {
    hands: Vec<HandSnap>,
    framerate: f32,
    connected: bool,
    string_pulse: [f32; N_STRINGS], // 0..1 per string, decays in render loop
    active_per_string: [u8; N_STRINGS],
    sustain: bool,
    transpose: i32,
    instrument_left: Instrument,
    instrument_right: Instrument,
    /// XOR-flip the SDK's chirality. Useful when the device is mounted
    /// upside-down or the SDK reports your hands the wrong way around.
    swap_hands: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            hands: Vec::new(),
            framerate: 0.0,
            connected: false,
            string_pulse: [0.0; N_STRINGS],
            active_per_string: [0; N_STRINGS],
            sustain: false,
            transpose: 0,
            instrument_left: Instrument::FmBell,
            instrument_right: Instrument::Pluck,
            swap_hands: true,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct FingerStringState {
    inside: [bool; N_STRINGS],
    last_note_id: [u64; N_STRINGS],
    last_midi: [u8; N_STRINGS],    // octave captured at strum time, used for matching note-off
    last_channel: [u8; N_STRINGS], // MIDI channel the strum was sent on
}

/// Quantise a finger X (mm) to one of the N_OCTAVES octave offsets.
fn octave_from_x(x_mm: f32) -> i32 {
    let idx = ((x_mm / (OCT_ZONE_HALF_MM * 2.0)).round() as i32 + (N_OCTAVES as i32 / 2))
        .clamp(0, N_OCTAVES as i32 - 1) as usize;
    OCT_OFFSETS[idx]
}

fn octave_index_from_x(x_mm: f32) -> usize {
    ((x_mm / (OCT_ZONE_HALF_MM * 2.0)).round() as i32 + (N_OCTAVES as i32 / 2))
        .clamp(0, N_OCTAVES as i32 - 1) as usize
}

fn project(p: [f32; 3]) -> (f32, f32) {
    let w = screen_width();
    let h = screen_height();
    let sx = w / 2.0 + p[0] * PROJ_SCALE;
    let sy = h / 2.0 - (p[1] - CENTER_Y_MM) * PROJ_SCALE;
    (sx, sy)
}

fn poll_thread(
    state: Arc<Mutex<SharedState>>,
    synth_tx: crossbeam_channel::Sender<SynthEvent>,
    midi: Arc<Mutex<MidiOut>>,
) {
    unsafe {
        let mut conn: LEAP_CONNECTION = std::ptr::null_mut();
        if LeapCreateConnection(std::ptr::null(), &mut conn) != eLeapRS_Success {
            eprintln!("LeapCreateConnection failed");
            return;
        }
        if LeapOpenConnection(conn) != eLeapRS_Success {
            eprintln!("LeapOpenConnection failed");
            LeapDestroyConnection(conn);
            return;
        }

        let mut msg = std::mem::zeroed::<LEAP_CONNECTION_MESSAGE>();
        let mut finger_states: HashMap<(u32, u8), FingerStringState> = HashMap::new();
        let mut next_note_id: u64 = 1;
        let mut last_sustain = false;

        loop {
            if LeapPollConnection(conn, 200, &mut msg) != eLeapRS_Success {
                continue;
            }
            let msg_type: u32 = get!(&msg, .msg_type);
            match msg_type {
                eLeapEventType_Connection => state.lock().unwrap().connected = true,
                eLeapEventType_ConnectionLost => state.lock().unwrap().connected = false,
                eLeapEventType_Tracking => {
                    let event_ptr: *const c_void = get!(&msg, .event_ptr);
                    let ev = event_ptr as *const LEAP_TRACKING_EVENT;
                    if ev.is_null() {
                        continue;
                    }
                    let n: u32 = get!(ev, .nHands);
                    let p_hands: *const LEAP_HAND = get!(ev, .pHands);
                    let framerate: f32 = get!(ev, .framerate);

                    let (transpose, inst_left, inst_right, swap_hands) = {
                        let s = state.lock().unwrap();
                        (s.transpose, s.instrument_left, s.instrument_right, s.swap_hands)
                    };

                    // Snapshot hands
                    let mut hands = Vec::with_capacity(n as usize);
                    for i in 0..n as usize {
                        let h = p_hands.add(i);
                        let mut fingers: [FingerSnap; 5] = Default::default();
                        for f in 0..5 {
                            let is_ext: u32 = get!(h, .digits[f].is_extended);
                            fingers[f] = FingerSnap {
                                tip: get!(h, .digits[f].bones[3].next_joint.v),
                                is_extended: is_ext != 0,
                            };
                        }
                        let raw_is_left = {
                            let t: u32 = get!(h, .hand_type);
                            t == eLeapHandType_Left
                        };
                        hands.push(HandSnap {
                            id: get!(h, .id),
                            is_left: raw_is_left ^ swap_hands,
                            palm: get!(h, .palm.position.v),
                            pinch: get!(h, .pinch_strength),
                            fingers,
                        });
                    }

                    // Sustain pedal
                    let sustain_now =
                        hands.iter().any(|h| h.pinch > PINCH_SUSTAIN_THRESHOLD);
                    if sustain_now != last_sustain {
                        let _ = synth_tx.send(SynthEvent::SetSustain(sustain_now));
                        last_sustain = sustain_now;
                    }

                    // Brightness ← average palm height
                    if !hands.is_empty() {
                        let y_avg = hands.iter().map(|h| h.palm[1]).sum::<f32>()
                            / hands.len() as f32;
                        let b = ((y_avg - 100.0) / 400.0).clamp(0.0, 1.0);
                        let _ = synth_tx.send(SynthEvent::SetBrightness(b));
                    }

                    let mut pulse_delta = [0.0f32; N_STRINGS];
                    let mut active_delta: [i32; N_STRINGS] = [0; N_STRINGS];

                    // Forget state for hands no longer visible (and release any held notes)
                    let live_hands: std::collections::HashSet<u32> =
                        hands.iter().map(|h| h.id).collect();
                    finger_states.retain(|(hid, _), st| {
                        let alive = live_hands.contains(hid);
                        if !alive {
                            for s in 0..N_STRINGS {
                                if st.last_note_id[s] != 0 {
                                    let _ = synth_tx.send(SynthEvent::NoteOff {
                                        id: st.last_note_id[s],
                                    });
                                    let mut m = midi.lock().unwrap();
                                    m.note_off(st.last_channel[s], st.last_midi[s]);
                                    active_delta[s] -= 1;
                                    st.last_note_id[s] = 0;
                                    st.inside[s] = false;
                                }
                            }
                        }
                        alive
                    });

                    for hand in &hands {
                        for fi in 0..5u8 {
                            let finger = &hand.fingers[fi as usize];
                            let tip = finger.tip;
                            let state_ref =
                                finger_states.entry((hand.id, fi)).or_default();

                            for s in 0..N_STRINGS {
                                let dy = (tip[1] - STRING_Y_MM[s]).abs();
                                // Trigger only if finger is extended AND within Y band
                                let inside_now =
                                    finger.is_extended && dy < STRING_HALF_HEIGHT_MM;
                                let was_inside = state_ref.inside[s];

                                if inside_now && !was_inside {
                                    // Note-on: pick octave from finger X at moment of crossing
                                    let octave = octave_from_x(tip[0]);
                                    let midi_note = (SCALE[s] as i32
                                        + 12 * octave
                                        + transpose)
                                        .clamp(0, 127)
                                        as u8;
                                    let id = next_note_id;
                                    next_note_id = next_note_id.wrapping_add(1).max(1);
                                    // Velocity grows with finger speed through the band
                                    // (approximated by pinch + a baseline)
                                    let velocity =
                                        (0.55 + 0.45 * hand.pinch).clamp(0.2, 1.0);
                                    let instrument = if hand.is_left {
                                        inst_left
                                    } else {
                                        inst_right
                                    };
                                    let channel: u8 = if hand.is_left { 1 } else { 0 };
                                    let _ = synth_tx.send(SynthEvent::NoteOn {
                                        id,
                                        midi: midi_note,
                                        velocity,
                                        instrument,
                                    });
                                    midi.lock().unwrap().note_on(
                                        channel,
                                        midi_note,
                                        (velocity * 127.0) as u8,
                                    );
                                    state_ref.last_note_id[s] = id;
                                    state_ref.last_midi[s] = midi_note;
                                    state_ref.last_channel[s] = channel;
                                    pulse_delta[s] = pulse_delta[s].max(1.0);
                                    active_delta[s] += 1;
                                } else if !inside_now && was_inside {
                                    // Note-off
                                    let id = state_ref.last_note_id[s];
                                    if id != 0 {
                                        let _ =
                                            synth_tx.send(SynthEvent::NoteOff { id });
                                        midi.lock().unwrap().note_off(
                                            state_ref.last_channel[s],
                                            state_ref.last_midi[s],
                                        );
                                        state_ref.last_note_id[s] = 0;
                                        active_delta[s] -= 1;
                                    }
                                }
                                state_ref.inside[s] = inside_now;
                            }
                        }
                    }

                    let mut s = state.lock().unwrap();
                    s.hands = hands;
                    s.framerate = framerate;
                    s.sustain = sustain_now;
                    for i in 0..N_STRINGS {
                        if pulse_delta[i] > s.string_pulse[i] {
                            s.string_pulse[i] = pulse_delta[i];
                        }
                        let new_active = s.active_per_string[i] as i32 + active_delta[i];
                        s.active_per_string[i] = new_active.max(0) as u8;
                    }
                }
                _ => {}
            }
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::new(r + m, g + m, b + m, 1.0)
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

fn note_name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let name = NAMES[(midi as usize) % 12];
    let oct = midi as i32 / 12 - 1;
    format!("{name}{oct}")
}

fn draw_octave_columns(highlighted: &[bool; N_OCTAVES]) {
    let h = screen_height();
    for (i, &oct) in OCT_OFFSETS.iter().enumerate() {
        let centre_x_mm = oct as f32 * OCT_ZONE_HALF_MM * 2.0;
        let (left_x, _) = project([centre_x_mm - OCT_ZONE_HALF_MM, 0.0, 0.0]);
        let (right_x, _) = project([centre_x_mm + OCT_ZONE_HALF_MM, 0.0, 0.0]);
        let alpha = if highlighted[i] { 0.18 } else { 0.05 };
        draw_rectangle(left_x, 0.0, right_x - left_x, h, color_u8!(80, 100, 180, (alpha * 255.0) as u8));
        // Label
        let (cx, _) = project([centre_x_mm, 0.0, 0.0]);
        let label = if oct == 0 { "0".to_string() } else { format!("{oct:+}") };
        let color = if highlighted[i] {
            color_u8!(180, 200, 255, 255)
        } else {
            color_u8!(70, 80, 110, 255)
        };
        draw_text(&label, cx - 6.0, 26.0, 22.0, color);
        draw_text("oct", cx - 14.0, 44.0, 12.0, color);
    }
}

fn draw_string(i: usize, color: Color, pulse: f32, active: bool) {
    let y_mm = STRING_Y_MM[i];
    let (_, sy) = project([0.0, y_mm, 0.0]);
    let w = screen_width();

    for (thick, alpha_mul) in [(14.0f32, 0.06), (8.0, 0.12), (4.0, 0.22)] {
        let a = (0.25 + 0.75 * pulse) * alpha_mul;
        draw_line(0.0, sy, w, sy, thick, with_alpha(color, a));
    }
    let core_alpha = 0.55 + 0.45 * pulse;
    let core_color = if active { WHITE } else { color };
    draw_line(0.0, sy, w, sy, 2.0, with_alpha(core_color, core_alpha));

    // Note label on the left
    let label = note_name(SCALE[i]);
    draw_text(&label, 10.0, sy + 5.0, 20.0, with_alpha(color, 0.95));
}

fn draw_hand(h: &HandSnap, base_color: Color) {
    let (px, py) = project(h.palm);
    draw_circle_lines(px, py, 14.0, 2.0, with_alpha(base_color, 0.6));
    if h.pinch > PINCH_SUSTAIN_THRESHOLD {
        draw_circle(px, py, 14.0, with_alpha(base_color, 0.3 + 0.5 * h.pinch));
    }
    for finger in &h.fingers {
        let (x, y) = project(finger.tip);
        if finger.is_extended {
            // Bright filled dot + faint halo when extended
            draw_circle(x, y, 8.0, base_color);
            draw_circle_lines(x, y, 14.0, 1.0, with_alpha(base_color, 0.35));
        } else {
            // Dimmed hollow ring when curled (muted)
            draw_circle_lines(x, y, 6.0, 1.5, with_alpha(base_color, 0.5));
        }
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Leap Laser Harp — 2D".to_owned(),
        window_width: WIDTH,
        window_height: HEIGHT,
        high_dpi: true,
        window_resizable: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let synth_handle = match synth::start() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("audio: {e} — running silent");
            None
        }
    };
    let synth_tx = synth_handle.as_ref().map(|h| h.tx.clone());

    let midi = Arc::new(Mutex::new(MidiOut::open()));
    let midi_label = midi.lock().unwrap().label.clone();

    let state = Arc::new(Mutex::new(SharedState::default()));
    let (poll_tx, _poll_rx_unused) = crossbeam_channel::unbounded();
    let tx_for_poll = synth_tx.clone().unwrap_or(poll_tx);
    {
        let s = state.clone();
        let m = midi.clone();
        thread::Builder::new()
            .name("leap-poll".into())
            .spawn(move || poll_thread(s, tx_for_poll, m))
            .expect("spawn leap poll thread");
    }

    let string_colors: [Color; N_STRINGS] =
        std::array::from_fn(|i| hsv(i as f32 / N_STRINGS as f32 * 0.78, 0.85, 1.0));

    let mut last_frame_time = Instant::now();

    loop {
        if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Q) {
            break;
        }
        let shift_down =
            is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        for (i, key) in [
            KeyCode::Key1,
            KeyCode::Key2,
            KeyCode::Key3,
            KeyCode::Key4,
            KeyCode::Key5,
        ]
        .iter()
        .enumerate()
        {
            if is_key_pressed(*key) && i < Instrument::ALL.len() {
                let mut s = state.lock().unwrap();
                if shift_down {
                    s.instrument_left = Instrument::ALL[i];
                } else {
                    s.instrument_right = Instrument::ALL[i];
                }
            }
        }
        if is_key_pressed(KeyCode::Space) {
            if let Some(tx) = &synth_tx {
                let _ = tx.send(SynthEvent::PanicOff);
            }
            midi.lock().unwrap().all_notes_off();
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            state.lock().unwrap().transpose -= 1;
        }
        if is_key_pressed(KeyCode::RightBracket) {
            state.lock().unwrap().transpose += 1;
        }
        if is_key_pressed(KeyCode::H) {
            // Swap chirality and silence any notes that were keyed off the old assignment
            let mut s = state.lock().unwrap();
            s.swap_hands = !s.swap_hands;
            drop(s);
            if let Some(tx) = &synth_tx {
                let _ = tx.send(SynthEvent::PanicOff);
            }
            midi.lock().unwrap().all_notes_off();
        }

        let now = Instant::now();
        let dt = (now - last_frame_time).as_secs_f32().min(0.1);
        last_frame_time = now;

        let snap: SharedState = {
            let mut s = state.lock().unwrap();
            for p in s.string_pulse.iter_mut() {
                *p = (*p - dt * 2.6).max(0.0);
            }
            s.clone()
        };

        clear_background(BG);

        // Determine which octave column each hand is currently in
        let mut hi_octaves = [false; N_OCTAVES];
        for h in &snap.hands {
            // Take the average X across extended fingertips when available;
            // otherwise the palm
            let mut sum = 0.0;
            let mut count = 0;
            for f in &h.fingers {
                if f.is_extended {
                    sum += f.tip[0];
                    count += 1;
                }
            }
            let x = if count > 0 { sum / count as f32 } else { h.palm[0] };
            hi_octaves[octave_index_from_x(x)] = true;
        }
        draw_octave_columns(&hi_octaves);

        // Strings
        for i in 0..N_STRINGS {
            draw_string(
                i,
                string_colors[i],
                snap.string_pulse[i],
                snap.active_per_string[i] > 0,
            );
        }

        // Hands
        for h in &snap.hands {
            let c = if h.is_left {
                color_u8!(120, 200, 255, 255)
            } else {
                color_u8!(255, 180, 100, 255)
            };
            draw_hand(h, c);
        }

        // HUD
        let txt_c = color_u8!(220, 225, 240, 255);
        let dim = color_u8!(140, 150, 180, 255);
        draw_text("Leap Laser Harp — 2D", 24.0, 80.0, 28.0, txt_c);
        let left_c = color_u8!(120, 200, 255, 255);
        let right_c = color_u8!(255, 180, 100, 255);
        draw_text("LEFT:", 24.0, 106.0, 18.0, left_c);
        draw_text(snap.instrument_left.name(), 84.0, 106.0, 18.0, left_c);
        draw_text("RIGHT:", 230.0, 106.0, 18.0, right_c);
        draw_text(snap.instrument_right.name(), 302.0, 106.0, 18.0, right_c);
        draw_text(
            "1..5 set RIGHT  ·  Shift+1..5 set LEFT  ·  (Pluck / FM Bell / Sine / Saw / Square)",
            24.0,
            128.0,
            14.0,
            dim,
        );
        let conn_label = if snap.connected { "connected" } else { "waiting..." };
        draw_text(
            &format!(
                "leap: {conn_label} @ {:0.0} fps    midi: {}    transpose: {:+} semitones",
                snap.framerate, midi_label, snap.transpose
            ),
            24.0,
            150.0,
            14.0,
            dim,
        );
        draw_text(
            "horizontal strings = pitch (pentatonic).  Sweep a fingertip THROUGH a string to pluck.",
            24.0,
            screen_height() - 64.0,
            14.0,
            dim,
        );
        draw_text(
            "Hand X = OCTAVE (top row 0/±1/±2).  Hand Y / palm height = brightness.  Pinch = sustain.",
            24.0,
            screen_height() - 46.0,
            14.0,
            dim,
        );
        draw_text(
            "Only EXTENDED fingers play — curl a finger to mute it.   H swap L/R   [ / ] transpose   space panic   esc quit",
            24.0,
            screen_height() - 26.0,
            14.0,
            dim,
        );
        if snap.sustain {
            draw_text(
                "SUSTAIN",
                screen_width() - 120.0,
                40.0,
                22.0,
                color_u8!(255, 200, 80, 255),
            );
        }

        next_frame().await;
    }

    if let Some(tx) = &synth_tx {
        let _ = tx.send(SynthEvent::PanicOff);
    }
    midi.lock().unwrap().all_notes_off();
}
