//! Independent map-detail controls. Inherit follows the game's Clean Map
//! command, Normal always shows every category, and Custom hides selected
//! categories in just this view. Fog and player visibility are unaffected.

use serde::{Deserialize, Serialize};
use crate::controls::ControlValue;

pub const FLAGS: [(&str, &str, u32); 13] = [
    ("rivers", "Rivers / flood plains", 1 << 0),
    ("mountains", "Mountains / volcanoes", 1 << 1),
    ("hills", "Hills", 1 << 2),
    ("forests", "Forests / jungles / marsh", 1 << 3),
    ("buildings", "Terrain buildings", 1 << 4),
    ("roads", "Roads / railroads", 1 << 5),
    ("irrigation", "Irrigation", 1 << 6),
    ("resources", "Natural resources", 1 << 7),
    ("yields", "Food / shield icons", 1 << 8),
    ("pollution", "Pollution / craters", 1 << 9),
    ("boundaries", "Territory boundaries", 1 << 10),
    ("cities", "Cities and city labels", 1 << 11),
    ("units", "Units and unit labels", 1 << 12),
];
pub const RIVERS: u32 = 1 << 0;
pub const MOUNTAINS: u32 = 1 << 1;
pub const HILLS: u32 = 1 << 2;
pub const FORESTS: u32 = 1 << 3;
pub const BUILDINGS: u32 = 1 << 4;
pub const ROADS: u32 = 1 << 5;
pub const IRRIGATION: u32 = 1 << 6;
pub const RESOURCES: u32 = 1 << 7;
pub const YIELDS: u32 = 1 << 8;
pub const POLLUTION: u32 = 1 << 9;
pub const BOUNDARIES: u32 = 1 << 10;
pub const CITIES: u32 = 1 << 11;
pub const UNITS: u32 = 1 << 12;
pub const ALL: u32 = (1 << 13) - 1;
pub const VIEWS: [&str; 3] = ["a", "b", "zoom"];

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Inherit,
    Normal,
    Custom,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub mode: Mode,
    /// The same category order as the game's thirteen Clean Map checkboxes.
    pub hidden: u32,
}

impl View {
    pub fn mask(self, game: u32) -> u32 {
        (match self.mode { Mode::Inherit => game, Mode::Normal => 0, Mode::Custom => self.hidden }) & ALL
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanMap {
    pub a: View,
    pub b: View,
    pub zoom: View,
}

impl CleanMap {
    fn view(&self, key: &str) -> Option<&View> {
        match key { "a" => Some(&self.a), "b" => Some(&self.b), "zoom" => Some(&self.zoom), _ => None }
    }
    fn view_mut(&mut self, key: &str) -> Option<&mut View> {
        match key { "a" => Some(&mut self.a), "b" => Some(&mut self.b), "zoom" => Some(&mut self.zoom), _ => None }
    }
    pub fn keys() -> impl Iterator<Item = String> {
        VIEWS.into_iter().flat_map(|v| std::iter::once("mode").chain(FLAGS.iter().map(|f| f.0)).map(move |f| format!("clean_map.{v}.{f}")))
    }
    pub fn control(&self, key: &str) -> Option<ControlValue> {
        let (view, flag) = key.strip_prefix("clean_map.")?.split_once('.')?;
        let view = self.view(view)?;
        if flag == "mode" {
            return Some(ControlValue::Choice(match view.mode { Mode::Inherit => "Inherit", Mode::Normal => "Normal", Mode::Custom => "Custom" }.into()));
        }
        let bit = FLAGS.iter().find(|f| f.0 == flag)?.2;
        Some(ControlValue::On(view.hidden & bit != 0))
    }
    /// Changing a hide flag selects Custom for that view; other views keep
    /// their modes and flags. Invalid controls leave the settings alone.
    pub fn set_control(&mut self, key: &str, value: &ControlValue) -> bool {
        let Some((view, flag)) = key.strip_prefix("clean_map.").and_then(|k| k.split_once('.')) else { return false };
        let Some(view) = self.view_mut(view) else { return false };
        if flag == "mode" {
            let ControlValue::Choice(mode) = value else { return false };
            let mode = match mode.as_str() { "Inherit" => Mode::Inherit, "Normal" => Mode::Normal, "Custom" => Mode::Custom, _ => return false };
            let changed = view.mode != mode;
            view.mode = mode;
            return changed;
        }
        let (Some((_, _, bit)), ControlValue::On(on)) = (FLAGS.iter().find(|f| f.0 == flag), value) else { return false };
        let before = *view;
        view.mode = Mode::Custom;
        if *on { view.hidden |= bit; } else { view.hidden &= !bit; }
        *view != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;

    #[test]
    fn every_flag_is_independent_in_each_view_and_survives_save() {
        let mut settings = Settings::default();
        let old: Settings = toml::from_str("[layout]\nsplit = 'Vertical1To1'\n").unwrap();
        assert_eq!(old.clean_map, CleanMap::default());
        for view in VIEWS {
            for (flag, _, bit) in FLAGS {
                let key = format!("clean_map.{view}.{flag}");
                assert!(settings.set_control(&key, &ControlValue::On(true)));
                assert_eq!(settings.control(&key), Some(ControlValue::On(true)));
                assert_eq!(settings.clean_map.view(view).unwrap().mask(0), bit);
                for other in VIEWS.into_iter().filter(|v| *v != view) {
                    assert_eq!(settings.clean_map.view(other).unwrap().mask(0), 0);
                }
                let saved: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
                assert_eq!(saved, settings);
                assert!(settings.set_control(&key, &ControlValue::On(false)));
            }
        }
        assert_eq!(CleanMap::keys().count(), 42);
        assert!(!settings.set_control("clean_map.x.units", &ControlValue::On(true)));
        assert!(!settings.set_control("clean_map.a.missing", &ControlValue::On(true)));
    }

    #[test]
    fn inherited_normal_and_custom_are_distinct_without_changing_game_state() {
        for (_, _, bit) in FLAGS {
            assert_eq!(View::default().mask(bit), bit);
            assert_eq!(View { mode: Mode::Normal, hidden: ALL }.mask(bit), 0);
            assert_eq!(View { mode: Mode::Custom, hidden: bit }.mask(ALL), bit);
        }
        assert_eq!(View { mode: Mode::Custom, hidden: u32::MAX }.mask(0), ALL);
    }
}
