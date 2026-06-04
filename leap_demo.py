"""Visual Leap Motion demo using pygame + the LeapC cffi bindings shipped
with the Ultraleap Hand Tracking installer.

Draws live skeletons for both hands, pinch/grab meters, finger-tip trails,
and a depth-cued shadow. Quit with Esc or window close.
"""

from __future__ import annotations

import math
import os
import sys
import threading
import time
from collections import deque
from dataclasses import dataclass, field

SDK_DIR = "/Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK"
if SDK_DIR not in sys.path:
    sys.path.insert(0, SDK_DIR)

os.environ.setdefault("PYGAME_HIDE_SUPPORT_PROMPT", "1")

import pygame  # noqa: E402
from leapc_cffi import ffi, libleapc  # noqa: E402


WIDTH, HEIGHT = 1280, 800
BG_COLOR = (10, 12, 22)
GRID_COLOR = (28, 32, 50)
TEXT_COLOR = (220, 225, 240)
LEFT_COLOR = (90, 200, 255)
RIGHT_COLOR = (255, 170, 80)

DIGIT_NAMES = ("thumb", "index", "middle", "ring", "pinky")


@dataclass
class HandSnapshot:
    id: int
    is_left: bool
    palm: tuple[float, float, float]
    palm_normal: tuple[float, float, float]
    pinch: float
    grab: float
    arm: tuple[tuple[float, float, float], tuple[float, float, float]]
    digits: list[list[tuple[tuple[float, float, float], tuple[float, float, float]]]]
    fingertips: list[tuple[float, float, float]]


@dataclass
class FrameState:
    hands: list[HandSnapshot] = field(default_factory=list)
    frame_id: int = 0
    fps: float = 0.0
    connected: bool = False
    device: str = ""


def _v(vec) -> tuple[float, float, float]:
    return (float(vec.x), float(vec.y), float(vec.z))


def _copy_hand(hand) -> HandSnapshot:
    digits = []
    fingertips = []
    for d in range(5):
        digit = hand.digits[d]
        bones = []
        for b in range(4):
            bone = digit.bones[b]
            bones.append((_v(bone.prev_joint), _v(bone.next_joint)))
        digits.append(bones)
        fingertips.append(_v(digit.distal.next_joint))
    return HandSnapshot(
        id=int(hand.id),
        is_left=(hand.type == libleapc.eLeapHandType_Left),
        palm=_v(hand.palm.position),
        palm_normal=_v(hand.palm.normal),
        pinch=float(hand.pinch_strength),
        grab=float(hand.grab_strength),
        arm=(_v(hand.arm.prev_joint), _v(hand.arm.next_joint)),
        digits=digits,
        fingertips=fingertips,
    )


class LeapReader(threading.Thread):
    """Polls the Leap service on a background thread and publishes the latest
    frame into the shared FrameState."""

    def __init__(self, state: FrameState, lock: threading.Lock) -> None:
        super().__init__(daemon=True, name="LeapReader")
        self.state = state
        self.lock = lock
        self._stop_event = threading.Event()

    def stop(self) -> None:
        self._stop_event.set()

    def run(self) -> None:
        conn_h = ffi.new("LEAP_CONNECTION*")
        rc = libleapc.LeapCreateConnection(ffi.NULL, conn_h)
        if rc != libleapc.eLeapRS_Success:
            print(f"LeapCreateConnection failed: 0x{rc:08x}", file=sys.stderr)
            return
        conn = conn_h[0]
        rc = libleapc.LeapOpenConnection(conn)
        if rc != libleapc.eLeapRS_Success:
            print(f"LeapOpenConnection failed: 0x{rc:08x}", file=sys.stderr)
            libleapc.LeapDestroyConnection(conn)
            return

        msg = ffi.new("LEAP_CONNECTION_MESSAGE*")
        try:
            while not self._stop_event.is_set():
                rc = libleapc.LeapPollConnection(conn, 200, msg)
                if rc != libleapc.eLeapRS_Success:
                    continue
                t = msg.type
                if t == libleapc.eLeapEventType_Connection:
                    with self.lock:
                        self.state.connected = True
                elif t == libleapc.eLeapEventType_ConnectionLost:
                    with self.lock:
                        self.state.connected = False
                elif t == libleapc.eLeapEventType_Device:
                    serial = ""
                    dev_h = ffi.new("LEAP_DEVICE*")
                    if libleapc.LeapOpenDevice(msg.device_event.device, dev_h) == 0:
                        info = ffi.new("LEAP_DEVICE_INFO*")
                        info.size = ffi.sizeof("LEAP_DEVICE_INFO")
                        info.serial_length = 64
                        info.serial = ffi.new("char[64]")
                        if libleapc.LeapGetDeviceInfo(dev_h[0], info) == 0:
                            serial = ffi.string(info.serial).decode("utf-8", "replace")
                        libleapc.LeapCloseDevice(dev_h[0])
                    with self.lock:
                        self.state.device = serial
                elif t == libleapc.eLeapEventType_Tracking:
                    ev = msg.tracking_event
                    hands = [_copy_hand(ev.pHands[i]) for i in range(ev.nHands)]
                    with self.lock:
                        self.state.hands = hands
                        self.state.frame_id = int(ev.tracking_frame_id)
                        self.state.fps = float(ev.framerate)
        finally:
            libleapc.LeapCloseConnection(conn)
            libleapc.LeapDestroyConnection(conn)


# --- Projection ---------------------------------------------------------------
# Leap coordinates: +x right, +y up (above the device), +z toward user.
# Project (x, y) orthographically and use z to scale/dim a shadow.

PROJ_SCALE = 2.2  # pixels per millimeter
CENTER_Y_MM = 250  # vertical position above the device that maps to screen center


def project(p: tuple[float, float, float]) -> tuple[float, float]:
    x, y, _z = p
    sx = WIDTH / 2 + x * PROJ_SCALE
    sy = HEIGHT / 2 - (y - CENTER_Y_MM) * PROJ_SCALE
    return (sx, sy)


def depth_factor(z: float) -> float:
    # Closer to user (+z) -> bigger; farther -> smaller. Clamp to a sane range.
    return max(0.45, min(1.6, 1.0 + (-z) / 400.0))


# --- Drawing helpers ---------------------------------------------------------


def draw_grid(surface: pygame.Surface) -> None:
    step = 80
    for x in range(0, WIDTH, step):
        pygame.draw.line(surface, GRID_COLOR, (x, 0), (x, HEIGHT), 1)
    for y in range(0, HEIGHT, step):
        pygame.draw.line(surface, GRID_COLOR, (0, y), (WIDTH, y), 1)
    pygame.draw.line(surface, (50, 55, 80), (0, HEIGHT // 2), (WIDTH, HEIGHT // 2), 1)
    pygame.draw.line(surface, (50, 55, 80), (WIDTH // 2, 0), (WIDTH // 2, HEIGHT), 1)


def mix(c1: tuple[int, int, int], c2: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    t = max(0.0, min(1.0, t))
    return (
        int(c1[0] + (c2[0] - c1[0]) * t),
        int(c1[1] + (c2[1] - c1[1]) * t),
        int(c1[2] + (c2[2] - c1[2]) * t),
    )


def draw_hand(surface: pygame.Surface, hand: HandSnapshot) -> None:
    base = LEFT_COLOR if hand.is_left else RIGHT_COLOR
    grab_tint = mix(base, (255, 60, 60), hand.grab)
    color = mix(grab_tint, (255, 255, 255), hand.pinch * 0.6)

    # Soft shadow on the imaginary table plane (y = 0 in Leap space).
    shadow_pts = []
    for digit in hand.digits:
        for prev_p, next_p in digit:
            shadow_pts.append(project((prev_p[0], 0, prev_p[2])))
            shadow_pts.append(project((next_p[0], 0, next_p[2])))
    if shadow_pts:
        xs = [p[0] for p in shadow_pts]
        ys = [p[1] for p in shadow_pts]
        cx = sum(xs) / len(xs)
        cy = sum(ys) / len(ys)
        sw, sh = max(xs) - min(xs), max(ys) - min(ys)
        shadow = pygame.Surface((max(20, int(sw + 60)), max(12, int(sh + 30))), pygame.SRCALPHA)
        pygame.draw.ellipse(shadow, (0, 0, 0, 90), shadow.get_rect())
        surface.blit(shadow, shadow.get_rect(center=(cx, cy)))

    # Arm
    elbow = project(hand.arm[0])
    wrist = project(hand.arm[1])
    pygame.draw.line(surface, mix(color, BG_COLOR, 0.45), elbow, wrist, 6)
    pygame.draw.circle(surface, color, (int(elbow[0]), int(elbow[1])), 6, 2)

    # Digit bones
    for digit in hand.digits:
        for prev_p, next_p in digit:
            p1 = project(prev_p)
            p2 = project(next_p)
            thickness = max(2, int(4 * depth_factor((prev_p[2] + next_p[2]) / 2)))
            pygame.draw.line(surface, color, p1, p2, thickness)
            pygame.draw.circle(surface, mix(color, (255, 255, 255), 0.4),
                               (int(p1[0]), int(p1[1])), 3)

    # Finger tips
    for tip in hand.fingertips:
        p = project(tip)
        r = max(3, int(6 * depth_factor(tip[2])))
        pygame.draw.circle(surface, mix(color, (255, 255, 255), 0.5),
                           (int(p[0]), int(p[1])), r)

    # Palm marker (size hints depth, fill hints grab)
    palm = project(hand.palm)
    palm_r = int(18 * depth_factor(hand.palm[2]))
    pygame.draw.circle(surface, color, (int(palm[0]), int(palm[1])), palm_r, 2)
    if hand.grab > 0.05:
        pygame.draw.circle(surface, color, (int(palm[0]), int(palm[1])),
                           int(palm_r * hand.grab))

    # Pinch indicator between thumb and index tips
    if hand.pinch > 0.15:
        thumb_tip = project(hand.fingertips[0])
        index_tip = project(hand.fingertips[1])
        alpha = int(255 * hand.pinch)
        layer = pygame.Surface((WIDTH, HEIGHT), pygame.SRCALPHA)
        pygame.draw.line(layer, (*mix(color, (255, 255, 255), 0.6), alpha),
                         thumb_tip, index_tip, max(2, int(4 * hand.pinch + 2)))
        mid = ((thumb_tip[0] + index_tip[0]) / 2, (thumb_tip[1] + index_tip[1]) / 2)
        pygame.draw.circle(layer, (255, 255, 255, alpha), (int(mid[0]), int(mid[1])),
                           int(6 + 10 * hand.pinch))
        surface.blit(layer, (0, 0))


def draw_trails(surface: pygame.Surface, trails: dict[tuple[int, int], deque],
                hand: HandSnapshot) -> None:
    color = LEFT_COLOR if hand.is_left else RIGHT_COLOR
    for finger_idx, tip in enumerate(hand.fingertips):
        key = (hand.id, finger_idx)
        trail = trails.setdefault(key, deque(maxlen=24))
        trail.append(project(tip))
        if len(trail) < 2:
            continue
        for i in range(1, len(trail)):
            alpha = int(220 * (i / len(trail)))
            layer = pygame.Surface((WIDTH, HEIGHT), pygame.SRCALPHA)
            pygame.draw.line(layer, (*color, alpha), trail[i - 1], trail[i], 2)
            surface.blit(layer, (0, 0))


def draw_meter(surface: pygame.Surface, label: str, value: float,
               x: int, y: int, color: tuple[int, int, int],
               font: pygame.font.Font) -> None:
    w, h = 180, 14
    pygame.draw.rect(surface, (40, 44, 60), (x, y, w, h), border_radius=4)
    pygame.draw.rect(surface, color, (x, y, int(w * max(0, min(1, value))), h),
                     border_radius=4)
    pygame.draw.rect(surface, (80, 84, 100), (x, y, w, h), 1, border_radius=4)
    txt = font.render(f"{label}: {value:0.2f}", True, TEXT_COLOR)
    surface.blit(txt, (x, y - 18))


def draw_hud(surface: pygame.Surface, state: FrameState,
             font: pygame.font.Font, big: pygame.font.Font) -> None:
    title = big.render("Leap Motion live demo", True, TEXT_COLOR)
    surface.blit(title, (24, 18))

    status = "connected" if state.connected else "waiting for service..."
    line1 = font.render(f"status: {status}    frame: {state.frame_id}    "
                        f"tracking fps: {state.fps:0.1f}", True, TEXT_COLOR)
    surface.blit(line1, (24, 60))
    if state.device:
        line2 = font.render(f"device: {state.device}", True, (160, 170, 200))
        surface.blit(line2, (24, 80))

    hint = font.render("hold a hand above the device — esc to quit",
                       True, (140, 150, 180))
    surface.blit(hint, (24, HEIGHT - 28))

    if not state.hands:
        msg = font.render("no hands detected", True, (200, 90, 90))
        surface.blit(msg, (WIDTH - 220, HEIGHT - 28))

    # Per-hand meters down the right side
    y = 110
    for hand in state.hands:
        color = LEFT_COLOR if hand.is_left else RIGHT_COLOR
        label = f"{'left' if hand.is_left else 'right'} hand #{hand.id}"
        head = font.render(label, True, color)
        surface.blit(head, (WIDTH - 220, y))
        draw_meter(surface, "pinch", hand.pinch, WIDTH - 220, y + 38, color, font)
        draw_meter(surface, "grab",  hand.grab,  WIDTH - 220, y + 78, color, font)
        z_mm = hand.palm[2]
        zline = font.render(f"depth: {z_mm:+5.0f} mm", True, (180, 190, 220))
        surface.blit(zline, (WIDTH - 220, y + 100))
        y += 150


# --- Main loop ---------------------------------------------------------------


def main() -> None:
    pygame.init()
    pygame.display.set_caption("Leap Motion — pygame demo")
    screen = pygame.display.set_mode((WIDTH, HEIGHT))
    clock = pygame.time.Clock()
    font = pygame.font.SysFont("Menlo", 16)
    big = pygame.font.SysFont("Menlo", 28, bold=True)

    state = FrameState()
    lock = threading.Lock()
    reader = LeapReader(state, lock)
    reader.start()

    trails: dict[tuple[int, int], deque] = {}
    pulse_t0 = time.time()

    running = True
    while running:
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False
            elif event.type == pygame.KEYDOWN and event.key in (
                pygame.K_ESCAPE, pygame.K_q
            ):
                running = False

        with lock:
            snap_hands = list(state.hands)
            frame_snap = FrameState(hands=snap_hands, frame_id=state.frame_id,
                                    fps=state.fps, connected=state.connected,
                                    device=state.device)

        screen.fill(BG_COLOR)
        draw_grid(screen)

        # Subtle pulse at origin so you can see the device is "alive" even
        # without hands.
        pulse = 0.5 + 0.5 * math.sin((time.time() - pulse_t0) * 2.0)
        pygame.draw.circle(screen, mix((40, 60, 90), (90, 130, 200), pulse),
                           (WIDTH // 2, HEIGHT // 2 + int(CENTER_Y_MM * PROJ_SCALE)),
                           int(10 + 6 * pulse), 2)

        for hand in snap_hands:
            draw_trails(screen, trails, hand)
            draw_hand(screen, hand)

        # Cull old trails for vanished hands
        live_keys = {(h.id, i) for h in snap_hands for i in range(5)}
        for k in list(trails.keys()):
            if k not in live_keys:
                trails[k].popleft() if trails[k] else None
                if not trails[k]:
                    del trails[k]

        draw_hud(screen, frame_snap, font, big)

        pygame.display.flip()
        clock.tick(60)

    reader.stop()
    reader.join(timeout=1.5)
    pygame.quit()


if __name__ == "__main__":
    main()
