//! Minimal FFI bindings to libLeapC. Only what the demo needs.
//!
//! NOTE: The Ultraleap SDK header wraps everything in `#pragma pack(1)`, so
//! every struct is byte-packed with no alignment padding. We mirror that with
//! `#[repr(C, packed)]`. Field reads must go through `read_unaligned` (see the
//! `get!` macro in main.rs).

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::ffi::c_void;

pub const eLeapRS_Success: u32 = 0;

pub const eLeapEventType_Connection: u32 = 1;
pub const eLeapEventType_ConnectionLost: u32 = 2;
pub const eLeapEventType_Tracking: u32 = 0x100;

pub const eLeapHandType_Left: u32 = 0;

pub type LEAP_CONNECTION = *mut c_void;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct LEAP_VECTOR {
    pub v: [f32; 3],
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct LEAP_QUATERNION {
    pub v: [f32; 4],
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct LEAP_BONE {
    pub prev_joint: LEAP_VECTOR,
    pub next_joint: LEAP_VECTOR,
    pub width: f32,
    pub rotation: LEAP_QUATERNION,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct LEAP_DIGIT {
    pub finger_id: i32,
    pub bones: [LEAP_BONE; 4], // [metacarpal, proximal, intermediate, distal]
    pub is_extended: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct LEAP_PALM {
    pub position: LEAP_VECTOR,
    pub stabilized_position: LEAP_VECTOR,
    pub velocity: LEAP_VECTOR,
    pub normal: LEAP_VECTOR,
    pub width: f32,
    pub direction: LEAP_VECTOR,
    pub orientation: LEAP_QUATERNION,
}

#[repr(C, packed)]
pub struct LEAP_HAND {
    pub id: u32,
    pub flags: u32,
    pub hand_type: u32, // eLeapHandType
    pub confidence: f32,
    pub visible_time: u64,
    pub pinch_distance: f32,
    pub grab_angle: f32,
    pub pinch_strength: f32,
    pub grab_strength: f32,
    pub palm: LEAP_PALM,
    pub digits: [LEAP_DIGIT; 5], // [thumb, index, middle, ring, pinky]
    pub arm: LEAP_BONE,
}

#[repr(C, packed)]
pub struct LEAP_FRAME_HEADER {
    pub reserved: *mut c_void,
    pub frame_id: i64,
    pub timestamp: i64,
}

#[repr(C, packed)]
pub struct LEAP_TRACKING_EVENT {
    pub info: LEAP_FRAME_HEADER,
    pub tracking_frame_id: i64,
    pub nHands: u32,
    pub pHands: *const LEAP_HAND,
    pub framerate: f32,
}

#[repr(C, packed)]
pub struct LEAP_CONNECTION_MESSAGE {
    pub size: u32,
    pub msg_type: u32, // eLeapEventType
    pub event_ptr: *const c_void,
    pub device_id: u32,
}

extern "C" {
    pub fn LeapCreateConnection(
        config: *const c_void,
        conn: *mut LEAP_CONNECTION,
    ) -> u32;
    pub fn LeapOpenConnection(conn: LEAP_CONNECTION) -> u32;
    pub fn LeapCloseConnection(conn: LEAP_CONNECTION);
    pub fn LeapDestroyConnection(conn: LEAP_CONNECTION);
    pub fn LeapPollConnection(
        conn: LEAP_CONNECTION,
        timeout_ms: u32,
        msg: *mut LEAP_CONNECTION_MESSAGE,
    ) -> u32;
}

// --- Compile-time layout sanity checks --------------------------------------
// These match what the SDK header reports under #pragma pack(1) on macOS arm64.
const _: [(); 12] = [(); std::mem::size_of::<LEAP_VECTOR>()];
const _: [(); 16] = [(); std::mem::size_of::<LEAP_QUATERNION>()];
const _: [(); 44] = [(); std::mem::size_of::<LEAP_BONE>()];
const _: [(); 184] = [(); std::mem::size_of::<LEAP_DIGIT>()];
const _: [(); 80] = [(); std::mem::size_of::<LEAP_PALM>()];
const _: [(); 1084] = [(); std::mem::size_of::<LEAP_HAND>()];
const _: [(); 24] = [(); std::mem::size_of::<LEAP_FRAME_HEADER>()];
const _: [(); 48] = [(); std::mem::size_of::<LEAP_TRACKING_EVENT>()];
const _: [(); 20] = [(); std::mem::size_of::<LEAP_CONNECTION_MESSAGE>()];
