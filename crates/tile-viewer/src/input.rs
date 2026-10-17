//! winit events -> scene input.

use scene::{Button, Input};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::scaling::Plan;

#[derive(Default)]
pub struct InputState {
    /// The pointer in window pixels, and in the driver's (see
    /// [`InputState::set_plan`]).
    window: (f64, f64),
    cursor: (i32, i32),
    plan: Option<Plan>,
}

impl InputState {
    /// The driver's screen is now shown so in the window: pointer positions
    /// are reported in its pixels (the pointer where it is now, if that
    /// moved it).
    pub fn set_plan(&mut self, plan: Plan) -> Option<Input> {
        if Some(plan) == self.plan {
            return None;
        }
        self.plan = Some(plan);
        let cursor = plan.to_screen(self.window);
        (cursor != self.cursor).then(|| {
            self.cursor = cursor;
            Input::MouseMove { x: cursor.0, y: cursor.1 }
        })
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

    pub fn translate(&mut self, event: &WindowEvent) -> Vec<Input> {
        let (x, y) = self.cursor;
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.place_pointer((position.x, position.y));
                vec![Input::MouseMove { x: self.cursor.0, y: self.cursor.1 }]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                    _ => return vec![],
                };
                vec![Input::MouseButton { button, down: *state == ElementState::Pressed, x, y }]
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, l) => (*l * 120.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => p.y as i32,
                };
                vec![Input::Wheel { delta: d, x, y }]
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return vec![] };
                let Some(vk) = virtual_key(code) else { return vec![] };
                let text = event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| !c.is_control());
                vec![Input::Key { vk, down: event.state == ElementState::Pressed, text }]
            }
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
    fn placement_updates_clicks_and_wheels_without_a_motion_event() {
        let mut input = InputState::default();
        let plan = crate::scaling::plan((1920, 1080), 2.0, (960, 540), &viewer_settings::Resolution::default());
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
        let smaller = crate::scaling::plan((1920, 1080), 1.0, (960, 540), &viewer_settings::Resolution::default());
        assert_eq!(input.set_plan(smaller), Some(Input::MouseMove { x: smaller.to_screen(target).0, y: smaller.to_screen(target).1 }));
    }
}
