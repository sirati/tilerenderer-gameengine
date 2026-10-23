//! Map aids, independent of the arrangement of the world views.
use serde::{Deserialize, Serialize};
use crate::controls::{ControlValue, Range};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    pub active_unit_arrow: bool,
    pub arrow_bobbing: bool,
    pub unit_rings: bool,
    pub war: RingColor,
    pub allied: RingColor,
    pub peace: RingColor,
    pub ring_width: u32,
    /// Index into HUD_SCALES, rather than an arbitrary fractional size.
    pub hud_size: usize,
    pub stack_count: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RingColor {
    pub color: [u8; 3],
    pub invert: bool,
}
impl Default for RingColor {
    fn default() -> Self { Self { color: [255, 0, 0], invert: false } }
}
impl Default for Accessibility {
    fn default() -> Self {
        Self { active_unit_arrow: false, arrow_bobbing: true, unit_rings: false,
            war: RingColor::default(), allied: RingColor { color: [0, 64, 255], invert: false },
            peace: RingColor { color: [64, 240, 96], invert: false }, ring_width: 2, hud_size: 0, stack_count: false }
    }
}

pub const HUD_SCALES: [f32; 4] = [1.0, 1.5, 2.0, 3.0];
/// The compact palette shown beside each diplomatic relation.
pub const PALETTE: [[u8; 3]; 16] = [
    [255, 0, 0], [240, 112, 24], [248, 208, 32], [64, 240, 96],
    [0, 152, 112], [32, 208, 216], [0, 64, 255], [104, 64, 192],
    [200, 48, 160], [248, 144, 176], [144, 80, 40], [192, 168, 104],
    [255, 255, 255], [176, 176, 176], [80, 80, 80], [0, 0, 0],
];
pub const KEYS: [&str; 12] = [
    "accessibility.active_unit_arrow", "accessibility.arrow_bobbing", "accessibility.unit_rings",
    "accessibility.war.color", "accessibility.war.invert", "accessibility.allied.color", "accessibility.allied.invert",
    "accessibility.peace.color", "accessibility.peace.invert", "accessibility.ring_width", "accessibility.hud_size", "accessibility.stack_count",
];
pub fn range(key: &str) -> Option<Range> {
    match key {
        "accessibility.ring_width" => Some(Range { min: 1.0, max: 6.0, step: 1.0 }),
        "accessibility.hud_size" => Some(Range { min: 0.0, max: 3.0, step: 1.0 }),
        _ => None,
    }
}
impl Accessibility {
    pub fn hud_scale(&self) -> f32 { HUD_SCALES[self.hud_size.min(3)] }
    pub fn control(&self, key: &str) -> Option<ControlValue> {
        use ControlValue::*;
        Some(match key.strip_prefix("accessibility.")? {
            "active_unit_arrow" => On(self.active_unit_arrow), "arrow_bobbing" => On(self.arrow_bobbing),
            "unit_rings" => On(self.unit_rings), "stack_count" => On(self.stack_count),
            "ring_width" => Number(self.ring_width.clamp(1, 6) as f32), "hud_size" => Number(self.hud_size.min(3) as f32),
            relation => {
                let (name, field) = relation.split_once('.')?;
                let ring = match name { "war" => &self.war, "allied" => &self.allied, "peace" => &self.peace, _ => return None };
                match field {
                    "color" => Choice(format!("{:02x}{:02x}{:02x}", ring.color[0], ring.color[1], ring.color[2])),
                    "invert" => On(ring.invert), _ => return None,
                }
            }
        })
    }
    pub fn set_control(&mut self, key: &str, value: &ControlValue) -> bool {
        use ControlValue::*;
        let before = self.clone();
        match (key.strip_prefix("accessibility."), value) {
            (Some("active_unit_arrow"), On(v)) => self.active_unit_arrow = *v,
            (Some("arrow_bobbing"), On(v)) => self.arrow_bobbing = *v,
            (Some("unit_rings"), On(v)) => self.unit_rings = *v,
            (Some("stack_count"), On(v)) => self.stack_count = *v,
            (Some("ring_width"), Number(v)) => self.ring_width = range(key).unwrap().snap(*v) as u32,
            (Some("hud_size"), Number(v)) => self.hud_size = range(key).unwrap().snap(*v) as usize,
            (Some(relation), value) => {
                let Some((name, field)) = relation.split_once('.') else { return false };
                let ring = match name { "war" => &mut self.war, "allied" => &mut self.allied, "peace" => &mut self.peace, _ => return false };
                match (field, value) {
                    ("invert", On(v)) => ring.invert = *v,
                    ("color", Choice(v)) => {
                        let Some(rgb) = (v.len() == 6).then(|| u32::from_str_radix(v, 16).ok()).flatten() else { return false };
                        ring.color = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;
    #[test]
    fn defaults_old_files_round_trip_and_control_validation() {
        let mut s: Settings = toml::from_str("").unwrap();
        assert_eq!(s.accessibility, Accessibility::default());
        assert!(!s.accessibility.active_unit_arrow && s.accessibility.arrow_bobbing);
        assert!(!s.accessibility.unit_rings && !s.accessibility.stack_count);
        assert_eq!((s.accessibility.hud_scale(), s.accessibility.ring_width), (1.0, 2));
        for key in KEYS {
            let value = s.control(key).unwrap();
            let value = match value { ControlValue::On(v) => ControlValue::On(!v), ControlValue::Number(_) => ControlValue::Number(3.0), ControlValue::Choice(_) => ControlValue::Choice("20d0d8".into()) };
            assert!(s.set_control(key, &value), "{key}");
        }
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&s).unwrap()).unwrap(), s);
        assert!(!s.set_control("accessibility.war.color", &ControlValue::Choice("invalid".into())));
        s.set_control("accessibility.hud_size", &ControlValue::Number(f32::NAN));
        assert_eq!(s.accessibility.hud_scale(), 1.0);
    }
}
