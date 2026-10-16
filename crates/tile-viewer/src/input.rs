//! winit events -> scene input.

use scene::{Button, Input};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Default)]
pub struct InputState {
    cursor: (i32, i32),
}

impl InputState {
    pub fn translate(&mut self, event: &WindowEvent) -> Vec<Input> {
        let (x, y) = self.cursor;
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as i32, position.y as i32);
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
