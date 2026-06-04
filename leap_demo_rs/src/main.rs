#![allow(non_upper_case_globals)]

//! Visual Leap Motion demo using macroquad + raw libLeapC FFI.
//!
//! A background thread polls LeapPollConnection at the device's native rate
//! and publishes a copy of the latest hand frame into a Mutex. The render
//! thread reads the snapshot every frame and draws skeletons, finger trails,
//! pinch/grab meters, and a depth-cued shadow.

mod leap;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use macroquad::prelude::*;

use leap::*;

/// Read a field from a (potentially packed/misaligned) C struct via a raw
/// pointer. Required for our `#[repr(C, packed)]` LeapC bindings — taking a
/// reference to a packed field is UB, so we must go through `addr_of!` and
/// `read_unaligned`.
macro_rules! get {
    ($ptr:expr, $($field:tt)*) => {{
        ::std::ptr::read_unaligned(::std::ptr::addr_of!((*$ptr) $($field)*))
    }};
}

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 800;
const PROJ_SCALE: f32 = 2.2; // pixels per millimeter
const CENTER_Y_MM: f32 = 250.0;

const BG: Color = color_u8!(10, 12, 22, 255);
const GRID: Color = color_u8!(28, 32, 50, 255);
const AXIS: Color = color_u8!(50, 55, 80, 255);
const TEXT: Color = color_u8!(220, 225, 240, 255);
const LEFT_C: Color = color_u8!(90, 200, 255, 255);
const RIGHT_C: Color = color_u8!(255, 170, 80, 255);

#[derive(Clone, Default)]
struct HandSnap {
    id: u32,
    is_left: bool,
    palm: [f32; 3],
    pinch: f32,
    grab: f32,
    arm_prev: [f32; 3],
    arm_next: [f32; 3],
    bones: [[([f32; 3], [f32; 3]); 4]; 5], // digits[5] x bones[4] x (prev, next)
}

#[derive(Clone, Default)]
struct FrameSnap {
    hands: Vec<HandSnap>,
    frame_id: i64,
    framerate: f32,
    connected: bool,
}

fn poll_thread(state: Arc<Mutex<FrameSnap>>) {
    unsafe {
        let mut conn: LEAP_CONNECTION = std::ptr::null_mut();
        let rc = LeapCreateConnection(std::ptr::null(), &mut conn);
        if rc != eLeapRS_Success {
            eprintln!("LeapCreateConnection failed: 0x{rc:08x}");
            return;
        }
        let rc = LeapOpenConnection(conn);
        if rc != eLeapRS_Success {
            eprintln!("LeapOpenConnection failed: 0x{rc:08x}");
            LeapDestroyConnection(conn);
            return;
        }

        let mut msg = std::mem::zeroed::<LEAP_CONNECTION_MESSAGE>();
        loop {
            let rc = LeapPollConnection(conn, 200, &mut msg);
            if rc != eLeapRS_Success {
                continue;
            }
            let msg_type: u32 = get!(&msg, .msg_type);
            match msg_type {
                eLeapEventType_Connection => {
                    state.lock().unwrap().connected = true;
                }
                eLeapEventType_ConnectionLost => {
                    state.lock().unwrap().connected = false;
                }
                eLeapEventType_Tracking => {
                    let event_ptr: *const c_void = get!(&msg, .event_ptr);
                    let ev = event_ptr as *const LEAP_TRACKING_EVENT;
                    if ev.is_null() {
                        continue;
                    }
                    let n: u32 = get!(ev, .nHands);
                    let p_hands: *const LEAP_HAND = get!(ev, .pHands);
                    let frame_id: i64 = get!(ev, .tracking_frame_id);
                    let framerate: f32 = get!(ev, .framerate);

                    let mut hands = Vec::with_capacity(n as usize);
                    for i in 0..n as usize {
                        let h = p_hands.add(i);
                        let mut bones = [[([0.0f32; 3], [0.0f32; 3]); 4]; 5];
                        for d in 0..5 {
                            for b in 0..4 {
                                bones[d][b] = (
                                    get!(h, .digits[d].bones[b].prev_joint.v),
                                    get!(h, .digits[d].bones[b].next_joint.v),
                                );
                            }
                        }
                        hands.push(HandSnap {
                            id: get!(h, .id),
                            is_left: { let t: u32 = get!(h, .hand_type); t == eLeapHandType_Left },
                            palm: get!(h, .palm.position.v),
                            pinch: get!(h, .pinch_strength),
                            grab: get!(h, .grab_strength),
                            arm_prev: get!(h, .arm.prev_joint.v),
                            arm_next: get!(h, .arm.next_joint.v),
                            bones,
                        });
                    }
                    let mut s = state.lock().unwrap();
                    s.hands = hands;
                    s.frame_id = frame_id;
                    s.framerate = framerate;
                }
                _ => {}
            }
        }
    }
}

fn project(p: [f32; 3]) -> (f32, f32) {
    let w = screen_width();
    let h = screen_height();
    let sx = w / 2.0 + p[0] * PROJ_SCALE;
    let sy = h / 2.0 - (p[1] - CENTER_Y_MM) * PROJ_SCALE;
    (sx, sy)
}

fn depth_factor(z: f32) -> f32 {
    (1.0 + (-z) / 400.0).clamp(0.45, 1.6)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

fn draw_grid_bg() {
    let w = screen_width();
    let h = screen_height();
    let step = 80.0;
    let mut x = 0.0;
    while x < w {
        draw_line(x, 0.0, x, h, 1.0, GRID);
        x += step;
    }
    let mut y = 0.0;
    while y < h {
        draw_line(0.0, y, w, y, 1.0, GRID);
        y += step;
    }
    draw_line(0.0, h / 2.0, w, h / 2.0, 1.0, AXIS);
    draw_line(w / 2.0, 0.0, w / 2.0, h, 1.0, AXIS);
}

fn draw_hand(h: &HandSnap, white: Color, red: Color) {
    let base = if h.is_left { LEFT_C } else { RIGHT_C };
    let grab_tint = mix(base, red, h.grab);
    let color = mix(grab_tint, white, h.pinch * 0.6);

    // Shadow on the imaginary table (y = 0)
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for d in 0..5 {
        for b in 0..4 {
            let (p1, p2) = h.bones[d][b];
            for p in [p1, p2] {
                min_x = min_x.min(p[0]);
                max_x = max_x.max(p[0]);
                min_z = min_z.min(p[2]);
                max_z = max_z.max(p[2]);
            }
        }
    }
    if min_x.is_finite() {
        let cx_mm = (min_x + max_x) * 0.5;
        let cz_mm = (min_z + max_z) * 0.5;
        let (cx, cy) = project([cx_mm, 0.0, cz_mm]);
        let rx = ((max_x - min_x) * 0.5 + 30.0) * PROJ_SCALE;
        let ry = rx * 0.35;
        draw_ellipse(cx, cy, rx, ry, 0.0, color_u8!(0, 0, 0, 110));
    }

    // Arm
    let (ex, ey) = project(h.arm_prev);
    let (wx, wy) = project(h.arm_next);
    draw_line(ex, ey, wx, wy, 6.0, mix(color, BG, 0.45));
    draw_circle_lines(ex, ey, 6.0, 2.0, color);

    // Bones
    for d in 0..5 {
        for b in 0..4 {
            let (p1, p2) = h.bones[d][b];
            let (x1, y1) = project(p1);
            let (x2, y2) = project(p2);
            let z = (p1[2] + p2[2]) * 0.5;
            let thick = (4.0 * depth_factor(z)).max(2.0);
            draw_line(x1, y1, x2, y2, thick, color);
            let joint = mix(color, white, 0.4);
            draw_circle(x1, y1, 3.0, joint);
        }
    }

    // Fingertips
    for d in 0..5 {
        let tip = h.bones[d][3].1;
        let (x, y) = project(tip);
        let r = (6.0 * depth_factor(tip[2])).max(3.0);
        draw_circle(x, y, r, mix(color, white, 0.5));
    }

    // Palm marker
    let (px, py) = project(h.palm);
    let palm_r = 18.0 * depth_factor(h.palm[2]);
    draw_circle_lines(px, py, palm_r, 2.0, color);
    if h.grab > 0.05 {
        draw_circle(px, py, palm_r * h.grab, color);
    }

    // Pinch indicator: glowing segment between thumb and index tips
    if h.pinch > 0.15 {
        let (tx, ty) = project(h.bones[0][3].1);
        let (ix, iy) = project(h.bones[1][3].1);
        let glow = with_alpha(mix(color, white, 0.6), h.pinch);
        draw_line(tx, ty, ix, iy, (4.0 * h.pinch + 2.0).max(2.0), glow);
        let mx = (tx + ix) * 0.5;
        let my = (ty + iy) * 0.5;
        draw_circle(mx, my, 6.0 + 10.0 * h.pinch, with_alpha(white, h.pinch));
    }
}

fn draw_meter(label: &str, value: f32, x: f32, y: f32, color: Color, font: f32) {
    let w = 180.0;
    let height = 14.0;
    draw_rectangle(x, y, w, height, color_u8!(40, 44, 60, 255));
    draw_rectangle(x, y, w * value.clamp(0.0, 1.0), height, color);
    draw_rectangle_lines(x, y, w, height, 1.0, color_u8!(80, 84, 100, 255));
    let txt = format!("{label}: {value:0.2}");
    draw_text(&txt, x, y - 6.0, font, TEXT);
}

#[derive(Default)]
struct TrailMap {
    // (hand_id, finger_idx) -> recent screen points
    trails: std::collections::HashMap<(u32, u8), VecDeque<(f32, f32)>>,
}

impl TrailMap {
    fn update_and_draw(&mut self, hands: &[HandSnap]) {
        let mut live: std::collections::HashSet<(u32, u8)> = Default::default();
        for h in hands {
            let color = if h.is_left { LEFT_C } else { RIGHT_C };
            for fi in 0..5u8 {
                let tip = h.bones[fi as usize][3].1;
                let key = (h.id, fi);
                live.insert(key);
                let q = self.trails.entry(key).or_insert_with(|| VecDeque::with_capacity(28));
                q.push_back(project(tip));
                while q.len() > 24 {
                    q.pop_front();
                }
                if q.len() >= 2 {
                    for i in 1..q.len() {
                        let a = q[i - 1];
                        let b = q[i];
                        let t = i as f32 / q.len() as f32;
                        draw_line(a.0, a.1, b.0, b.1, 2.0, with_alpha(color, 0.85 * t));
                    }
                }
            }
        }
        self.trails.retain(|k, _| live.contains(k));
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Leap Motion — Rust + macroquad".to_owned(),
        window_width: WIDTH,
        window_height: HEIGHT,
        high_dpi: true,
        window_resizable: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let state = Arc::new(Mutex::new(FrameSnap::default()));
    {
        let s = state.clone();
        thread::Builder::new()
            .name("leap-poll".into())
            .spawn(move || poll_thread(s))
            .expect("spawn leap thread");
    }

    let white = color_u8!(255, 255, 255, 255);
    let red = color_u8!(255, 60, 60, 255);

    let mut trails = TrailMap::default();
    let started = Instant::now();
    let mut last_render_fps_t = Instant::now();
    let mut render_frames = 0u32;
    let mut render_fps = 0.0f32;

    loop {
        if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Q) {
            break;
        }

        clear_background(BG);
        draw_grid_bg();

        let snap = state.lock().unwrap().clone();

        // Pulse at the device origin, in screen mm space
        let t = started.elapsed().as_secs_f32();
        let pulse = 0.5 + 0.5 * (t * 2.0).sin();
        let origin = project([0.0, 0.0, 0.0]);
        draw_circle_lines(
            origin.0,
            origin.1,
            10.0 + 6.0 * pulse,
            2.0,
            mix(color_u8!(40, 60, 90, 255), color_u8!(90, 130, 200, 255), pulse),
        );

        trails.update_and_draw(&snap.hands);
        for h in &snap.hands {
            draw_hand(h, white, red);
        }

        // HUD
        draw_text("Leap Motion live demo", 24.0, 38.0, 28.0, TEXT);
        let status = if snap.connected { "connected" } else { "waiting..." };
        draw_text(
            &format!(
                "status: {status}    frame: {}    tracking fps: {:0.1}    render fps: {:0.0}",
                snap.frame_id, snap.framerate, render_fps
            ),
            24.0,
            64.0,
            18.0,
            TEXT,
        );
        draw_text(
            "hold a hand above the device — esc to quit",
            24.0,
            screen_height() - 18.0,
            18.0,
            color_u8!(140, 150, 180, 255),
        );
        if snap.hands.is_empty() {
            draw_text(
                "no hands detected",
                screen_width() - 220.0,
                screen_height() - 18.0,
                18.0,
                color_u8!(200, 90, 90, 255),
            );
        }

        let mut y = 110.0;
        for h in &snap.hands {
            let c = if h.is_left { LEFT_C } else { RIGHT_C };
            let label = format!("{} hand #{}", if h.is_left { "left" } else { "right" }, h.id);
            draw_text(&label, screen_width() - 220.0, y, 18.0, c);
            draw_meter("pinch", h.pinch, screen_width() - 220.0, y + 24.0, c, 16.0);
            draw_meter("grab", h.grab, screen_width() - 220.0, y + 56.0, c, 16.0);
            draw_text(
                &format!("depth: {:+5.0} mm", h.palm[2]),
                screen_width() - 220.0,
                y + 92.0,
                16.0,
                color_u8!(180, 190, 220, 255),
            );
            y += 130.0;
        }

        render_frames += 1;
        if last_render_fps_t.elapsed().as_secs_f32() >= 0.5 {
            render_fps = render_frames as f32 / last_render_fps_t.elapsed().as_secs_f32();
            render_frames = 0;
            last_render_fps_t = Instant::now();
        }

        next_frame().await;
    }
}
