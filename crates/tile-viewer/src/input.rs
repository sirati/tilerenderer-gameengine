//! winit events -> scene input.

use scene::{Button, Input};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::keyboard::{KeyCode, NativeKeyCode, PhysicalKey};

use crate::scaling::Plan;

#[derive(Clone, Default)]
pub struct InputState {
    /// The pointer in window pixels, and in the driver's (see
    /// [`InputState::set_plan`]).
    window: (f64, f64),
    cursor: (i32, i32),
    plan: Option<Plan>,
    /// Physical sides stay distinct until the final release of a shared
    /// Windows virtual key, such as ShiftLeft and ShiftRight.
    keys_down: Vec<KeyCode>,
    wheel_rest: f64,
    wheel_pixels: bool,
    wheel_scale: Option<f64>,
    wheel_context: Option<(Option<String>, Option<u32>, Option<u64>, bool)>,
    /// The top row's Mission Control, Spotlight, Dictation and Do Not
    /// Disturb keys are F3-F6 (macOS, the media keys setting on).
    top_row_keys: bool,
    /// macOS key codes with no key here, already logged.
    logged_native: Vec<u16>,
}

impl InputState {
    /// PixelDelta uses physical pixels. Keep gesture distance independent
    /// of monitor scale while line-based wheels retain their native units.
    pub fn set_wheel_scale(&mut self, scale: f64) {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        if self.wheel_scale != Some(scale) {
            self.wheel_rest = 0.0;
            self.wheel_scale = Some(scale);
        }
    }

    /// Whether the top row's special keys that reach the window with codes
    /// of their own (Mission Control, Spotlight, Dictation, Do Not
    /// Disturb: `function_keys::keys::TOP_ROW_KEYS`) are taken as the
    /// function keys they sit on. Normally the event tap has already made
    /// them F3-F6; this is for when it did not.
    pub fn set_top_row_keys(&mut self, on: bool) { self.top_row_keys = on; }

    /// The key a code winit does not know stands for, if any; logged once
    /// otherwise.
    fn unidentified(&mut self, native: NativeKeyCode) -> Option<KeyCode> {
        let NativeKeyCode::MacOS(code) = native else { return None };
        const F: [KeyCode; 12] = [KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12];
        let key = function_keys::keys::top_row_number(code).filter(|_| self.top_row_keys).and_then(|n| F.get(usize::from(n).checked_sub(1)?).copied());
        if !self.logged_native.contains(&code) {
            self.logged_native.push(code);
            match key {
                Some(key) => log::info!("input: macOS key code {code:#04x} (top row) taken as {key:?}"),
                None => log::info!("input: macOS key code {code:#04x} has no key here; ignored"),
            }
        }
        key
    }

    /// A viewer-owned modal or gesture took over the wheel.
    pub fn reset_wheel(&mut self) { self.wheel_rest = 0.0; }

    /// Fractions belong to the current native scroll target, not to a
    /// frame sequence or the row that happens to occupy its top slot.
    pub fn sync_wheel_context(&mut self, scene: &scene::Scene, point: Option<(i32, i32)>, virtual_pointer: bool) {
        let contains = |r: [i32; 4]| point.is_some_and(|(x, y)| x >= r[0] && y >= r[1] && x < r[0].saturating_add(r[2]) && y < r[1].saturating_add(r[3]));
        let target = scene.interactables.iter().filter(|i| matches!(i.kind, scene::InteractableKind::List { .. }) && contains(i.rect))
            .min_by_key(|i| i64::from(i.rect[2]) * i64::from(i.rect[3]))
            .or_else(|| scene.interactables.iter().rev().find(|i| contains(i.rect))).map(|i| i.id);
        let context = (scene.interface.screen.clone(), scene.interface.popup, target, virtual_pointer);
        if self.wheel_context.as_ref() != Some(&context) {
            self.wheel_rest = 0.0;
            self.wheel_context = Some(context);
        }
    }

    fn wheel(&mut self, delta: &MouseScrollDelta) -> Vec<Input> {
        let (amount, pixels) = match delta {
            MouseScrollDelta::LineDelta(_, y) => (f64::from(*y), false),
            // A conventional wheel line is forty pixels. Keep subpixel
            // trackpad motion until a complete line is available.
            MouseScrollDelta::PixelDelta(p) => (p.y / self.wheel_scale.unwrap_or(1.0) / 40.0, true),
        };
        if !amount.is_finite() { return vec![] }
        // Both axes as reported, unrounded, for the map (`Layout`; pixels
        // in the window's, as the pointer's); the notches below stay as
        // they were for the driver's interface.
        let (sx, sy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y)),
            MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
        };
        let milli = |v: f64| if v.is_finite() { (v * 1000.0).round().clamp(-1e9, 1e9) as i32 } else { 0 };
        let scroll = (milli(sx), milli(sy));
        let scroll = (scroll != (0, 0)).then_some(Input::Scroll { dx: scroll.0, dy: scroll.1, pixels, x: self.cursor.0, y: self.cursor.1 });
        if pixels != self.wheel_pixels { self.wheel_rest = 0.0; }
        self.wheel_pixels = pixels;
        self.wheel_rest += amount;
        let whole = (self.wheel_rest.abs() + 1e-9).floor().copysign(self.wheel_rest);
        self.wheel_rest -= whole;
        if self.wheel_rest.abs() < 1e-9 { self.wheel_rest = 0.0; }
        // The game's list handlers count signed messages, not their
        // magnitude. Emit one message per notch even for coalesced input.
        let delta = if whole < 0.0 { -120 } else { 120 };
        // Discard pathological event excess instead of allocating or
        // replaying an unbounded backlog. Keep only the fractional part.
        scroll.into_iter().chain((0..whole.abs().min(32.0) as usize).map(|_| Input::Wheel { delta, x: self.cursor.0, y: self.cursor.1 })).collect()
    }

    /// The driver's screen is now shown so in the window: pointer positions
    /// are reported in its pixels (the pointer where it is now, if that
    /// moved it).
    pub fn set_plan(&mut self, plan: Plan) -> Option<Input> {
        if Some(plan) == self.plan {
            return None;
        }
        self.plan = Some(plan);
        self.wheel_rest = 0.0;
        let cursor = plan.to_screen(self.window);
        (cursor != self.cursor).then(|| {
            self.cursor = cursor;
            Input::MouseMove { x: cursor.0, y: cursor.1 }
        })
    }

    /// Physical window pixels, before the native screen transform.
    pub fn window_pointer(&self) -> (f64, f64) { self.window }

    pub fn held_virtual_keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys_down.iter().filter_map(|&code| virtual_key(code))
    }

    /// The latest known pointer in canvas pixels.
    pub fn pointer(&self) -> (i32, i32) {
        self.cursor
    }

    /// The pointer was placed in window pixels. Wayland position hints can
    /// move it without a CursorMoved event, so the next click and wheel must
    /// already use this position.
    pub fn place_pointer(&mut self, window: (f64, f64)) -> (i32, i32) {
        self.window = window;
        self.cursor = self.plan.map_or((window.0 as i32, window.1 as i32), |p| p.to_screen(window));
        self.cursor
    }

    fn key(&mut self, code: KeyCode, down: bool, text: Option<char>) -> Vec<Input> {
        let Some(vk) = virtual_key(code) else { return vec![] };
        self.wheel_rest = 0.0;
        if down {
            if !self.keys_down.contains(&code) { self.keys_down.push(code); }
        } else {
            self.keys_down.retain(|&held| held != code);
            if self.keys_down.iter().any(|&held| virtual_key(held) == Some(vk)) {
                return vec![];
            }
        }
        vec![Input::Key { vk, down, text }]
    }

    fn release_keys(&mut self) -> Vec<Input> {
        let mut released = Vec::new();
        for code in self.keys_down.drain(..).rev() {
            let Some(vk) = virtual_key(code) else { continue };
            if !released.contains(&vk) { released.push(vk); }
        }
        released.into_iter().map(|vk| Input::Key { vk, down: false, text: None }).collect()
    }

    pub fn translate(&mut self, event: &WindowEvent) -> Vec<Input> {
        let (x, y) = self.cursor;
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.place_pointer((position.x, position.y));
                vec![Input::MouseMove { x: self.cursor.0, y: self.cursor.1 }]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.wheel_rest = 0.0;
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                    _ => return vec![],
                };
                vec![Input::MouseButton { button, down: *state == ElementState::Pressed, x, y }]
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                if *phase == TouchPhase::Cancelled { self.wheel_rest = 0.0; return vec![]; }
                if *phase == TouchPhase::Started { self.wheel_rest = 0.0; }
                let out = self.wheel(delta);
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) { self.wheel_rest = 0.0; }
                out
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let (code, text) = match event.physical_key {
                    PhysicalKey::Code(code) => (code, event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| !c.is_control())),
                    PhysicalKey::Unidentified(native) => match self.unidentified(native) {
                        Some(code) => (code, None),
                        None => return vec![],
                    },
                };
                self.key(code, event.state == ElementState::Pressed, text)
            }
            WindowEvent::Focused(false) => { self.wheel_rest = 0.0; self.release_keys() },
            WindowEvent::CursorLeft { .. } => { self.wheel_rest = 0.0; vec![] },
            _ => vec![],
        }
    }
}

/// Physical key -> Windows virtual-key code (US layout positions).
pub fn virtual_key(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    let letters = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
    if let Some(i) = letters.iter().position(|&k| k == code) {
        return Some(0x41 + i as u32);
    }
    let digits = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    if let Some(i) = digits.iter().position(|&k| k == code) {
        return Some(0x30 + i as u32);
    }
    let numpad = [Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9];
    if let Some(i) = numpad.iter().position(|&k| k == code) {
        return Some(0x60 + i as u32);
    }
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24];
    if let Some(i) = fkeys.iter().position(|&k| k == code) {
        return Some(0x70 + i as u32);
    }
    Some(match code {
        Backspace => 0x08,
        Tab => 0x09,
        Enter | NumpadEnter => 0x0D,
        ShiftLeft | ShiftRight => 0x10,
        ControlLeft | ControlRight => 0x11,
        AltLeft | AltRight => 0x12,
        Pause => 0x13,
        Escape => 0x1B,
        Space => 0x20,
        PageUp => 0x21,
        PageDown => 0x22,
        End => 0x23,
        Home => 0x24,
        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,
        Insert => 0x2D,
        Delete => 0x2E,
        NumpadMultiply => 0x6A,
        NumpadAdd => 0x6B,
        NumpadSubtract => 0x6D,
        NumpadDecimal => 0x6E,
        NumpadDivide => 0x6F,
        Semicolon => 0xBA,
        Equal => 0xBB,
        Comma => 0xBC,
        Minus => 0xBD,
        Period => 0xBE,
        Slash => 0xBF,
        Backquote => 0xC0,
        BracketLeft => 0xDB,
        Backslash => 0xDC,
        BracketRight => 0xDD,
        Quote => 0xDE,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::{DeviceId, TouchPhase};

    #[test]
    fn top_row_codes_are_f3_to_f6_only_when_asked() {
        let mut input = InputState::default();
        for code in [0xA0u16, 0xB1, 0xB0, 0xCF, 0xB2] {
            assert_eq!(input.unidentified(NativeKeyCode::MacOS(code)), None, "{code:#x}");
        }
        input.set_top_row_keys(true);
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xA0)), Some(KeyCode::F3));
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xB1)), Some(KeyCode::F4));
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xB0)), Some(KeyCode::F5));
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xCF)), Some(KeyCode::F5));
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xB2)), Some(KeyCode::F6));
        assert_eq!(input.unidentified(NativeKeyCode::MacOS(0xB3)), None);
        assert_eq!(input.unidentified(NativeKeyCode::Windows(0xA0)), None);
        // Pressed and released as F3, VK_F3 (0x72), held in between.
        assert_eq!(input.key(KeyCode::F3, true, None), [Input::Key { vk: 0x72, down: true, text: None }]);
        assert_eq!(input.held_virtual_keys().collect::<Vec<_>>(), [0x72]);
        assert_eq!(input.key(KeyCode::F3, false, None), [Input::Key { vk: 0x72, down: false, text: None }]);
    }

    #[test]
    fn function_keys_map_to_their_windows_codes() {
        // VK_F1..VK_F12 (Civ III's advisors, F11 demographics), and on to VK_F24.
        assert_eq!(virtual_key(KeyCode::F1), Some(0x70));
        assert_eq!(virtual_key(KeyCode::F8), Some(0x77));
        assert_eq!(virtual_key(KeyCode::F11), Some(0x7A));
        assert_eq!(virtual_key(KeyCode::F12), Some(0x7B));
        assert_eq!(virtual_key(KeyCode::F13), Some(0x7C));
        assert_eq!(virtual_key(KeyCode::F24), Some(0x87));
        assert_eq!(virtual_key(KeyCode::Fn), None);
    }

    #[test]
    fn focus_loss_releases_held_keys_in_reverse_order_without_repeats() {
        let mut input = InputState::default();
        input.key(KeyCode::ShiftLeft, true, None);
        input.key(KeyCode::ControlRight, true, None);
        input.key(KeyCode::KeyA, true, Some('A'));
        input.key(KeyCode::KeyA, true, Some('A'));
        assert_eq!(input.translate(&WindowEvent::Focused(false)), vec![
            Input::Key { vk: 0x41, down: false, text: None },
            Input::Key { vk: 0x11, down: false, text: None },
            Input::Key { vk: 0x10, down: false, text: None },
        ]);
        assert!(input.translate(&WindowEvent::Focused(false)).is_empty());
        assert!(input.translate(&WindowEvent::Focused(true)).is_empty());
    }

    #[test]
    fn releasing_one_physical_modifier_side_keeps_the_other_held() {
        let mut input = InputState::default();
        input.key(KeyCode::ShiftLeft, true, None);
        input.key(KeyCode::ShiftRight, true, None);
        assert!(input.key(KeyCode::ShiftLeft, false, None).is_empty());
        assert_eq!(input.key(KeyCode::ShiftRight, false, None), vec![
            Input::Key { vk: 0x10, down: false, text: None },
        ]);
        input.key(KeyCode::AltLeft, true, None);
        input.key(KeyCode::AltRight, true, None);
        assert_eq!(input.translate(&WindowEvent::Focused(false)), vec![
            Input::Key { vk: 0x12, down: false, text: None },
        ]);
    }

    fn wheel_event(delta: MouseScrollDelta) -> WindowEvent {
        WindowEvent::MouseWheel { device_id: DeviceId::dummy(), delta, phase: TouchPhase::Moved }
    }

    impl InputState {
        /// What `event` makes for the driver's interface: its `Wheel`
        /// notches (not the map's `Scroll`).
        fn wheels(&mut self, event: &WindowEvent) -> Vec<Input> {
            self.translate(event).into_iter().filter(|i| !matches!(i, Input::Scroll { .. })).collect()
        }
    }

    #[test]
    fn a_scroll_reports_both_axes_unrounded_before_its_notches() {
        let mut input = InputState::default();
        input.set_wheel_scale(2.0);
        // A trackpad, diagonally: window pixels as reported, no notch yet.
        let out = input.translate(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(-3.5, 1.25))));
        assert_eq!(out, [Input::Scroll { dx: -3500, dy: 1250, pixels: true, x: 0, y: 0 }]);
        // A tilt wheel: sideways lines only (no notch for the interface).
        let out = input.translate(&wheel_event(MouseScrollDelta::LineDelta(1.0, 0.0)));
        assert_eq!(out, [Input::Scroll { dx: 1000, dy: 0, pixels: false, x: 0, y: 0 }]);
        // A wheel: its lines, then its notches.
        let out = input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, -2.0)));
        assert_eq!(out, [Input::Scroll { dx: 0, dy: -2000, pixels: false, x: 0, y: 0 }, Input::Wheel { delta: -120, x: 0, y: 0 }, Input::Wheel { delta: -120, x: 0, y: 0 }]);
        assert!(input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.0))).is_empty());
    }

    #[test]
    fn trackpad_subpixels_accumulate_without_a_row_per_message() {
        let mut input = InputState::default();
        for _ in 0..159 {
            assert!(input.wheels(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 0.25)))).is_empty());
        }
        assert_eq!(input.wheels(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 0.25)))), [Input::Wheel { delta: 120, x: 0, y: 0 }]);
        assert_eq!(input.wheels(&wheel_event(MouseScrollDelta::LineDelta(0.0, -3.0))), vec![Input::Wheel { delta: -120, x: 0, y: 0 }; 3]);
    }

    #[test]
    fn wheel_event_work_is_bounded_and_nonfinite_input_does_not_poison_fractions() {
        let mut input = InputState::default();
        for delta in [f64::MAX, -f64::MAX] {
            assert_eq!(input.wheels(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, delta)))).len(), 32);
        }
        for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(input.wheels(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, delta)))).is_empty());
        }
        assert!(input.wheels(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5))).is_empty());
        assert_eq!(input.wheels(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5))).len(), 1);
        assert!(input.wheels(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.0))).is_empty());
    }

    #[test]
    fn trackpad_distance_uses_logical_pixels_across_monitor_scales() {
        for scale in [1.0, 1.5, 2.0] {
            let mut input = InputState::default();
            input.set_wheel_scale(scale);
            let delta = wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 20.0 * scale)));
            assert!(input.wheels(&delta).is_empty());
            assert_eq!(input.wheels(&delta), [Input::Wheel { delta: 120, x: 0, y: 0 }]);
        }
    }

    #[test]
    fn wheel_fractions_follow_target_and_modal_context_but_not_frame_updates() {
        let mut input = InputState::default();
        let mut scene = scene::Scene::default();
        scene.interactables.push(scene::Interactable { region: None, alt_right_click: false, anchor: None, id: 1,
            rect: [0, 0, 100, 100], kind: scene::InteractableKind::List { scroll: [0, 0, 90, 100], step: 20 } });
        input.sync_wheel_context(&scene, Some((10, 10)), false);
        let half = wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5));
        assert!(input.wheels(&half).is_empty());
        scene.seq += 1;
        input.sync_wheel_context(&scene, Some((20, 20)), false);
        assert_eq!(input.wheels(&half).len(), 1);
        assert!(input.wheels(&half).is_empty());
        input.sync_wheel_context(&scene, Some((200, 200)), false);
        assert!(input.wheels(&half).is_empty());
        scene.interface.popup = Some(9);
        input.sync_wheel_context(&scene, Some((200, 200)), false);
        assert!(input.wheels(&half).is_empty());
        input.sync_wheel_context(&scene, Some((200, 200)), true);
        assert!(input.wheels(&half).is_empty());
        input.translate(&WindowEvent::Focused(false));
        assert!(input.wheels(&half).is_empty());
        input.key(KeyCode::AltLeft, true, None);
        assert!(input.wheels(&half).is_empty());
    }

    #[test]
    fn placement_updates_clicks_and_wheels_without_a_motion_event() {
        let mut input = InputState::default();
        let plan = crate::scaling::plan((1920, 1080), (960, 540), &viewer_settings::Resolution { pixel_scale: 2, ..Default::default() });
        input.set_plan(plan);
        input.place_pointer((100.0, 80.0));
        let target = (1200.0, 700.0);
        let at = plan.to_screen(target);
        assert_eq!(input.place_pointer(target), at);
        assert_eq!(input.translate(&WindowEvent::MouseInput {
            device_id: DeviceId::dummy(), state: ElementState::Pressed, button: MouseButton::Left,
        }), vec![Input::MouseButton { button: Button::Left, down: true, x: at.0, y: at.1 }]);
        assert_eq!(input.wheels(&WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(), delta: MouseScrollDelta::LineDelta(0.0, 1.0), phase: TouchPhase::Moved,
        }), vec![Input::Wheel { delta: 120, x: at.0, y: at.1 }]);
        // A later physical motion replaces the hinted position normally.
        input.translate(&WindowEvent::CursorMoved { device_id: DeviceId::dummy(), position: winit::dpi::PhysicalPosition::new(400.0, 200.0) });
        assert_eq!(input.cursor, plan.to_screen((400.0, 200.0)));
        // Scale changes also start from the placed window position.
        input.place_pointer(target);
        let smaller = crate::scaling::plan((1920, 1080), (960, 540), &viewer_settings::Resolution::default());
        assert_eq!(input.set_plan(smaller), Some(Input::MouseMove { x: smaller.to_screen(target).0, y: smaller.to_screen(target).1 }));
    }
}
