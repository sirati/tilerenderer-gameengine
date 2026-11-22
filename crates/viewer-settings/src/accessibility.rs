//! Map aids, independent of the arrangement of the world views.
use serde::{Deserialize, Serialize};
use crate::controls::{ControlValue, Range};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    pub active_unit_arrow: bool,
    pub arrow_bobbing: bool,
    /// The arrow's size: an index into [`ARROW_SCALES`] (whole multiples
    /// of its art, so its edges stay smooth). Saved as `arrow_step`: older
    /// files' `arrow_size` indexed [`OLD_ARROW_SCALES`] ([`Accessibility::migrating`]).
    #[serde(rename = "arrow_step")]
    pub arrow_size: usize,
    pub unit_rings: bool,
    pub war: RingColor,
    pub allied: RingColor,
    pub peace: RingColor,
    pub ring_width: u32,
    /// Index into HUD_SCALES, rather than an arbitrary fractional size.
    pub hud_size: usize,
    pub stack_count: bool,
    /// The stack count on its dark translucent plate, as city labels;
    /// off, bare white with a dark outline.
    pub stack_backdrop: bool,
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
        Self { active_unit_arrow: false, arrow_bobbing: true, arrow_size: 0, unit_rings: false,
            war: RingColor::default(), allied: RingColor { color: [0, 64, 255], invert: false },
            peace: RingColor { color: [64, 240, 96], invert: false }, ring_width: 2, hud_size: 0, stack_count: false,
            stack_backdrop: true }
    }
}

pub const HUD_SCALES: [f32; 4] = [1.0, 1.5, 2.0, 3.0];
pub const ARROW_SCALES: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
/// The arrow sizes older files indexed (`arrow_size`), with fractional
/// sizes that painted its art's pixels unevenly.
pub const OLD_ARROW_SCALES: [f32; 5] = [1.0, 1.5, 2.0, 2.5, 3.0];
/// The compact palette shown beside each diplomatic relation.
pub const PALETTE: [[u8; 3]; 16] = [
    [255, 0, 0], [240, 112, 24], [248, 208, 32], [64, 240, 96],
    [0, 152, 112], [32, 208, 216], [0, 64, 255], [104, 64, 192],
    [200, 48, 160], [248, 144, 176], [144, 80, 40], [192, 168, 104],
    [255, 255, 255], [176, 176, 176], [80, 80, 80], [0, 0, 0],
];
pub const KEYS: [&str; 14] = [
    "accessibility.active_unit_arrow", "accessibility.arrow_bobbing", "accessibility.arrow_size", "accessibility.unit_rings",
    "accessibility.war.color", "accessibility.war.invert", "accessibility.allied.color", "accessibility.allied.invert",
    "accessibility.peace.color", "accessibility.peace.invert", "accessibility.ring_width", "accessibility.hud_size", "accessibility.stack_count",
    "accessibility.stack_backdrop",
];
pub fn range(key: &str) -> Option<Range> {
    match key {
        "accessibility.ring_width" => Some(Range { min: 1.0, max: 6.0, step: 1.0 }),
        "accessibility.hud_size" => Some(Range { min: 0.0, max: 3.0, step: 1.0 }),
        "accessibility.arrow_size" => Some(Range { min: 0.0, max: 3.0, step: 1.0 }),
        _ => None,
    }
}
impl Accessibility {
    /// Reads the accessibility table. An older file's `arrow_size` (an
    /// index into [`OLD_ARROW_SCALES`]) becomes the step of the nearest
    /// whole size, a half size rounding up (1.5 to 2); an explicit
    /// `arrow_step` wins.
    pub(crate) fn migrating<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Accessibility, D::Error> {
        let mut table = toml::Table::deserialize(d)?;
        let old = table.remove("arrow_size");
        let explicit = table.contains_key("arrow_step");
        let mut accessibility: Accessibility = toml::Value::Table(table).try_into().map_err(serde::de::Error::custom)?;
        if let (false, Some(toml::Value::Integer(old))) = (explicit, old) {
            let scale = OLD_ARROW_SCALES[usize::try_from(old).unwrap_or(0).min(OLD_ARROW_SCALES.len() - 1)];
            accessibility.arrow_size = ARROW_SCALES.iter().position(|&s| s >= scale.round()).unwrap_or(ARROW_SCALES.len() - 1);
        }
        Ok(accessibility)
    }
    pub fn hud_scale(&self) -> f32 { HUD_SCALES[self.hud_size.min(3)] }
    pub fn arrow_scale(&self) -> f32 { ARROW_SCALES[self.arrow_size.min(ARROW_SCALES.len() - 1)] }
    pub fn control(&self, key: &str) -> Option<ControlValue> {
        use ControlValue::*;
        Some(match key.strip_prefix("accessibility.")? {
            "active_unit_arrow" => On(self.active_unit_arrow), "arrow_bobbing" => On(self.arrow_bobbing),
            "unit_rings" => On(self.unit_rings), "stack_count" => On(self.stack_count),
            "stack_backdrop" => On(self.stack_backdrop),
            "ring_width" => Number(self.ring_width.clamp(1, 6) as f32), "hud_size" => Number(self.hud_size.min(3) as f32),
            "arrow_size" => Number(self.arrow_size.min(ARROW_SCALES.len() - 1) as f32),
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
            (Some("stack_backdrop"), On(v)) => self.stack_backdrop = *v,
            (Some("ring_width"), Number(v)) => self.ring_width = range(key).unwrap().snap(*v) as u32,
            (Some("hud_size"), Number(v)) => self.hud_size = range(key).unwrap().snap(*v) as usize,
            (Some("arrow_size"), Number(v)) => self.arrow_size = range(key).unwrap().snap(*v) as usize,
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
        assert!(s.accessibility.stack_backdrop, "the count on its plate, as before the option");
        let bare: Settings = toml::from_str("[accessibility]\nstack_backdrop = false\n").unwrap();
        assert!(!bare.accessibility.stack_backdrop && toml::from_str::<Settings>(&toml::to_string(&bare).unwrap()).unwrap() == bare);
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
        assert_eq!((s.accessibility.arrow_size, s.accessibility.arrow_scale()), (3, 4.0), "the arrow's size by its slider's steps");
        assert!(s.set_control("accessibility.arrow_size", &ControlValue::Number(2.0)));
        assert_eq!(s.accessibility.arrow_scale(), 3.0);
        assert!(s.set_control("accessibility.arrow_size", &ControlValue::Number(9.0)));
        assert_eq!(s.accessibility.arrow_scale(), 4.0);
    }

    #[test]
    fn an_older_files_fractional_arrow_size_becomes_the_nearest_whole_one() {
        let scale = |text: &str| toml::from_str::<Settings>(text).unwrap().accessibility.arrow_scale();
        // Old steps 1.0, 1.5, 2.0, 2.5, 3.0 (and past the end).
        for (old, whole) in [(0, 1.0), (1, 2.0), (2, 2.0), (3, 3.0), (4, 3.0), (9, 3.0)] {
            assert_eq!(scale(&format!("[accessibility]\narrow_size = {old}\nunit_rings = true\n")), whole, "old step {old}");
        }
        assert_eq!(scale("[accessibility]\narrow_size = 4\narrow_step = 1\n"), 2.0, "the new key wins");
        // Saved again, only the new key, read back the same.
        let s: Settings = toml::from_str("[accessibility]\narrow_size = 3\nunit_rings = true\n").unwrap();
        assert!(s.accessibility.unit_rings, "the rest of the table is read");
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("arrow_step = 2") && !text.contains("arrow_size"), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
    }
}
