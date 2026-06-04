//! Polyphonic synth engine. Runs in the cpal audio callback thread; the main
//! thread sends events over a crossbeam channel.
//!
//! Instruments:
//!   - Pluck      Karplus-Strong, very harp-like
//!   - FM Bell    two-operator FM with decaying modulation index
//!   - Sine       pure tone with soft ADSR
//!   - Saw        bright, ADSR-shaped
//!   - Square     hollow, ADSR-shaped
//!
//! A global one-pole low-pass filter sits at the end of the mix; its cutoff is
//! driven by `SetBrightness` (0..1), which the harp logic maps from hand
//! height above the device. Lifting your hands opens the filter.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::f32::consts::TAU;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Instrument {
    Pluck,
    FmBell,
    Sine,
    Saw,
    Square,
}

impl Instrument {
    pub const ALL: [Instrument; 5] = [
        Instrument::Pluck,
        Instrument::FmBell,
        Instrument::Sine,
        Instrument::Saw,
        Instrument::Square,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Instrument::Pluck => "Pluck",
            Instrument::FmBell => "FM Bell",
            Instrument::Sine => "Sine",
            Instrument::Saw => "Saw",
            Instrument::Square => "Square",
        }
    }
}

#[derive(Debug)]
pub enum SynthEvent {
    NoteOn { id: u64, midi: u8, velocity: f32, instrument: Instrument },
    NoteOff { id: u64 },
    SetBrightness(f32), // 0..1
    SetSustain(bool),
    PanicOff,
}

const POLYPHONY: usize = 24;

#[derive(Copy, Clone, PartialEq, Eq)]
enum EnvState {
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

struct Voice {
    id: u64,            // 0 = free
    held: bool,         // released by NoteOff but kept alive if sustain pedal down
    midi: u8,
    freq: f32,
    velocity: f32,
    instrument: Instrument,
    env_state: EnvState,
    env_level: f32,
    phase: f32,
    phase2: f32,
    fm_env: f32,
    // Karplus-Strong delay line
    ks_delay: Vec<f32>,
    ks_idx: usize,
}

impl Voice {
    fn new() -> Self {
        Self {
            id: 0,
            held: false,
            midi: 0,
            freq: 0.0,
            velocity: 0.0,
            instrument: Instrument::Sine,
            env_state: EnvState::Off,
            env_level: 0.0,
            phase: 0.0,
            phase2: 0.0,
            fm_env: 0.0,
            ks_delay: Vec::new(),
            ks_idx: 0,
        }
    }

    fn start(&mut self, id: u64, midi: u8, velocity: f32, instrument: Instrument, sr: f32) {
        self.id = id;
        self.held = true;
        self.midi = midi;
        self.freq = midi_to_hz(midi);
        self.velocity = velocity.clamp(0.05, 1.0);
        self.instrument = instrument;
        self.phase = 0.0;
        self.phase2 = 0.0;
        self.fm_env = 1.0;
        self.env_level = 0.0;
        self.env_state = EnvState::Attack;
        if matches!(instrument, Instrument::Pluck) {
            let n = ((sr / self.freq).round() as usize).max(2);
            self.ks_delay.clear();
            self.ks_delay.resize(n, 0.0);
            // Fill with bandlimited noise scaled by velocity
            let mut seed = (midi as u32).wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
            for s in self.ks_delay.iter_mut() {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let r = ((seed >> 8) & 0xFFFF) as f32 / 32768.0 - 1.0;
                *s = r * self.velocity;
            }
            self.ks_idx = 0;
            // Pluck has its own decay; treat envelope as instant-on, hold
            self.env_level = 1.0;
            self.env_state = EnvState::Sustain;
        }
    }

    fn release(&mut self) {
        self.held = false;
        if matches!(self.instrument, Instrument::Pluck) {
            // Pluck rings out on its own; nothing to do
            return;
        }
        self.env_state = EnvState::Release;
    }

    fn is_active(&self) -> bool {
        self.env_state != EnvState::Off
    }

    fn tick(&mut self, sr: f32) -> f32 {
        if self.env_state == EnvState::Off {
            return 0.0;
        }

        // --- envelope ---
        let (attack_t, decay_t, sustain_l, release_t) = match self.instrument {
            Instrument::Pluck => (0.0, 0.0, 1.0, 0.0),
            Instrument::FmBell => (0.005, 1.2, 0.0, 0.4),
            Instrument::Sine => (0.012, 0.25, 0.6, 0.4),
            Instrument::Saw => (0.008, 0.20, 0.55, 0.3),
            Instrument::Square => (0.008, 0.20, 0.55, 0.3),
        };
        match self.env_state {
            EnvState::Attack => {
                let step = if attack_t > 0.0 { 1.0 / (attack_t * sr) } else { 1.0 };
                self.env_level += step;
                if self.env_level >= 1.0 {
                    self.env_level = 1.0;
                    self.env_state = EnvState::Decay;
                }
            }
            EnvState::Decay => {
                let step = if decay_t > 0.0 {
                    (1.0 - sustain_l) / (decay_t * sr)
                } else {
                    1.0
                };
                self.env_level -= step;
                if self.env_level <= sustain_l {
                    self.env_level = sustain_l;
                    self.env_state = EnvState::Sustain;
                }
            }
            EnvState::Sustain => {}
            EnvState::Release => {
                let step = if release_t > 0.0 {
                    self.env_level / (release_t * sr)
                } else {
                    1.0
                };
                self.env_level -= step;
                if self.env_level <= 0.0001 {
                    self.env_level = 0.0;
                    self.env_state = EnvState::Off;
                }
            }
            EnvState::Off => {}
        }

        // --- oscillator ---
        let inc = self.freq / sr;
        let sample = match self.instrument {
            Instrument::Sine => {
                let s = (self.phase * TAU).sin();
                self.phase = (self.phase + inc).fract();
                s
            }
            Instrument::Saw => {
                // simple ramp -1..1; not bandlimited, but cheap and bright
                let s = 2.0 * self.phase - 1.0;
                self.phase = (self.phase + inc).fract();
                s
            }
            Instrument::Square => {
                let s = if self.phase < 0.5 { 1.0 } else { -1.0 };
                self.phase = (self.phase + inc).fract();
                s
            }
            Instrument::FmBell => {
                // Modulator: 3.5x carrier freq. Index decays exponentially.
                let mod_freq = self.freq * 3.5;
                let modulator = (self.phase2 * TAU).sin();
                self.phase2 = (self.phase2 + mod_freq / sr).fract();
                self.fm_env *= (-1.0 / (0.8 * sr)).exp(); // ~0.8s mod decay
                let index = 4.0 * self.fm_env;
                let p = self.phase * TAU + index * modulator;
                let s = p.sin();
                self.phase = (self.phase + inc).fract();
                s
            }
            Instrument::Pluck => {
                let n = self.ks_delay.len();
                if n == 0 {
                    0.0
                } else {
                    let i = self.ks_idx;
                    let j = (i + 1) % n;
                    let avg = (self.ks_delay[i] + self.ks_delay[j]) * 0.5 * 0.9965;
                    let out = self.ks_delay[i];
                    self.ks_delay[i] = avg;
                    self.ks_idx = j;
                    // Decide when to retire the voice
                    if !self.held && out.abs() < 1e-4 && avg.abs() < 1e-4 {
                        // let it ring; only retire after a long quiet period
                    }
                    if !self.held {
                        // accelerated decay after note-off
                        self.ks_delay[i] *= 0.999;
                    }
                    out
                }
            }
        };

        let amp = self.env_level * self.velocity;
        let out = sample * amp;

        // Pluck has no ADSR-based death; retire when energy is tiny
        if matches!(self.instrument, Instrument::Pluck) {
            let energy: f32 = self
                .ks_delay
                .iter()
                .take(32)
                .map(|s| s.abs())
                .sum::<f32>()
                / 32.0;
            if energy < 1e-4 {
                self.env_state = EnvState::Off;
            }
        }

        out
    }
}

pub fn midi_to_hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0)
}

pub struct SynthHandle {
    pub tx: Sender<SynthEvent>,
    pub sample_rate: f32,
    _stream: cpal::Stream,
}

struct OnePole {
    y: f32,
    alpha: f32,
}
impl OnePole {
    fn new() -> Self { Self { y: 0.0, alpha: 1.0 } }
    fn set_cutoff(&mut self, fc_hz: f32, sr: f32) {
        let x = 1.0 - (-TAU * fc_hz / sr).exp();
        self.alpha = x.clamp(0.0001, 1.0);
    }
    fn tick(&mut self, x: f32) -> f32 {
        self.y += self.alpha * (x - self.y);
        self.y
    }
}

pub fn start() -> Result<SynthHandle, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default audio output device".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("default_output_config: {e}"))?;
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let stream_config: cpal::StreamConfig = config.clone().into();

    let (tx, rx) = unbounded::<SynthEvent>();

    let mut voices: Vec<Voice> = (0..POLYPHONY).map(|_| Voice::new()).collect();
    let mut lp = OnePole::new();
    lp.set_cutoff(4000.0, sample_rate);
    let mut sustain = false;

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    process_callback(
                        data,
                        channels,
                        sample_rate,
                        &rx,
                        &mut voices,
                        &mut lp,
                        &mut sustain,
                    );
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_output_stream: {e}"))?,
        other => return Err(format!("unsupported sample format: {other:?}")),
    };

    stream
        .play()
        .map_err(|e| format!("stream.play: {e}"))?;

    Ok(SynthHandle {
        tx,
        sample_rate,
        _stream: stream,
    })
}

fn process_callback(
    data: &mut [f32],
    channels: usize,
    sr: f32,
    rx: &Receiver<SynthEvent>,
    voices: &mut [Voice],
    lp: &mut OnePole,
    sustain: &mut bool,
) {
    // Drain events
    while let Ok(ev) = rx.try_recv() {
        match ev {
            SynthEvent::NoteOn { id, midi, velocity, instrument } => {
                let slot = pick_voice(voices);
                voices[slot].start(id, midi, velocity, instrument, sr);
            }
            SynthEvent::NoteOff { id } => {
                for v in voices.iter_mut() {
                    if v.id == id && v.held {
                        if *sustain {
                            v.held = false; // still active, marked released-pending
                        } else {
                            v.release();
                        }
                    }
                }
            }
            SynthEvent::SetBrightness(b) => {
                // 0 -> 350 Hz, 1 -> 9 kHz, log mapping
                let b = b.clamp(0.0, 1.0);
                let fc = 350.0 * (9000f32 / 350f32).powf(b);
                lp.set_cutoff(fc, sr);
            }
            SynthEvent::SetSustain(on) => {
                if *sustain && !on {
                    // release any voices that were waiting on the pedal
                    for v in voices.iter_mut() {
                        if !v.held && v.is_active() && !matches!(v.instrument, Instrument::Pluck)
                        {
                            v.release();
                        }
                    }
                }
                *sustain = on;
            }
            SynthEvent::PanicOff => {
                for v in voices.iter_mut() {
                    v.env_state = EnvState::Off;
                    v.env_level = 0.0;
                    v.id = 0;
                    v.held = false;
                    v.ks_delay.clear();
                }
            }
        }
    }

    for frame in data.chunks_mut(channels) {
        let mut mix = 0.0;
        for v in voices.iter_mut() {
            if v.is_active() {
                mix += v.tick(sr);
            }
        }
        // Soft clip to avoid hard distortion on dense chords
        let mix = (mix * 0.18).tanh();
        let filtered = lp.tick(mix);
        for ch in frame.iter_mut() {
            *ch = filtered;
        }
    }
}

fn pick_voice(voices: &mut [Voice]) -> usize {
    // Prefer a free slot; otherwise steal the quietest one
    if let Some(i) = voices.iter().position(|v| !v.is_active()) {
        return i;
    }
    voices
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.env_level
                .partial_cmp(&b.env_level)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}
