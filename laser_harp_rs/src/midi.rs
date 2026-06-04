//! MIDI output. Creates a virtual CoreMIDI / ALSA / WinMM port that DAWs and
//! other apps can pick up. If the virtual port can't be created (e.g. WinMM
//! doesn't support virtual ports), falls back to the first available output.

use midir::{MidiOutput, MidiOutputConnection};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use midir::os::unix::VirtualOutput;

const PORT_NAME: &str = "Leap Laser Harp";

pub struct MidiOut {
    conn: Option<MidiOutputConnection>,
    pub label: String,
}

impl MidiOut {
    /// A MIDI sink that swallows everything. Used in screenshot mode so we
    /// don't open a real virtual port just to save a PNG.
    pub fn disabled() -> Self {
        Self {
            conn: None,
            label: "disabled".to_string(),
        }
    }

    pub fn open() -> Self {
        match try_open() {
            Ok((conn, label)) => Self {
                conn: Some(conn),
                label,
            },
            Err(e) => {
                eprintln!("midi: {e} — running without MIDI out");
                Self {
                    conn: None,
                    label: format!("disabled ({e})"),
                }
            }
        }
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(c) = self.conn.as_mut() {
            let status = 0x90 | (channel & 0x0F);
            let _ = c.send(&[status, note & 0x7F, velocity & 0x7F]);
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        if let Some(c) = self.conn.as_mut() {
            let status = 0x80 | (channel & 0x0F);
            let _ = c.send(&[status, note & 0x7F, 0]);
        }
    }

    pub fn cc(&mut self, channel: u8, controller: u8, value: u8) {
        if let Some(c) = self.conn.as_mut() {
            let status = 0xB0 | (channel & 0x0F);
            let _ = c.send(&[status, controller & 0x7F, value & 0x7F]);
        }
    }

    pub fn all_notes_off(&mut self) {
        for ch in 0..16u8 {
            self.cc(ch, 123, 0);
        }
    }
}

fn try_open() -> Result<(MidiOutputConnection, String), String> {
    let out = MidiOutput::new("Leap Laser Harp client")
        .map_err(|e| format!("init MidiOutput: {e}"))?;

    // Try a virtual port first (macOS / Linux). Windows doesn't support this.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        match out.create_virtual(PORT_NAME) {
            Ok(conn) => return Ok((conn, format!("virtual port \"{PORT_NAME}\""))),
            Err(e) => eprintln!("midi: create_virtual failed ({e}), falling back"),
        }
    }

    // Fallback: connect to the first available output port.
    let out = MidiOutput::new("Leap Laser Harp client")
        .map_err(|e| format!("init MidiOutput (retry): {e}"))?;
    let ports = out.ports();
    let port = ports
        .into_iter()
        .next()
        .ok_or_else(|| "no MIDI output ports available".to_string())?;
    let name = out.port_name(&port).unwrap_or_else(|_| "unknown".into());
    let conn = out
        .connect(&port, "harp")
        .map_err(|e| format!("connect: {e}"))?;
    Ok((conn, name))
}
