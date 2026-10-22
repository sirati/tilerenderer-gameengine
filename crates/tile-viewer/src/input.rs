//! winit events -> scene input.

use scene::{Button, Input};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

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
        (0..whole.abs().min(32.0) as usize).map(|_| Input::Wheel { delta, x: self.cursor.0, y: self.cursor.1 }).collect()
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
                let PhysicalKey::Code(code) = event.physical_key else { return vec![] };
                let text = event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| !c.is_control());
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
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
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

    #[test]
    fn trackpad_subpixels_accumulate_without_a_row_per_message() {
        let mut input = InputState::default();
        for _ in 0..159 {
            assert!(input.translate(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 0.25)))).is_empty());
        }
        assert_eq!(input.translate(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 0.25)))), [Input::Wheel { delta: 120, x: 0, y: 0 }]);
        assert_eq!(input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, -3.0))), vec![Input::Wheel { delta: -120, x: 0, y: 0 }; 3]);
    }

    #[test]
    fn wheel_event_work_is_bounded_and_nonfinite_input_does_not_poison_fractions() {
        let mut input = InputState::default();
        for delta in [f64::MAX, -f64::MAX] {
            assert_eq!(input.translate(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, delta)))).len(), 32);
        }
        for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(input.translate(&wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, delta)))).is_empty());
        }
        assert!(input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5))).is_empty());
        assert_eq!(input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5))).len(), 1);
        assert!(input.translate(&wheel_event(MouseScrollDelta::LineDelta(0.0, 0.0))).is_empty());
    }

    #[test]
    fn trackpad_distance_uses_logical_pixels_across_monitor_scales() {
        for scale in [1.0, 1.5, 2.0] {
            let mut input = InputState::default();
            input.set_wheel_scale(scale);
            let delta = wheel_event(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 20.0 * scale)));
            assert!(input.translate(&delta).is_empty());
            assert_eq!(input.translate(&delta), [Input::Wheel { delta: 120, x: 0, y: 0 }]);
        }
    }

    #[test]
    fn wheel_fractions_follow_target_and_modal_context_but_not_frame_updates() {
        let mut input = InputState::default();
        let mut scene = scene::Scene::default();
        scene.interactables.push(scene::Interactable { alt_right_click: false, anchor: None, id: 1,
            rect: [0, 0, 100, 100], kind: scene::InteractableKind::List { scroll: [0, 0, 90, 100], step: 20 } });
        input.sync_wheel_context(&scene, Some((10, 10)), false);
        let half = wheel_event(MouseScrollDelta::LineDelta(0.0, 0.5));
        assert!(input.translate(&half).is_empty());
        scene.seq += 1;
        input.sync_wheel_context(&scene, Some((20, 20)), false);
        assert_eq!(input.translate(&half).len(), 1);
        assert!(input.translate(&half).is_empty());
        input.sync_wheel_context(&scene, Some((200, 200)), false);
        assert!(input.translate(&half).is_empty());
        scene.interface.popup = Some(9);
        input.sync_wheel_context(&scene, Some((200, 200)), false);
        assert!(input.translate(&half).is_empty());
        input.sync_wheel_context(&scene, Some((200, 200)), true);
        assert!(input.translate(&half).is_empty());
        input.translate(&WindowEvent::Focused(false));
        assert!(input.translate(&half).is_empty());
        input.key(KeyCode::AltLeft, true, None);
        assert!(input.translate(&half).is_empty());
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
        assert_eq!(input.translate(&WindowEvent::MouseWheel {
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
