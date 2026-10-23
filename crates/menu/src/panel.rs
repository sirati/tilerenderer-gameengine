//! Panels: menus of controls bound to named values — choices (button groups
//! and tabs), checkboxes, sliders — with images and text, described in TOML.
//! A panel draws as run-time screen elements ([`crate::screen::Element`]:
//! regions of image files, fills, sprites of the render description) and
//! text in a named face ([`TextDraw`], drawn by the driver with its own text
//! path), and turns pointer input into what the user changed
//! ([`PanelEvent`]), telling which input it took ([`Routed::consumed`]) and
//! which belongs to what lies below.
//!
//! Game agnostic: what the values mean, and what a panel is drawn over, is
//! the driver's business. The panel only knows them by name.
//!
//! ```toml
//! [styles.label]                  # text: a face, a size, colours
//! face = "Lucida Sans"
//! height = 14                     # pixels
//! weight = 700                    # 400 normal (default), 700 bold
//! color = [0, 0, 0]
//! disabled_color = [110, 110, 110]  # optional, like selected_ and hover_color
//!
//! [art.button]                    # a region of an image file
//! image = "Art/buttons.pcx"
//! rect = [1, 1, 196, 90]          # x, y, w, h; a smaller item takes the
//!                                 # top-left part, a larger one repeats it
//! keyed = true                    # palette index 255 transparent (default)
//! border = [60, 50, 30]           # optional one-pixel outline
//! tint = [128, 128, 128, 120]     # optional translucent fill over it
//!
//! [art.bar]
//! image = "Art/background.pcx"
//! rect = [259, 551, 409, 25]
//! slice = [3, 3]                  # left and right ends kept at any width
//!
//! [art.shade]
//! fill = [0, 0, 0]                # a solid fill of the item's size
//! alpha = 96
//!
//! [art.knob]
//! sprite = "Knob/0"               # a sprite of the render description
//! size = [20, 20]
//!
//! [art.two_panes]                 # a picture made by the render description
//! generated = "panes:60,50,30,255:100x60:vertical11"
//! size = [100, 60]
//! labels = [{ text = "A", at = [25, 30], style = "icon" },
//!           { text = "B", at = [75, 30], style = "icon" }]
//! icons = [{ art = "knob", at = [75, 30] }]   # art centred on a point
//!
//! [panels.settings]
//! size = [1024, 768]
//! anchor = "center"               # within the area the driver gives
//!
//! [[panels.settings.items]]       # a tab: a choice like any other
//! choice = "tab"                  # the value it sets
//! value = "layout"                # to this
//! at = [280, 26]
//! size = [105, 32]
//! text = "Layout"
//! style = "tab"
//! normal = "button"               # art; also hover, selected, disabled
//! selected = "button_selected"
//!
//! [[panels.settings.items]]
//! image = "shade"                 # art
//! at = [47, 77]
//! size = [928, 615]
//! show_if = "tab=resolution|layout"   # drawn (and taking input) only then
//!
//! [[panels.settings.items]]
//! choice = "split"
//! value = "Vertical1To1"
//! at = [70, 115]
//! size = [104, 64]
//! normal = "button"
//! icon = "two_panes"              # art centred on it (or at `icon_at`)
//! enable_if = "mode!=simple"      # greyed out (`disabled` art) otherwise
//!
//! [[panels.settings.items]]
//! checkbox = "smooth"
//! at = [70, 460]
//! size = [300, 16]                # where it takes clicks: box and label
//! normal = "check_off"            # the box; `selected`: ticked
//! selected = "check_on"
//! text = "Smooth movement"        # right of the box (or at `text_at`)
//!
//! [[panels.settings.items]]
//! slider = "scale"
//! at = [200, 300]
//! size = [240, 25]                # the track
//! normal = "bar"                  # the track; `knob`, `knob_hover`
//! knob = "knob"
//! tick_step = 0.5                # ruler marks at multiples inside the track
//! tick_label_step = 1.0          # unitless labels at whole numbers
//! # tick_labels = ["1.5×", "2×", "3×", "4×"]  # optional, one per tick
//! tick_style = "caption"        # small text, clipped to the slider
//! min = 0.5
//! max = 3.0
//! step = 0.05
//! text = "{value}×"               # shown right of it (or at `value_at`)
//! decimals = 2                    # trailing zeros dropped; `percent = true`
//!
//! [[panels.settings.items]]
//! text = "Screen: {screen.size}"  # `{name}`: the value of `name`
//! at = [500, 200]
//! style = "label"
//! align = "left"                  # left, center, right of `at`
//! valign = "top"                  # top, middle, baseline
//!
//! [[panels.settings.items]]       # one item per entry: `{item}` replaced,
//! repeat = ["F1", "F2", "F3"]     # moved by `spacing` each
//! spacing = [0, 30]
//! choice = "screens.{item}"
//! value = "Floating"
//! at = [600, 120]
//! size = [105, 24]
//! normal = "button"
//! text = "Floating"
//! ```
//!
//! Conditions (`show_if`, `enable_if`): `name=a|b` (one of these), `name!=a|b`,
//! `name` (on, or a text that is not empty), `!name`; several joined by `&`.
//! Art names may hold `{name}` too (e.g. `icon = "panes_{split}"`).

use std::collections::{BTreeMap, BTreeSet};

use scene::{Button, Input};
use serde::Deserialize;

use crate::Anchor;
use crate::screen::{Element, Region};

#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error("TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("panel {panel:?}: unknown {kind} {name:?}")]
    Unknown { panel: String, kind: &'static str, name: String },
    #[error("{what}: {message}")]
    Invalid { what: String, message: String },
}

/// Horizontal placement of text relative to its point.
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Vertical placement of text relative to its point.
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Baseline,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleDesc {
    pub face: String,
    pub height: u32,
    #[serde(default)]
    pub weight: i32,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub color: [u8; 3],
    pub disabled_color: Option<[u8; 3]>,
    pub selected_color: Option<[u8; 3]>,
    pub hover_color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelDesc {
    pub text: String,
    pub at: [i32; 2],
    pub style: Option<String>,
    #[serde(default = "center")]
    pub align: Align,
    #[serde(default = "middle")]
    pub valign: VAlign,
}

fn center() -> Align {
    Align::Center
}

fn middle() -> VAlign {
    VAlign::Middle
}

fn yes() -> bool {
    true
}

fn opaque() -> u8 {
    255
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtDesc {
    /// A region (`rect`) of an image file, as the driver's loader names it.
    pub image: Option<String>,
    pub rect: Option<[i32; 4]>,
    #[serde(default = "yes")]
    pub keyed: bool,
    /// An alpha mask image, its pixel for a region pixel `mask_at` away.
    pub mask: Option<String>,
    pub mask_at: Option<[i32; 2]>,
    /// A sprite of the render description by name.
    pub sprite: Option<String>,
    /// A picture the render description makes (its `source`), see
    /// [`Panels::description`].
    pub generated: Option<String>,
    /// The size of a sprite or generated picture.
    pub size: Option<[u32; 2]>,
    /// A solid fill.
    pub fill: Option<[u8; 3]>,
    #[serde(default = "opaque")]
    pub alpha: u8,
    /// Left and right ends of a region kept at any width (the middle cut).
    pub slice: Option<[i32; 2]>,
    /// A translucent fill (RGBA) over the art.
    pub tint: Option<[u8; 4]>,
    /// A one-pixel outline around the art.
    pub border: Option<[u8; 3]>,
    #[serde(default)]
    pub labels: Vec<LabelDesc>,
    /// Other art drawn over it at its own size, centred on a point (e.g. an
    /// icon in each pane of a picture).
    #[serde(default)]
    pub icons: Vec<IconDesc>,
}

/// Art drawn over other art (`ArtDesc::icons`), its middle at `at`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconDesc {
    pub art: String,
    pub at: [i32; 2],
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDesc {
    pub image: Option<String>,
    pub choice: Option<String>,
    pub value: Option<String>,
    pub checkbox: Option<String>,
    pub slider: Option<String>,
    pub text: Option<String>,
    /// Hover text, drawn above the panel and in immediate hover looks.
    pub tooltip: Option<String>,
    pub tooltip_art: Option<String>,
    pub tooltip_style: Option<String>,
    #[serde(default)]
    pub at: [i32; 2],
    pub size: Option<[u32; 2]>,
    pub style: Option<String>,
    pub align: Option<Align>,
    pub valign: Option<VAlign>,
    pub normal: Option<String>,
    pub hover: Option<String>,
    pub selected: Option<String>,
    pub disabled: Option<String>,
    pub icon: Option<String>,
    pub icon_at: Option<[i32; 2]>,
    pub text_at: Option<[i32; 2]>,
    pub knob: Option<String>,
    pub knob_hover: Option<String>,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
    /// Ruler marks at multiples of this value, within the slider's range.
    pub tick_step: Option<f32>,
    /// Label only multiples of this value (defaults to `tick_step`). No units.
    pub tick_label_step: Option<f32>,
    /// Optional labels in tick order, e.g. magnifications for an index slider.
    pub tick_labels: Option<Vec<String>>,
    pub tick_style: Option<String>,
    pub decimals: Option<u32>,
    #[serde(default)]
    pub percent: bool,
    pub value_at: Option<[i32; 2]>,
    pub show_if: Option<String>,
    pub enable_if: Option<String>,
    pub repeat: Option<Vec<String>>,
    pub spacing: Option<[i32; 2]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelDesc {
    pub size: [u32; 2],
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub items: Vec<ItemDesc>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelsDesc {
    #[serde(default)]
    pub styles: BTreeMap<String, StyleDesc>,
    #[serde(default)]
    pub art: BTreeMap<String, ArtDesc>,
    #[serde(default)]
    pub panels: BTreeMap<String, PanelDesc>,
}

/// A face, a size and a colour.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextStyle {
    pub face: String,
    /// Pixels (a character height).
    pub height: u32,
    /// 0 or 400 normal, 700 bold.
    pub weight: i32,
    pub italic: bool,
    pub color: [u8; 3],
}

/// How large a run of text is (from the driver's fonts, see [`estimate`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextMetrics {
    pub width: i32,
    /// Above the baseline and below it.
    pub ascent: i32,
    pub descent: i32,
}

/// Metrics guessed from the size alone, for text whose font is not there.
pub fn estimate(style: &TextStyle, text: &str) -> TextMetrics {
    let h = style.height as i32;
    TextMetrics { width: text.chars().count() as i32 * h * 11 / 20, ascent: h * 4 / 5, descent: h - h * 4 / 5 }
}

/// Text to draw: its pen at (`x`, baseline `y`), screen pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDraw {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub style: TextStyle,
    /// Left, top, right, bottom (exclusive).
    pub clip: [i32; 4],
}

/// What a panel draws, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Element(Element),
    Text(TextDraw),
}

/// A named value a panel shows and sets.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    On(bool),
    Number(f32),
}

impl Value {
    fn matches(&self, s: &str) -> bool {
        match self {
            Value::Text(t) => t == s,
            Value::On(b) => s == if *b { "true" } else { "false" },
            Value::Number(n) => s.parse::<f32>().is_ok_and(|v| v == *n),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Text(t) => !t.is_empty() && t != "false",
            Value::On(b) => *b,
            Value::Number(n) => *n != 0.0,
        }
    }
}

/// What the user did on a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelEvent {
    /// Choice `control` set to `value`.
    Chose { control: String, value: String },
    /// Checkbox `control` ticked (or not).
    Toggled { control: String, on: bool },
    /// Slider `control` moved to `value` (on its steps, within its range).
    Slid { control: String, value: f32 },
}

/// What became of one input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Routed {
    /// The panel took it (the pointer is over it, or a press began on it);
    /// otherwise it belongs to what lies below.
    pub consumed: bool,
    pub events: Vec<PanelEvent>,
}

/// The values a panel shows and its pointer state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelState {
    pub values: BTreeMap<String, Value>,
    /// The control under the pointer, the one pressed, the slider dragged
    /// (item indices).
    pub hover: Option<usize>,
    pub pressed: Option<usize>,
    pub dragging: Option<usize>,
    /// Mouse buttons (bit per [`Button`]) held since a press the panel took,
    /// and since one it left to what is below (their releases go the same
    /// way, and the pointer's moves go below while one of those is held).
    pub ours: u8,
    pub theirs: u8,
    /// Alt slider transaction: control id, item index, exact initial value.
    pub slider_edit: Option<(u64, usize, f32)>,
}

impl PanelState {
    /// Cancels an Alt slider edit, even after its control stops being visible.
    pub fn cancel_slider(&mut self, panel: &Panel) -> Option<PanelEvent> {
        let (_, index, original) = self.slider_edit.take()?;
        let control = panel.items.get(index)?.control()?.to_owned();
        let changed = self.values.get(&control) != Some(&Value::Number(original));
        self.values.insert(control.clone(), Value::Number(original));
        changed.then_some(PanelEvent::Slid { control, value: original })
    }
}

/// One condition of `show_if` / `enable_if`.
#[derive(Clone, Debug, PartialEq)]
struct Test {
    key: String,
    negate: bool,
    /// `None`: whether the value is on.
    values: Option<Vec<String>>,
}

impl Test {
    fn holds(&self, values: &BTreeMap<String, Value>) -> bool {
        let v = values.get(&self.key);
        let yes = match &self.values {
            None => v.is_some_and(Value::truthy),
            Some(options) => v.is_some_and(|v| options.iter().any(|o| v.matches(o))),
        };
        yes != self.negate
    }
}

/// Conditions joined by `&`.
#[derive(Clone, Debug, PartialEq)]
struct Cond(Vec<Test>);

impl Cond {
    fn parse(s: &str) -> Result<Cond, String> {
        let tests = s
            .split('&')
            .map(|t| {
                let t = t.trim();
                let (key, negate, values) = if let Some((k, v)) = t.split_once("!=") {
                    (k, true, Some(v))
                } else if let Some((k, v)) = t.split_once('=') {
                    (k, false, Some(v))
                } else if let Some(k) = t.strip_prefix('!') {
                    (k, true, None)
                } else {
                    (t, false, None)
                };
                let key = key.trim();
                if key.is_empty() {
                    return Err(format!("condition {s:?}: a test without a name"));
                }
                Ok(Test { key: key.to_owned(), negate, values: values.map(|v| v.split('|').map(|o| o.trim().to_owned()).collect()) })
            })
            .collect::<Result<_, _>>()?;
        Ok(Cond(tests))
    }

    fn holds(&self, values: &BTreeMap<String, Value>) -> bool {
        self.0.iter().all(|t| t.holds(values))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ArtKind {
    None,
    Region(Region),
    Sprite(String),
    Fill { rgb: [u8; 3], alpha: u8 },
}

#[derive(Clone, Debug, PartialEq)]
struct Label {
    text: String,
    at: (i32, i32),
    style: Option<String>,
    align: Align,
    valign: VAlign,
}

#[derive(Clone, Debug, PartialEq)]
struct Art {
    kind: ArtKind,
    /// Its own size, if it has one (a region's, a sprite's).
    size: Option<(u32, u32)>,
    slice: Option<(i32, i32)>,
    tint: Option<[u8; 4]>,
    border: Option<[u8; 3]>,
    labels: Vec<Label>,
    /// Other art drawn over it, centred on these points (`icons`).
    icons: Vec<(String, (i32, i32))>,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Image(String),
    Text,
    Choice { control: String, value: String },
    Checkbox { control: String },
    Slider { control: String, min: f32, max: f32, step: f32, knob: Option<String>, knob_hover: Option<String>, ticks: Vec<(f32, bool)>, tick_labels: Option<Vec<String>>, tick_style: Option<String> },
}

#[derive(Clone, Debug, PartialEq)]
struct Item {
    kind: Kind,
    at: (i32, i32),
    size: Option<(u32, u32)>,
    text: Option<String>,
    tooltip: Option<String>,
    tooltip_art: Option<String>,
    tooltip_style: Option<String>,
    style: Option<String>,
    align: Option<Align>,
    valign: Option<VAlign>,
    normal: Option<String>,
    hover: Option<String>,
    selected: Option<String>,
    disabled: Option<String>,
    icon: Option<String>,
    icon_at: Option<(i32, i32)>,
    text_at: Option<(i32, i32)>,
    decimals: u32,
    percent: bool,
    value_at: Option<(i32, i32)>,
    show_if: Option<Cond>,
    enable_if: Option<Cond>,
}

impl Item {
    fn control(&self) -> Option<&str> {
        match &self.kind {
            Kind::Choice { control, .. } | Kind::Checkbox { control } | Kind::Slider { control, .. } => Some(control),
            Kind::Image(_) | Kind::Text => None,
        }
    }
}

/// A panel resolved: its items in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub name: String,
    size: (u32, u32),
    anchor: Anchor,
    items: Vec<Item>,
    content_controls: BTreeSet<String>,
}

impl Panel {
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The range and step of slider `control` (min, max, step), if it is one.
    pub fn slider_range(&self, control: &str) -> Option<(f32, f32, f32)> {
        self.items.iter().find_map(|i| match &i.kind {
            Kind::Slider { control: c, min, max, step, .. } if c == control => Some((*min, *max, *step)),
            _ => None,
        })
    }

    /// The controls of the panel (their value names), each once.
    pub fn controls(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for c in self.items.iter().filter_map(Item::control) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }
}

/// Panels resolved: styles, art and panels checked against each other.
#[derive(Clone, Debug, PartialEq)]
pub struct Panels {
    styles: BTreeMap<String, StyleDesc>,
    art: BTreeMap<String, Art>,
    /// Pictures the render description makes: art name, source, size.
    generated: Vec<(String, String, (u32, u32))>,
    panels: BTreeMap<String, Panel>,
}

/// The sprite set of generated art `name` (see [`Panels::description`]).
pub fn generated_sprite(name: &str) -> String {
    format!("Panel@{name}/0")
}

/// `s` with every `{key}` replaced by `replace(key)` (unknown: nothing).
fn substitute(s: &str, mut replace: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open + 1..].find('}') {
            Some(close) => {
                out.push_str(&replace(&rest[open + 1..open + 1 + close]).unwrap_or_default());
                rest = &rest[open + 1 + close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `v` as text: `decimals` places at most (trailing zeros dropped), as a
/// percentage with `percent`.
pub fn format_number(v: f32, decimals: u32, percent: bool) -> String {
    let v = if percent { v * 100.0 } else { v };
    let s = format!("{v:.*}", decimals as usize);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    if s == "-0" { "0".into() } else { s }
}

/// An item with `{item}` replaced by `entry` everywhere.
fn repeated(desc: &ItemDesc, entry: &str) -> ItemDesc {
    let s = |o: &Option<String>| o.as_ref().map(|v| v.replace("{item}", entry));
    ItemDesc {
        image: s(&desc.image),
        choice: s(&desc.choice),
        value: s(&desc.value),
        checkbox: s(&desc.checkbox),
        slider: s(&desc.slider),
        text: s(&desc.text),
        tooltip: s(&desc.tooltip),
        tooltip_art: s(&desc.tooltip_art),
        tooltip_style: s(&desc.tooltip_style),
        style: s(&desc.style),
        tick_style: s(&desc.tick_style),
        normal: s(&desc.normal),
        hover: s(&desc.hover),
        selected: s(&desc.selected),
        disabled: s(&desc.disabled),
        icon: s(&desc.icon),
        knob: s(&desc.knob),
        knob_hover: s(&desc.knob_hover),
        show_if: s(&desc.show_if),
        enable_if: s(&desc.enable_if),
        repeat: None,
        spacing: None,
        ..desc.clone()
    }
}

impl Panels {
    pub fn parse(text: &str) -> Result<Self, PanelError> {
        let desc: PanelsDesc = toml::from_str(text)?;
        Self::new(&desc)
    }

    pub fn new(desc: &PanelsDesc) -> Result<Self, PanelError> {
        let invalid = |what: String, message: &str| PanelError::Invalid { what, message: message.to_owned() };
        let mut art = BTreeMap::new();
        let mut generated = Vec::new();
        for (name, a) in &desc.art {
            let what = || format!("art {name:?}");
            let kinds = [a.image.is_some(), a.sprite.is_some(), a.generated.is_some(), a.fill.is_some()].iter().filter(|&&k| k).count();
            if kinds > 1 {
                return Err(invalid(what(), "one of image, sprite, generated, fill"));
            }
            let size = a.size.map(|[w, h]| (w, h));
            let (kind, size) = if let Some(image) = &a.image {
                let rect = a.rect.ok_or_else(|| invalid(what(), "an image needs a rect"))?;
                if rect[2] <= 0 || rect[3] <= 0 {
                    return Err(invalid(what(), "an empty rect"));
                }
                let mask = a.mask.as_ref().map(|m| (m.clone(), a.mask_at.map_or((0, 0), |[x, y]| (x, y))));
                let region = Region { source: image.clone(), rect, keyed: a.keyed, mask, palette: None };
                (ArtKind::Region(region), Some((rect[2] as u32, rect[3] as u32)))
            } else if let Some(sprite) = &a.sprite {
                (ArtKind::Sprite(sprite.clone()), size)
            } else if let Some(source) = &a.generated {
                let s = size.ok_or_else(|| invalid(what(), "generated art needs its size"))?;
                generated.push((name.clone(), source.clone(), s));
                (ArtKind::Sprite(generated_sprite(name)), size)
            } else if let Some(rgb) = a.fill {
                (ArtKind::Fill { rgb, alpha: a.alpha }, size)
            } else {
                (ArtKind::None, size)
            };
            for l in &a.labels {
                if let Some(s) = l.style.as_ref().filter(|s| !desc.styles.contains_key(*s)) {
                    return Err(invalid(what(), &format!("unknown style {s:?}")));
                }
            }
            let labels = a
                .labels
                .iter()
                .map(|l| Label { text: l.text.clone(), at: (l.at[0], l.at[1]), style: l.style.clone(), align: l.align, valign: l.valign })
                .collect();
            // (Icons are art of their own size, with no icons of theirs.)
            for i in &a.icons {
                match desc.art.get(&i.art) {
                    None => return Err(invalid(what(), &format!("unknown icon art {:?}", i.art))),
                    Some(icon) if !icon.icons.is_empty() => return Err(invalid(what(), &format!("icon art {:?} has icons itself", i.art))),
                    Some(icon) if icon.size.is_none() && icon.rect.is_none() => return Err(invalid(what(), &format!("icon art {:?} has no size", i.art))),
                    Some(_) => {}
                }
            }
            let icons = a.icons.iter().map(|i| (i.art.clone(), (i.at[0], i.at[1]))).collect();
            art.insert(name.clone(), Art { kind, size, slice: a.slice.map(|[l, r]| (l, r)), tint: a.tint, border: a.border, labels, icons });
        }
        let mut panels = BTreeMap::new();
        for (name, p) in &desc.panels {
            let unknown = |kind: &'static str, n: &str| PanelError::Unknown { panel: name.clone(), kind, name: n.to_owned() };
            // Art names with `{…}` are resolved when drawn.
            let art_ref = |n: &Option<String>| -> Result<Option<String>, PanelError> {
                match n {
                    Some(n) if !n.contains('{') && !art.contains_key(n) => Err(unknown("art", n)),
                    other => Ok(other.clone()),
                }
            };
            let mut items = Vec::new();
            let expanded: Vec<(ItemDesc, i32)> = p
                .items
                .iter()
                .flat_map(|d| match &d.repeat {
                    Some(entries) => entries.iter().enumerate().map(|(i, e)| (repeated(d, e), i as i32)).collect::<Vec<_>>(),
                    None => vec![(d.clone(), 0)],
                })
                .zip(p.items.iter().flat_map(|d| std::iter::repeat_n(d.spacing.unwrap_or([0, 0]), d.repeat.as_ref().map_or(1, Vec::len))))
                .map(|((d, i), [sx, sy])| (ItemDesc { at: [d.at[0] + sx * i, d.at[1] + sy * i], ..d }, i))
                .collect();
            for (index, (d, _)) in expanded.iter().enumerate() {
                let what = || format!("panel {name:?} item {index}");
                let kinds = [d.image.is_some(), d.choice.is_some(), d.checkbox.is_some(), d.slider.is_some()].iter().filter(|&&k| k).count();
                if kinds > 1 {
                    return Err(invalid(what(), "one of image, choice, checkbox, slider"));
                }
                let kind = if let Some(image) = &d.image {
                    Kind::Image(art_ref(&Some(image.clone()))?.unwrap_or_default())
                } else if let Some(control) = &d.choice {
                    let value = d.value.clone().ok_or_else(|| invalid(what(), "a choice needs its value"))?;
                    if d.size.is_none() {
                        return Err(invalid(what(), "a choice needs its size"));
                    }
                    Kind::Choice { control: control.clone(), value }
                } else if let Some(control) = &d.checkbox {
                    Kind::Checkbox { control: control.clone() }
                } else if let Some(control) = &d.slider {
                    let (min, max) = (d.min.unwrap_or(0.0), d.max.unwrap_or(1.0));
                    let step = d.step.unwrap_or(0.0);
                    if !(min < max) || step < 0.0 || d.size.is_none() {
                        return Err(invalid(what(), "a slider needs its size and min < max (step >= 0)"));
                    }
                    let mut ticks = Vec::new();
                    if let Some(tick_step) = d.tick_step {
                        let label_step = d.tick_label_step.unwrap_or(tick_step);
                        if !tick_step.is_finite() || tick_step <= 0.0 || !label_step.is_finite() || label_step <= 0.0
                            || !min.is_finite() || !max.is_finite() || (max - min) / tick_step > 1000.0 {
                            return Err(invalid(what(), "finite positive tick steps with at most 1001 marks"));
                        }
                        for k in (min / tick_step).ceil() as i32..=(max / tick_step).floor() as i32 {
                            let v = k as f32 * tick_step;
                            let label = (v / label_step - (v / label_step).round()).abs() < 0.0001;
                            ticks.push((v, label));
                        }
                    }
                    if d.tick_labels.as_ref().is_some_and(|labels| labels.len() != ticks.len()) {
                        return Err(invalid(what(), "one custom label per ruler tick"));
                    }
                    if let Some(style) = d.tick_style.as_ref().filter(|s| !desc.styles.contains_key(*s)) {
                        return Err(unknown("style", style));
                    }
                    Kind::Slider { control: control.clone(), min, max, step, knob: art_ref(&d.knob)?, knob_hover: art_ref(&d.knob_hover)?, ticks, tick_labels: d.tick_labels.clone(), tick_style: d.tick_style.clone() }
                } else if d.text.is_some() {
                    Kind::Text
                } else {
                    return Err(invalid(what(), "an image, choice, checkbox, slider or text"));
                };
                if let Some(s) = d.style.as_ref().filter(|s| !desc.styles.contains_key(*s)) {
                    return Err(unknown("style", s));
                }
                if let Some(s) = d.tooltip_style.as_ref().filter(|s| !desc.styles.contains_key(*s)) {
                    return Err(unknown("style", s));
                }
                let cond = |c: &Option<String>| c.as_deref().map(Cond::parse).transpose().map_err(|m| invalid(what(), &m));
                let pair = |p: Option<[i32; 2]>| p.map(|[x, y]| (x, y));
                items.push(Item {
                    kind,
                    at: (d.at[0], d.at[1]),
                    size: d.size.map(|[w, h]| (w, h)),
                    text: d.text.clone(),
                    tooltip: d.tooltip.clone(),
                    tooltip_art: art_ref(&d.tooltip_art)?,
                    tooltip_style: d.tooltip_style.clone(),
                    style: d.style.clone(),
                    align: d.align,
                    valign: d.valign,
                    normal: art_ref(&d.normal)?,
                    hover: art_ref(&d.hover)?,
                    selected: art_ref(&d.selected)?,
                    disabled: art_ref(&d.disabled)?,
                    icon: art_ref(&d.icon)?,
                    icon_at: pair(d.icon_at),
                    text_at: pair(d.text_at),
                    decimals: d.decimals.unwrap_or(2),
                    percent: d.percent,
                    value_at: pair(d.value_at),
                    show_if: cond(&d.show_if)?,
                    enable_if: cond(&d.enable_if)?,
                });
            }
            let content_controls = items.iter().flat_map(|item| [&item.show_if, &item.enable_if])
                .flatten().flat_map(|condition| condition.0.iter().map(|test| test.key.clone())).collect();
            panels.insert(name.clone(), Panel { name: name.clone(), size: (p.size[0], p.size[1]), anchor: p.anchor, items, content_controls });
        }
        Ok(Self { styles: desc.styles.clone(), art, generated, panels })
    }

    pub fn panel(&self, name: &str) -> Option<&Panel> {
        self.panels.get(name)
    }

    /// Render description entries for the generated art: sprite set
    /// [`generated_sprite`] per picture.
    pub fn description(&self) -> String {
        let mut out = String::from("\n# Panel art (generated).\n");
        for (n, (name, source, (w, h))) in self.generated.iter().enumerate() {
            let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
            out.push_str(&format!(
                "[images.panel_art_{n}]\nsource = {}\n\n[sheets.panel_art_{n}]\nimage = \"panel_art_{n}\"\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n\n",
                q(source),
                q(generated_sprite(name).trim_end_matches("/0"))
            ));
        }
        out
    }

    /// Every style in every colour it is drawn in (to load their fonts
    /// before drawing).
    pub fn text_styles(&self) -> Vec<TextStyle> {
        let mut out = Vec::new();
        for s in self.styles.values() {
            for color in [Some(s.color), s.disabled_color, s.selected_color, s.hover_color].into_iter().flatten() {
                let style = TextStyle { face: s.face.clone(), height: s.height, weight: s.weight, italic: s.italic, color };
                if !out.contains(&style) {
                    out.push(style);
                }
            }
        }
        out
    }

    /// Top-left of `panel` placed in `area` (x, y, w, h).
    pub fn origin(&self, panel: &Panel, area: [i32; 4]) -> (i32, i32) {
        let (dx, dy) = panel.anchor.place(panel.size, (area[2].max(0) as u32, area[3].max(0) as u32));
        (area[0] + dx, area[1] + dy)
    }

    fn visible(item: &Item, state: &PanelState) -> bool {
        item.show_if.as_ref().is_none_or(|c| c.holds(&state.values))
    }

    fn enabled(item: &Item, state: &PanelState) -> bool {
        item.enable_if.as_ref().is_none_or(|c| c.holds(&state.values))
    }

    /// An item's own size: given, or its art's.
    fn item_size(&self, item: &Item, state: &PanelState) -> Option<(u32, u32)> {
        item.size.or_else(|| {
            let art = match &item.kind {
                Kind::Image(a) => Some(a.clone()),
                _ => item.normal.clone(),
            }?;
            self.art_named(&art, state)?.size
        })
    }

    /// Art `name` with `{…}` replaced by the values.
    fn art_named(&self, name: &str, state: &PanelState) -> Option<&Art> {
        let name = substitute(name, |k| state.values.get(k).map(|v| value_text(v, 2, false)));
        self.art.get(&name)
    }

    /// The window rectangle (x, y, w, h) of `item`, if it covers any.
    fn rect(&self, item: &Item, state: &PanelState, (ox, oy): (i32, i32)) -> Option<[i32; 4]> {
        if matches!(item.kind, Kind::Text) {
            return None;
        }
        let (w, h) = self.item_size(item, state)?;
        Some([ox + item.at.0, oy + item.at.1, w as i32, h as i32])
    }

    /// Whether the panel covers window point (x, y) (an item drawn there
    /// that is no text).
    pub fn covers(&self, panel: &Panel, state: &PanelState, area: [i32; 4], x: i32, y: i32) -> bool {
        let origin = self.origin(panel, area);
        panel.items.iter().filter(|i| Self::visible(i, state)).filter_map(|i| self.rect(i, state, origin)).any(|r| inside(r, x, y))
    }

    /// Whether the panel intercepts any part of a rectangle. A driver
    /// excludes controls below that rectangle from button snapping, so a
    /// pointer cannot snap through a panel's painted cover.
    pub fn covers_any(&self, panel: &Panel, state: &PanelState, area: [i32; 4], [x, y, w, h]: [i32; 4]) -> bool {
        if w <= 0 || h <= 0 {
            return false;
        }
        let origin = self.origin(panel, area);
        panel.items.iter().filter(|i| Self::visible(i, state)).filter_map(|i| self.rect(i, state, origin)).any(|[ix, iy, iw, ih]| {
            iw > 0 && ih > 0 && x < ix + iw && ix < x + w && y < iy + ih && iy < y + h
        })
    }

    /// The control (item index) under window point (x, y) that can be
    /// used.
    fn hit(&self, panel: &Panel, state: &PanelState, area: [i32; 4], x: i32, y: i32) -> Option<usize> {
        let origin = self.origin(panel, area);
        panel.items.iter().enumerate().rev().find_map(|(n, i)| {
            let usable = i.control().is_some() && Self::visible(i, state) && Self::enabled(i, state);
            (usable && self.rect(i, state, origin).is_some_and(|r| inside(r, x, y))).then_some(n)
        })
    }

    /// The value slider `item` would take at window x, if it changes.
    fn slide(&self, panel: &Panel, state: &PanelState, area: [i32; 4], index: usize, x: i32) -> Option<(String, f32)> {
        let item = &panel.items[index];
        let Kind::Slider { control, min, max, step, knob, .. } = &item.kind else { return None };
        let origin = self.origin(panel, area);
        let [tx, _, tw, _] = self.rect(item, state, origin)?;
        let kw = knob.as_ref().and_then(|k| self.art_named(k, state)).and_then(|a| a.size).map_or(0, |s| s.0 as i32);
        let span = (tw - kw).max(1) as f32;
        let t = ((x - tx - kw / 2) as f32 / span).clamp(0.0, 1.0);
        let v = snap(min + t * (max - min), *min, *max, *step);
        let now = match state.values.get(control) {
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        };
        (now != Some(v)).then(|| (control.clone(), v))
    }

    /// Takes one input: updates hover, presses and drags, the values
    /// changed (in `state.values`), and says what changed and whether the
    /// input was the panel's.
    pub fn input(&self, panel: &Panel, state: &mut PanelState, area: [i32; 4], input: &Input) -> Routed {
        let bit = |b: Button| match b {
            Button::Left => 1u8,
            Button::Right => 2,
            Button::Middle => 4,
        };
        let mut events = Vec::new();
        if let Some((_, index, _)) = state.slider_edit {
            let valid = panel.items.get(index).is_some_and(|i| Self::visible(i, state) && Self::enabled(i, state));
            let ordinary_press = matches!(input, Input::MouseButton { down: true, .. });
            if !valid || ordinary_press {
                events.extend(state.cancel_slider(panel));
            }
        }
        let consumed = match *input {
            Input::Slider { id, action, value } => {
                use scene::SliderAction;
                let index = panel.items.iter().enumerate().find_map(|(n, i)| {
                    (id_hash(&panel.name, &n.to_string()) == id && matches!(i.kind, Kind::Slider { .. })).then_some(n)
                });
                if let Some(index) = index {
                    if action == SliderAction::Begin {
                        events.extend(state.cancel_slider(panel));
                        let item = &panel.items[index];
                        if Self::visible(item, state) && Self::enabled(item, state) {
                            let Kind::Slider { control, min, .. } = &item.kind else { unreachable!() };
                            let original = match state.values.get(control) { Some(Value::Number(v)) => *v, _ => *min };
                            state.slider_edit = Some((id, index, original));
                            state.hover = Some(index);
                        }
                    } else if action == SliderAction::Step {
                        // A keyboard step: inside an Alt edit when one is
                        // open on it (its Cancel restores), else on its own.
                        if state.slider_edit.is_some_and(|edit| edit.0 != id) {
                            events.extend(state.cancel_slider(panel));
                        }
                        let item = &panel.items[index];
                        if Self::visible(item, state) && Self::enabled(item, state) {
                            events.extend(step_slider(item, state, value));
                        }
                    } else if state.slider_edit.is_some_and(|edit| edit.0 == id) {
                        match action {
                            SliderAction::Cancel => events.extend(state.cancel_slider(panel)),
                            SliderAction::Commit => { state.slider_edit = None; }
                            SliderAction::Preview => {
                                if let Kind::Slider { control, min, max, step, .. } = &panel.items[index].kind {
                                    let value = snap(min + (max - min) * value.clamp(0, 1_000_000) as f32 / 1_000_000.0, *min, *max, *step);
                                    if state.values.get(control) != Some(&Value::Number(value)) {
                                        state.values.insert(control.clone(), Value::Number(value));
                                        events.push(PanelEvent::Slid { control: control.clone(), value });
                                    }
                                }
                            }
                            SliderAction::Begin | SliderAction::Step => unreachable!(),
                        }
                    }
                    true
                } else { false }
            }
            Input::MouseMove { x, y } => {
                if let Some(i) = state.dragging {
                    if let Some((control, value)) = self.slide(panel, state, area, i, x) {
                        state.values.insert(control.clone(), Value::Number(value));
                        events.push(PanelEvent::Slid { control, value });
                    }
                    true
                } else {
                    state.hover = if state.theirs == 0 { self.hit(panel, state, area, x, y) } else { None };
                    state.theirs == 0 && (state.ours != 0 || self.covers(panel, state, area, x, y))
                }
            }
            Input::MouseButton { button, down: true, x, y } => {
                if state.theirs & bit(button) == 0 && (state.ours != 0 || self.covers(panel, state, area, x, y)) {
                    state.ours |= bit(button);
                    if button == Button::Left {
                        state.hover = self.hit(panel, state, area, x, y);
                        state.pressed = state.hover;
                        if let Some(i) = state.pressed.filter(|&i| matches!(panel.items[i].kind, Kind::Slider { .. })) {
                            state.dragging = Some(i);
                            if let Some((control, value)) = self.slide(panel, state, area, i, x) {
                                state.values.insert(control.clone(), Value::Number(value));
                                events.push(PanelEvent::Slid { control, value });
                            }
                        }
                    }
                    true
                } else {
                    state.theirs |= bit(button);
                    false
                }
            }
            Input::MouseButton { button, down: false, x, y } => {
                if state.ours & bit(button) != 0 {
                    state.ours &= !bit(button);
                    if button == Button::Left {
                        let over = self.hit(panel, state, area, x, y);
                        let pressed = state.pressed.take();
                        if state.dragging.take().is_none() && pressed.is_some() && pressed == over {
                            events.extend(self.activate(panel, state, pressed.unwrap_or_default()));
                        }
                        state.hover = over;
                    }
                    true
                } else {
                    state.theirs &= !bit(button);
                    false
                }
            }
            Input::Wheel { delta, x, y } => {
                let ours = state.theirs == 0 && self.covers(panel, state, area, x, y);
                if let Some(i) = self.hit(panel, state, area, x, y).filter(|_| ours) {
                    events.extend(step_slider(&panel.items[i], state, delta.signum()));
                }
                ours
            }
            _ => false,
        };
        Routed { consumed, events }
    }

    /// A click on control `index`.
    fn activate(&self, panel: &Panel, state: &mut PanelState, index: usize) -> Option<PanelEvent> {
        match &panel.items[index].kind {
            Kind::Choice { control, value } => {
                let now = state.values.get(control);
                if now.is_some_and(|v| v.matches(value)) {
                    return None;
                }
                state.values.insert(control.clone(), Value::Text(value.clone()));
                Some(PanelEvent::Chose { control: control.clone(), value: value.clone() })
            }
            Kind::Checkbox { control } => {
                let on = !state.values.get(control).is_some_and(Value::truthy);
                state.values.insert(control.clone(), Value::On(on));
                Some(PanelEvent::Toggled { control: control.clone(), on })
            }
            _ => None,
        }
    }

    fn style(&self, name: Option<&str>) -> Option<&StyleDesc> {
        name.and_then(|n| self.styles.get(n)).or_else(|| self.styles.get("default"))
    }

    /// Draws `panel` placed in `area` (x, y, w, h), measuring text with
    /// `measure` (see [`estimate`]).
    pub fn draw(&self, panel: &Panel, state: &PanelState, area: [i32; 4], measure: &dyn Fn(&TextStyle, &str) -> TextMetrics) -> Vec<Draw> {
        let mut out = Vec::new();
        let tooltip = state.hover.filter(|&n| panel.items.get(n).is_some_and(|i| i.tooltip.is_some() && Self::visible(i, state) && Self::enabled(i, state)));
        for (n, item) in panel.items.iter().enumerate().filter(|(_, i)| Self::visible(i, state)) {
            if tooltip == Some(n) { continue; }
            out.extend(self.draw_item(panel, state, area, measure, n, item));
        }
        if let Some(n) = tooltip {
            let item = &panel.items[n];
            // Keep the hovered control and tooltip together on top. The
            // viewer replaces one contiguous sprite range for each look.
            out.extend(self.draw_item(panel, state, area, measure, n, item));
            out.extend(self.tooltip(panel, state, area, measure, item));
        }
        out
    }

    fn tooltip(&self, panel: &Panel, state: &PanelState, area: [i32; 4], measure: &dyn Fn(&TextStyle, &str) -> TextMetrics, item: &Item) -> Vec<Draw> {
        let Some(text) = &item.tooltip else { return Vec::new() };
        let style = item.tooltip_style.as_deref().or(item.style.as_deref());
        let Some(desc) = self.style(style) else { return Vec::new() };
        let (ox, oy) = self.origin(panel, area);
        let clip = [ox, oy, ox + panel.size.0 as i32, oy + panel.size.1 as i32];
        let mut out = Vec::new();
        let mut d = Drawer { panels: self, state, clip, measure, out: &mut out };
        let text = d.values_text(text, item.decimals, item.percent);
        let metrics = measure(&TextStyle { face: desc.face.clone(), height: desc.height, weight: desc.weight, italic: desc.italic, color: desc.color }, &text);
        let (w, h) = ((metrics.width + 12).min(clip[2] - ox), (metrics.ascent + metrics.descent + 10).min(clip[3] - oy));
        let [x, y, _, ih] = self.rect(item, state, (ox, oy)).unwrap_or([ox, oy, 0, 0]);
        let x = x.clamp(ox, clip[2] - w);
        let y = if y + ih + 5 + h <= clip[3] { y + ih + 5 } else { (y - h - 5).max(oy) };
        if let Some(art) = &item.tooltip_art { d.art(art, (x, y), Some((w as u32, h as u32))); }
        d.text(&text, style, TextLook { enabled: true, selected: false, hover: false }, (x + 6, y + 5), Align::Left, VAlign::Top);
        out
    }

    /// The panel's controls as things to point at, with their draws as
    /// they are and in their other states: under the pointer or not,
    /// selected or not (a choice's option chosen, a check box on), for a
    /// renderer to draw those itself at once (`scene::Looks`). Ids are
    /// stable per panel and item, clear of a game's object addresses.
    pub fn looks(&self, panel: &Panel, state: &PanelState, area: [i32; 4], measure: &dyn Fn(&TextStyle, &str) -> TextMetrics) -> Vec<ControlLooks> {
        use scene::{LookState, PointerState};
        let origin = self.origin(panel, area);
        let mut out = Vec::new();
        for (n, item) in panel.items.iter().enumerate().rev() {
            let usable = item.control().is_some() && Self::visible(item, state) && Self::enabled(item, state);
            let Some(rect) = self.rect(item, state, origin).filter(|_| usable) else { continue };
            let (select, selected) = match &item.kind {
                Kind::Choice { control, value } => (scene::Select::Pick { group: id_hash(&panel.name, control) }, state.values.get(control).is_some_and(|v| v.matches(value))),
                Kind::Checkbox { control } => (scene::Select::Toggle, state.values.get(control).is_some_and(Value::truthy)),
                _ => (scene::Select::None, false),
            };
            // The item drawn with the pointer over it or not, selected or not.
            let draws_in = |look: LookState| {
                let mut s = state.clone();
                s.hover = (look.pointer != PointerState::Normal).then_some(n);
                s.pressed = (look.pointer == PointerState::Pressed).then_some(n);
                match &item.kind {
                    Kind::Choice { control, value } => {
                        let other = if look.selected { value.clone() } else { format!("{value}\u{0}") };
                        s.values.insert(control.clone(), Value::Text(other));
                    }
                    Kind::Checkbox { control } => {
                        s.values.insert(control.clone(), Value::On(look.selected));
                    }
                    _ => {}
                }
                let mut draws = self.draw_item(panel, &s, area, measure, n, item);
                if s.hover == Some(n) { draws.extend(self.tooltip(panel, &s, area, measure, item)); }
                draws
            };
            let shown = LookState { pointer: if state.hover == Some(n) { PointerState::Hover } else { PointerState::Normal }, selected };
            let selections: &[bool] = if select == scene::Select::None { &[false] } else { &[false, true] };
            let states = selections
                .iter()
                .flat_map(|&selected| [PointerState::Normal, PointerState::Hover].map(|pointer| LookState { pointer, selected }))
                .filter(|&l| l != shown)
                .map(|l| (l, draws_in(l)))
                .collect();
            let mut draws = self.draw_item(panel, state, area, measure, n, item);
            if state.hover == Some(n) { draws.extend(self.tooltip(panel, state, area, measure, item)); }
            let kind = match &item.kind {
                Kind::Slider { control, min, max, knob, .. } => {
                    let kw = knob.as_ref().and_then(|k| self.art_named(k, state)).and_then(|a| a.size).map_or(0, |s| s.0 as i32);
                    let value = match state.values.get(control) { Some(Value::Number(v)) => *v, _ => *min };
                    let value = (((value - min) / (max - min)).clamp(0.0, 1.0) * 1_000_000.0).round() as i32;
                    scene::InteractableKind::Slider { track: [rect[0] + kw / 2, rect[1] + rect[3] / 2, (rect[2] - kw).max(1), 1], value }
                }
                _ => scene::InteractableKind::Button,
            };
            // A control that changes other commands or enabled controls is
            // committed with those commands. Predicting only its selection
            // would show the new tab over the previous tab's body. Keep all
            // hover/pressed art and both committed selection variants.
            let select = if matches!(select, scene::Select::Pick { .. } | scene::Select::Toggle) && item.control().is_some_and(|key| panel.content_controls.contains(key)) {
                scene::Select::None
            } else { select };
            // A check box is the box and its label to a person: the label
            // draws button snapping too, which points at the box.
            let (anchor, region) = match &item.kind {
                Kind::Checkbox { .. } => {
                    let (bw, bh) = item.normal.as_ref().and_then(|a| self.art_named(a, state)).and_then(|a| a.size).unwrap_or((16, 16));
                    let anchor = [rect[0] + (bw as i32).min(rect[2]) / 2, rect[1] + (bh as i32).min(rect[3]) / 2];
                    let [mut l, mut t, mut r, mut b] = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
                    for d in &draws {
                        if let Draw::Text(text) = d {
                            let m = measure(&text.style, &text.text);
                            let [cl, ct, cr, cb] = text.clip;
                            let (tl, tt) = (text.x.max(cl), (text.y - m.ascent).max(ct));
                            let (tr, tb) = ((text.x + m.width).min(cr), (text.y + m.descent).min(cb));
                            if tr > tl && tb > tt {
                                (l, t, r, b) = (l.min(tl), t.min(tt), r.max(tr), b.max(tb));
                            }
                        }
                    }
                    let region = [l, t, r - l, b - t];
                    (Some(anchor), (region != rect).then_some(region))
                }
                _ => (None, None),
            };
            out.push(ControlLooks { id: id_hash(&panel.name, &n.to_string()), rect, anchor, region, kind, select, shown, draws, states });

        }
        out
    }

    /// The draws of item `n` of `panel`.
    fn draw_item(&self, panel: &Panel, state: &PanelState, area: [i32; 4], measure: &dyn Fn(&TextStyle, &str) -> TextMetrics, n: usize, item: &Item) -> Vec<Draw> {
        let (ox, oy) = self.origin(panel, area);
        let clip = [ox, oy, ox + panel.size.0 as i32, oy + panel.size.1 as i32];
        let mut out = Vec::new();
        let mut d = Drawer { panels: self, state, clip, measure, out: &mut out };
        {
            let enabled = Self::enabled(item, state);
            let hover = enabled && state.hover == Some(n);
            let (x, y) = (ox + item.at.0, oy + item.at.1);
            let size = self.item_size(item, state);
            let look = |selected: bool| -> Option<&String> {
                if !enabled {
                    item.disabled.as_ref().or(item.normal.as_ref())
                } else if selected {
                    item.selected.as_ref().or(item.normal.as_ref())
                } else if hover {
                    item.hover.as_ref().or(item.normal.as_ref())
                } else {
                    item.normal.as_ref()
                }
            };
            let text_look = TextLook { enabled, selected: false, hover };
            match &item.kind {
                Kind::Image(art) => d.art(art, (x, y), size),
                Kind::Text => {
                    let text = d.values_text(item.text.as_deref().unwrap_or_default(), item.decimals, item.percent);
                    let (align, valign) = (item.align.unwrap_or_default(), item.valign.unwrap_or_default());
                    // In a box, aligned within it.
                    let at = match size {
                        Some((w, h)) => (
                            x + match align {
                                Align::Left => 0,
                                Align::Center => w as i32 / 2,
                                Align::Right => w as i32,
                            },
                            y + match valign {
                                VAlign::Top | VAlign::Baseline => 0,
                                VAlign::Middle => h as i32 / 2,
                            },
                        ),
                        None => (x, y),
                    };
                    d.text(&text, item.style.as_deref(), text_look, at, align, valign);
                }
                Kind::Choice { control, value } => {
                    let selected = state.values.get(control).is_some_and(|v| v.matches(value));
                    let (w, h) = size.unwrap_or_default();
                    if let Some(a) = look(selected) {
                        d.art(a, (x, y), Some((w, h)));
                    }
                    if let Some(icon) = &item.icon {
                        let isize = self.art_named(icon, state).and_then(|a| a.size).unwrap_or_default();
                        let at = item.icon_at.map_or((x + (w as i32 - isize.0 as i32) / 2, y + (h as i32 - isize.1 as i32) / 2), |(ix, iy)| (x + ix, y + iy));
                        d.art(icon, at, None);
                    }
                    if let Some(t) = &item.text {
                        let t = d.values_text(t, item.decimals, item.percent);
                        let look = TextLook { selected, ..text_look };
                        match item.text_at {
                            Some((tx, ty)) => d.text(&t, item.style.as_deref(), look, (x + tx, y + ty), Align::Left, VAlign::Top),
                            None => d.text(&t, item.style.as_deref(), look, (x + w as i32 / 2, y + h as i32 / 2), Align::Center, VAlign::Middle),
                        }
                    }
                }
                Kind::Checkbox { control } => {
                    let on = state.values.get(control).is_some_and(Value::truthy);
                    let box_size = item.normal.as_ref().and_then(|a| self.art_named(a, state)).and_then(|a| a.size).unwrap_or((16, 16));
                    if let Some(a) = look(on) {
                        d.art(a, (x, y), None);
                    }
                    if let Some(t) = &item.text {
                        let t = d.values_text(t, item.decimals, item.percent);
                        match item.text_at {
                            Some((tx, ty)) => d.text(&t, item.style.as_deref(), text_look, (x + tx, y + ty), Align::Left, VAlign::Top),
                            None => d.text(&t, item.style.as_deref(), text_look, (x + box_size.0 as i32 + 6, y + box_size.1 as i32 / 2), Align::Left, VAlign::Middle),
                        }
                    }
                }
                Kind::Slider { control, min, max, knob, knob_hover, ticks, tick_labels, tick_style, .. } => {
                    let (w, h) = size.unwrap_or_default();
                    if let Some(a) = look(false) {
                        d.art(a, (x, y), Some((w, h)));
                    }
                    let v = match state.values.get(control) {
                        Some(Value::Number(n)) => n.clamp(*min, *max),
                        _ => *min,
                    };
                    let kw = knob.as_ref().and_then(|k| self.art_named(k, state)).and_then(|a| a.size).map_or(0, |s| s.0 as i32);
                    let ruler_style = tick_style.as_deref().or(item.style.as_deref());
                    let clip = d.clip;
                    // The ruler stays inside the item, including labels at either end.
                    d.clip = [clip[0].max(x), clip[1].max(y), clip[2].min(x + w as i32), clip[3].min(y + h as i32)];
                    for (tick, &(value, label)) in ticks.iter().enumerate() {
                        let cx = x + slider_center(w as i32, kw, (value - min) / (max - min));
                        let rgb = self.style(ruler_style).map_or([72, 60, 36], |s| if enabled { s.color } else { s.disabled_color.unwrap_or(s.color) });
                        // As on a ruler: a mark down from the top edge and one up
                        // from the bottom edge, the label between them.
                        let (top, bottom, len) = ruler_marks(y, h as i32, label);
                        d.element(Element::Fill { rect: [cx, top, 1, len], rgb, alpha: 255, clip: d.clip });
                        d.element(Element::Fill { rect: [cx, bottom, 1, len], rgb, alpha: 255, clip: d.clip });
                        if label {
                            let text = tick_labels.as_ref().map(|labels| labels[tick].clone()).unwrap_or_else(|| format_number(value, item.decimals, item.percent));
                            d.text(&text, ruler_style, text_look,
                                (cx, y + h as i32 / 2), Align::Center, VAlign::Middle);
                        }
                    }
                    d.clip = clip;
                    let knob = if hover || state.dragging == Some(n) { knob_hover.as_ref().or(knob.as_ref()) } else { knob.as_ref() };
                    if let Some(k) = knob {
                        let (kw, kh) = self.art_named(k, state).and_then(|a| a.size).unwrap_or_default();
                        let t = (v - min) / (max - min);
                        let kx = x + slider_center(w as i32, kw as i32, t) - kw as i32 / 2;
                        d.art(k, (kx, y + (h as i32 - kh as i32) / 2), None);
                    }
                    if let Some(t) = &item.text {
                        let t = substitute(t, |k| if k == "value" { Some(format_number(v, item.decimals, item.percent)) } else { d.value(k, item.decimals, item.percent) });
                        match item.value_at {
                            Some((tx, ty)) => d.text(&t, item.style.as_deref(), text_look, (x + tx, y + ty), Align::Left, VAlign::Top),
                            None => d.text(&t, item.style.as_deref(), text_look, (x + w as i32 + 10, y + h as i32 / 2), Align::Left, VAlign::Middle),
                        }
                    }
                }
            }
        }
        out
    }
}

/// A panel's control: where it is (window pixels x, y, w, h), its id as
/// something to point at, and how it is drawn now and in its other states
/// (see [`Panels::looks`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ControlLooks {
    pub id: u64,
    pub rect: [i32; 4],
    /// Where a pointer goes on it (`scene::Interactable::anchor`): a check
    /// box's box.
    pub anchor: Option<[i32; 2]>,
    /// All of it as a person sees it, when more than `rect`
    /// (`scene::Interactable::region`): a check box with its label.
    pub region: Option<[i32; 4]>,
    pub kind: scene::InteractableKind,
    pub select: scene::Select,
    pub shown: scene::LookState,
    pub draws: Vec<Draw>,
    pub states: Vec<(scene::LookState, Vec<Draw>)>,
}

impl ControlLooks {
    /// The group a click picks it in (0: none).
    pub fn select_group(&self) -> u64 {
        match self.select {
            scene::Select::Pick { group } => group,
            _ => 0,
        }
    }
}

/// A stable id for `key` of panel `panel` (FNV-1a, the top two bits set:
/// clear of a game's object addresses and of the menus' ids).
fn id_hash(panel: &str, key: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in panel.bytes().chain([0]).chain(key.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h | 3 << 62
}

/// How a text is shown: greyed out, selected, under the pointer.
#[derive(Copy, Clone, Debug)]
struct TextLook {
    enabled: bool,
    selected: bool,
    hover: bool,
}

struct Drawer<'a> {
    panels: &'a Panels,
    state: &'a PanelState,
    clip: [i32; 4],
    measure: &'a dyn Fn(&TextStyle, &str) -> TextMetrics,
    out: &'a mut Vec<Draw>,
}

impl Drawer<'_> {
    fn value(&self, key: &str, decimals: u32, percent: bool) -> Option<String> {
        self.state.values.get(key).map(|v| value_text(v, decimals, percent))
    }

    fn values_text(&self, text: &str, decimals: u32, percent: bool) -> String {
        substitute(text, |k| self.value(k, decimals, percent))
    }

    fn element(&mut self, e: Element) {
        if !e.is_empty() {
            self.out.push(Draw::Element(e));
        }
    }

    /// Art `name` with its top-left at (x, y), `size` large (else its own).
    fn art(&mut self, name: &str, (x, y): (i32, i32), size: Option<(u32, u32)>) {
        let Some(art) = self.panels.art_named(name, self.state) else { return };
        let Some((w, h)) = size.or(art.size) else { return };
        let (w, h) = (w as i32, h as i32);
        let clip = self.clip;
        match &art.kind {
            ArtKind::None => {}
            ArtKind::Region(region) => {
                let [rx, ry, rw, rh] = region.rect;
                let piece = |dx: i32, dy: i32, sx: i32, sw: i32, sh: i32| Element::Image {
                    region: Region { rect: [sx, ry, sw, sh], ..region.clone() },
                    at: (x + dx, y + dy),
                    clip,
                };
                match art.slice {
                    // Both ends, the middle from the left part.
                    Some((left, right)) if w < rw && w >= left + right => {
                        let right = right.min(w);
                        self.element(piece(0, 0, rx, w - right, h.min(rh)));
                        self.element(piece(w - right, 0, rx + rw - right, right, h.min(rh)));
                    }
                    // Repeated over the size (or its top-left part).
                    _ => {
                        let mut ty = 0;
                        while ty < h {
                            let mut tx = 0;
                            while tx < w {
                                self.element(piece(tx, ty, rx, rw.min(w - tx), rh.min(h - ty)));
                                tx += rw;
                            }
                            ty += rh;
                        }
                    }
                }
            }
            ArtKind::Sprite(name) => self.out.push(Draw::Element(Element::Sprite { name: name.clone(), at: (x, y), clip, scale_den: 0, palette: String::new() })),
            ArtKind::Fill { rgb, alpha } => self.element(Element::Fill { rect: [x, y, w, h], rgb: *rgb, alpha: *alpha, clip }),
        }
        if let Some([r, g, b, a]) = art.tint {
            self.element(Element::Fill { rect: [x, y, w, h], rgb: [r, g, b], alpha: a, clip });
        }
        if let Some(rgb) = art.border {
            for rect in [[x, y, w, 1], [x, y + h - 1, w, 1], [x, y + 1, 1, h - 2], [x + w - 1, y + 1, 1, h - 2]] {
                self.element(Element::Fill { rect, rgb, alpha: 255, clip });
            }
        }
        for l in &art.labels {
            let text = self.values_text(&l.text, 2, false);
            self.text(&text, l.style.as_deref(), TextLook { enabled: true, selected: false, hover: false }, (x + l.at.0, y + l.at.1), l.align, l.valign);
        }
        for (icon, (cx, cy)) in art.icons.clone() {
            let Some((iw, ih)) = self.panels.art_named(&icon, self.state).and_then(|a| a.size) else { continue };
            self.art(&icon, (x + cx - iw as i32 / 2, y + cy - ih as i32 / 2), None);
        }
    }

    /// Text in a style with its point at `at`, aligned so.
    fn text(&mut self, text: &str, style: Option<&str>, look: TextLook, (x, y): (i32, i32), align: Align, valign: VAlign) {
        let Some(s) = self.panels.style(style) else { return };
        if text.is_empty() {
            return;
        }
        let color = if !look.enabled {
            s.disabled_color.unwrap_or(s.color)
        } else if look.selected {
            s.selected_color.unwrap_or(s.color)
        } else if look.hover {
            s.hover_color.unwrap_or(s.color)
        } else {
            s.color
        };
        let style = TextStyle { face: s.face.clone(), height: s.height, weight: s.weight, italic: s.italic, color };
        let m = (self.measure)(&style, text);
        let x = match align {
            Align::Left => x,
            Align::Center => x - m.width / 2,
            Align::Right => x - m.width,
        };
        let y = match valign {
            VAlign::Top => y + m.ascent,
            // The middle of the ink, ascent over descent.
            VAlign::Middle => y + (m.ascent - m.descent + 1) / 2,
            VAlign::Baseline => y,
        };
        self.out.push(Draw::Text(TextDraw { text: text.to_owned(), x, y, style, clip: self.clip }));
    }
}

fn value_text(v: &Value, decimals: u32, percent: bool) -> String {
    match v {
        Value::Text(t) => t.clone(),
        Value::On(b) => b.to_string(),
        Value::Number(n) => format_number(*n, decimals, percent),
    }
}

fn inside([x, y, w, h]: [i32; 4], px: i32, py: i32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// Handle centre along the track. Ends are inset by half its width.
fn slider_center(width: i32, knob: i32, t: f32) -> i32 {
    knob / 2 + ((width - knob).max(0) as f32 * t.clamp(0.0, 1.0)).round() as i32
}

/// A ruler tick's marks on a track at `y`, `h` high: (top mark's y, bottom
/// mark's y, their length). Both touch the inside of the track's 2-pixel
/// frame; labelled ticks get longer marks, as whole numbers on a ruler.
fn ruler_marks(y: i32, h: i32, labelled: bool) -> (i32, i32, i32) {
    const FRAME: i32 = 2;
    let len = if labelled { 4 } else { 3 };
    (y + FRAME, y + h - FRAME - len, len)
}

/// `v` on the nearest step from `min` (any value with no step), within
/// `min..=max`, without float noise.
fn snap(v: f32, min: f32, max: f32, step: f32) -> f32 {
    let v = v.clamp(min, max);
    let v = if step > 0.0 { min + ((v - min) / step).round() * step } else { v };
    ((v * 10_000.0).round() / 10_000.0).clamp(min, max)
}

/// Slider `item` moved `steps` of its own step (a twentieth of its range
/// without one), clamped to its range: a wheel notch or an arrow key.
fn step_slider(item: &Item, state: &mut PanelState, steps: i32) -> Option<PanelEvent> {
    let Kind::Slider { control, min, max, step, .. } = &item.kind else { return None };
    let by = if *step > 0.0 { *step } else { (max - min) / 20.0 };
    let now = match state.values.get(control) {
        Some(Value::Number(n)) => *n,
        _ => *min,
    };
    let v = snap(now + by * steps as f32, *min, *max, *step);
    if v == now { return None; }
    state.values.insert(control.clone(), Value::Number(v));
    Some(PanelEvent::Slid { control: control.clone(), value: v })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANELS: &str = r#"
        [styles.default]
        face = "Sans"
        height = 10
        color = [1, 2, 3]
        disabled_color = [9, 9, 9]
        selected_color = [0, 0, 200]

        [art.button]
        image = "buttons.pcx"
        rect = [1, 1, 50, 20]
        border = [5, 5, 5]

        [art.button_on]
        image = "buttons.pcx"
        rect = [60, 1, 50, 20]

        [art.button_off]
        image = "buttons.pcx"
        rect = [1, 1, 50, 20]
        tint = [128, 128, 128, 100]

        [art.track]
        image = "bar.pcx"
        rect = [0, 0, 200, 10]
        slice = [2, 3]

        [art.knob]
        fill = [0, 200, 0]
        size = [10, 10]

        [art.cover]
        fill = [0, 0, 0]
        alpha = 128

        [art.icon_a]
        generated = "panes:1,1,1,255:20x10:vertical11"
        size = [20, 10]
        labels = [{ text = "A", at = [5, 5] }]
        icons = [{ art = "dot", at = [15, 5] }]

        [art.dot]
        sprite = "Dot/0"
        size = [4, 2]

        [panels.p]
        size = [400, 300]
        anchor = "center"

        [[panels.p.items]]
        choice = "tab"
        value = "one"
        at = [0, 0]
        size = [50, 20]
        text = "One"
        normal = "button"
        selected = "button_on"

        [[panels.p.items]]
        choice = "tab"
        value = "two"
        at = [60, 0]
        size = [50, 20]
        text = "Two"
        normal = "button"
        selected = "button_on"

        [[panels.p.items]]
        image = "cover"
        at = [0, 30]
        size = [400, 270]
        show_if = "tab=two"

        [[panels.p.items]]
        repeat = ["x", "y"]
        spacing = [0, 30]
        choice = "pick.{item}"
        value = "Left"
        at = [10, 40]
        size = [50, 20]
        normal = "button"
        selected = "button_on"
        disabled = "button_off"
        icon = "icon_{mode}"
        enable_if = "mode=a|b & !locked"
        show_if = "tab=two"

        [[panels.p.items]]
        checkbox = "smooth"
        at = [10, 120]
        size = [100, 16]
        normal = "knob"
        selected = "button_on"
        text = "Smooth"
        show_if = "tab=two"

        [[panels.p.items]]
        slider = "scale"
        at = [10, 150]
        size = [120, 10]
        normal = "track"
        knob = "knob"
        min = 0.5
        max = 3.0
        step = 0.25
        text = "{value}x"
        show_if = "tab=two"

        [[panels.p.items]]
        text = "Size {size} at {strip}"
        percent = true
        at = [10, 200]
        show_if = "tab=two"
    "#;

    fn state(tab: &str) -> PanelState {
        let mut s = PanelState::default();
        s.values.insert("tab".into(), Value::Text(tab.into()));
        s.values.insert("mode".into(), Value::Text("a".into()));
        s.values.insert("scale".into(), Value::Number(1.0));
        s.values.insert("size".into(), Value::Text("1280×720".into()));
        s.values.insert("strip".into(), Value::Number(0.25));
        s
    }

    const AREA: [i32; 4] = [100, 50, 600, 400];
    // Centred: the panel's origin is (200, 100).

    fn click(panels: &Panels, p: &Panel, s: &mut PanelState, x: i32, y: i32) -> Vec<Routed> {
        [true, false]
            .iter()
            .map(|&down| panels.input(p, s, AREA, &Input::MouseButton { button: Button::Left, down, x, y }))
            .collect()
    }

    #[test]
    fn panel_target_mask_follows_visible_items_and_rectangle_edges() {
        let panels = Panels::parse(PANELS).unwrap();
        let panel = panels.panel("p").unwrap();
        let mut state = state("one");
        assert!(!panels.covers_any(panel, &state, AREA, [220, 150, 20, 20]));
        assert!(panels.covers_any(panel, &state, AREA, [190, 105, 11, 5]));
        assert!(!panels.covers_any(panel, &state, AREA, [190, 105, 10, 5]), "touching edges do not overlap");
        assert!(!panels.covers_any(panel, &state, AREA, [200, 100, 0, 20]));
        assert!(!panels.covers_any(panel, &state, AREA, [200, 100, 20, -1]));
        state.values.insert("tab".into(), Value::Text("two".into()));
        state.values.insert("locked".into(), Value::On(true));
        assert!(panels.covers_any(panel, &state, AREA, [220, 150, 20, 20]), "painted cover still blocks the native control when its own control is disabled");
        assert!(!panels.covers_any(panel, &state, AREA, [600, 150, 20, 20]));
    }

    #[test]
    fn parses_repeats_and_checks_names() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        assert_eq!(p.items.len(), 8);
        assert_eq!(p.controls(), ["tab", "pick.x", "pick.y", "smooth", "scale"]);
        assert_eq!((p.items[3].at, p.items[4].at), ((10, 40), (10, 70)));
        assert!(panels.description().contains("source = \"panes:1,1,1,255:20x10:vertical11\"\n"));
        assert!(panels.description().contains("group = \"Panel@icon_a\""));
        assert_eq!(panels.text_styles().len(), 3);
        let bad = |extra: &str| Panels::parse(&format!("{PANELS}\n{extra}")).unwrap_err().to_string();
        assert!(bad("[[panels.p.items]]\nimage = \"nothing\"").contains("unknown art \"nothing\""));
        assert!(bad("[[panels.p.items]]\nchoice = \"c\"\nsize = [1, 1]").contains("needs its value"));
        assert!(bad("[[panels.p.items]]\nslider = \"s\"\nsize = [1, 1]\nmin = 2.0\nmax = 1.0").contains("min < max"));
        assert!(bad("[[panels.p.items]]\ntext = \"t\"\nstyle = \"bold\"").contains("unknown style"));
        assert!(bad("[[panels.p.items]]\ntext = \"t\"\nshow_if = \"=x\"").contains("without a name"));
        assert!(bad("[[panels.p.items]]\nimage = \"cover\"\nchoice = \"c\"").contains("one of"));
        assert!(bad("[art.both]\nfill = [1, 1, 1]\nsprite = \"s\"").contains("one of"));
        assert!(bad("[art.gen]\ngenerated = \"solid:1,1,1,1:1x1\"").contains("needs its size"));
        assert!(Panels::parse("[panels.q]\nsize = [1, 1]\nbogus = 1").is_err());
    }

    #[test]
    fn conditions_test_values() {
        let values: BTreeMap<String, Value> =
            [("a".to_string(), Value::Text("x".into())), ("on".to_string(), Value::On(true)), ("n".to_string(), Value::Number(0.5))].into();
        let holds = |c: &str| Cond::parse(c).unwrap().holds(&values);
        assert!(holds("a=x") && holds("a=y|x") && !holds("a=y") && holds("a!=y") && !holds("a!=x|y"));
        assert!(holds("on") && !holds("!on") && holds("on=true") && !holds("off") && holds("!off"));
        assert!(holds("n=0.5") && holds("a=x & on") && !holds("a=x & !on"));
    }

    #[test]
    fn tabs_choose_and_cover_only_what_they_draw() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let mut s = state("one");
        // Tab one shows the tabs only: below them is not the panel's.
        assert!(panels.covers(p, &s, AREA, 205, 105));
        assert!(!panels.covers(p, &s, AREA, 205, 200));
        let move_below = panels.input(p, &mut s, AREA, &Input::MouseMove { x: 205, y: 200 });
        assert!(!move_below.consumed);
        // A click on tab two: taken, chosen.
        let r = click(&panels, p, &mut s, 265, 105);
        assert!(r.iter().all(|r| r.consumed));
        assert_eq!(r[1].events, [PanelEvent::Chose { control: "tab".into(), value: "two".into() }]);
        assert_eq!(s.values["tab"], Value::Text("two".into()));
        // Choosing it again changes nothing.
        assert!(click(&panels, p, &mut s, 265, 105)[1].events.is_empty());
        // Now the cover takes the rest.
        assert!(panels.covers(p, &s, AREA, 205, 200));
        // A press below, released on the panel: neither is the panel's.
        let s2 = &mut state("one");
        let down = panels.input(p, s2, AREA, &Input::MouseButton { button: Button::Left, down: true, x: 205, y: 200 });
        let over = panels.input(p, s2, AREA, &Input::MouseMove { x: 205, y: 105 });
        let up = panels.input(p, s2, AREA, &Input::MouseButton { button: Button::Left, down: false, x: 205, y: 105 });
        assert!(!down.consumed && !over.consumed && !up.consumed && up.events.is_empty());
        // Pressed on a tab, released elsewhere: taken, no click.
        let s3 = &mut state("one");
        let down = panels.input(p, s3, AREA, &Input::MouseButton { button: Button::Left, down: true, x: 265, y: 105 });
        let up = panels.input(p, s3, AREA, &Input::MouseButton { button: Button::Left, down: false, x: 205, y: 200 });
        assert!(down.consumed && up.consumed && up.events.is_empty());
        // Keys are never the panel's.
        assert!(!panels.input(p, &mut s, AREA, &Input::Key { vk: 13, down: true, text: None }).consumed);
    }

    #[test]
    fn controls_come_with_their_looks_in_other_states() {
        use scene::{LookState, PointerState, Select};
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let measure = |s: &TextStyle, t: &str| estimate(s, t);
        let mut s = state("one");
        s.hover = Some(1);
        let looks = panels.looks(p, &s, AREA, &measure);
        // The two tabs only (the rest is hidden), topmost first.
        assert_eq!(looks.iter().map(|l| l.rect).collect::<Vec<_>>(), [[260, 100, 50, 20], [200, 100, 50, 20]]);
        let (one, two) = (&looks[1], &looks[0]);
        assert_eq!((one.select, two.select), (Select::None, Select::None), "content-switching tabs commit with the body");
        assert_eq!(one.shown, LookState { pointer: PointerState::Normal, selected: true });
        assert_eq!(two.shown, LookState { pointer: PointerState::Hover, selected: false });
        assert_eq!(one.draws, panels.draw_item(p, &s, AREA, &measure, 0, &p.items[0]));
        // Tab two chosen: what it draws when the tab is two.
        let mut chosen = s.clone();
        chosen.values.insert("tab".into(), Value::Text("two".into()));
        let selected = two.states.iter().find(|(l, _)| *l == LookState { pointer: PointerState::Hover, selected: true }).unwrap();
        assert_eq!(selected.1, panels.draw_item(p, &chosen, AREA, &measure, 1, &p.items[1]));
        assert_ne!(selected.1, two.draws);
        assert_eq!(two.states.len(), 3);
        // Ids stay with the item.
        assert_eq!(panels.looks(p, &state("one"), AREA, &measure)[0].id, two.id);
        let independent_index = p.items.iter().position(|item| matches!(&item.kind, Kind::Choice { control, .. } if control == "pick.x")).unwrap();
        let independent_id = id_hash(&p.name, &independent_index.to_string());
        let expanded = panels.looks(p, &state("two"), AREA, &measure);
        assert!(matches!(expanded.iter().find(|l| l.id == independent_id).unwrap().select, Select::Pick { .. }), "independent choices keep immediate selection feedback");
    }

    #[test]
    fn disabled_controls_do_nothing_and_checkboxes_toggle() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let mut s = state("two");
        // pick.y at (210, 170): enabled with mode a.
        let r = click(&panels, p, &mut s, 215, 175);
        assert_eq!(r[1].events, [PanelEvent::Chose { control: "pick.y".into(), value: "Left".into() }]);
        s.values.insert("locked".into(), Value::On(true));
        let r = click(&panels, p, &mut s, 215, 145);
        assert!(r[1].consumed && r[1].events.is_empty());
        s.values.insert("locked".into(), Value::On(false));
        s.values.insert("mode".into(), Value::Text("c".into()));
        assert!(click(&panels, p, &mut s, 215, 145)[1].events.is_empty());
        // The checkbox over its label.
        let r = click(&panels, p, &mut s, 290, 225);
        assert_eq!(r[1].events, [PanelEvent::Toggled { control: "smooth".into(), on: true }]);
        let r = click(&panels, p, &mut s, 215, 225);
        assert_eq!(r[1].events, [PanelEvent::Toggled { control: "smooth".into(), on: false }]);
    }

    #[test]
    fn a_checkbox_points_at_its_box_and_spans_its_label() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let s = state("two");
        let looks = panels.looks(p, &s, AREA, &estimate);
        let check = looks.iter().find(|l| l.select == scene::Select::Toggle).unwrap();
        let [x, y, w, h] = check.rect;
        let [ax, ay] = check.anchor.unwrap();
        // On the box (its art at the item's corner), inside the hit area.
        assert!(ax >= x && ax < x + 16 && ay >= y && ay < y + h, "{:?} {:?}", check.anchor, check.rect);
        // The region holds the hit area and every bit of the label's text.
        let [rx, ry, rw, rh] = check.region.unwrap_or(check.rect);
        assert!(rx <= x && ry <= y && rx + rw >= x + w && ry + rh >= y + h);
        for d in &check.draws {
            if let Draw::Text(t) = d {
                let m = estimate(&t.style, &t.text);
                assert!(rx <= t.x && rx + rw >= (t.x + m.width).min(t.clip[2]) && ry <= t.y - m.ascent, "{:?} {t:?}", check.region);
            }
        }
        // Other controls: neither.
        assert!(looks.iter().filter(|l| l.select != scene::Select::Toggle).all(|l| l.anchor.is_none() && l.region.is_none()));
    }

    #[test]
    fn sliders_follow_drags_and_the_wheel_on_their_steps() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let mut s = state("two");
        // The track at (210, 250), 120 wide; the knob 10: 110 pixels for
        // 0.5..3.0. Pressing at its far left: the minimum.
        let down = panels.input(p, &mut s, AREA, &Input::MouseButton { button: Button::Left, down: true, x: 212, y: 255 });
        assert_eq!(down.events, [PanelEvent::Slid { control: "scale".into(), value: 0.5 }]);
        // Dragged past the end (the pointer may leave the track): the maximum.
        let far = panels.input(p, &mut s, AREA, &Input::MouseMove { x: 900, y: 10 });
        assert!(far.consumed);
        assert_eq!(far.events, [PanelEvent::Slid { control: "scale".into(), value: 3.0 }]);
        assert!(panels.input(p, &mut s, AREA, &Input::MouseMove { x: 901, y: 10 }).events.is_empty());
        // Half way (215 + 55): 1.75, on a step of 0.25.
        let half = panels.input(p, &mut s, AREA, &Input::MouseMove { x: 270, y: 255 });
        assert_eq!(half.events, [PanelEvent::Slid { control: "scale".into(), value: 1.75 }]);
        let up = panels.input(p, &mut s, AREA, &Input::MouseButton { button: Button::Left, down: false, x: 270, y: 255 });
        assert!(up.consumed && up.events.is_empty() && s.dragging.is_none());
        let wheel = panels.input(p, &mut s, AREA, &Input::Wheel { delta: -120, x: 250, y: 255 });
        assert_eq!(wheel.events, [PanelEvent::Slid { control: "scale".into(), value: 1.5 }]);
        assert_eq!(s.values["scale"], Value::Number(1.5));
    }

    #[test]
    fn draws_art_text_and_values() {
        let panels = Panels::parse(PANELS).unwrap();
        let p = panels.panel("p").unwrap();
        let mut s = state("two");
        s.values.insert("smooth".into(), Value::On(true));
        s.hover = Some(1);
        let draws = panels.draw(p, &s, AREA, &estimate);
        let texts: Vec<&TextDraw> = draws.iter().filter_map(|d| if let Draw::Text(t) = d { Some(t) } else { None }).collect();
        let text = |t: &str| texts.iter().find(|d| d.text == t).copied();
        // Labels centred on their buttons: "Two" (3 · 10 · 11 / 20 = 16 wide) on (260..310).
        let two = text("Two").unwrap();
        assert_eq!((two.x, two.style.color), (260 + 25 - 8, [0, 0, 200]));
        assert_eq!(text("One").unwrap().style.color, [1, 2, 3]);
        assert_eq!(text("1x").unwrap().x, 210 + 120 + 10);
        assert_eq!(text("Size 1280×720 at 25").unwrap().y, 300 + 8);
        // The icon centred on pick.x (210..260, 140..160): at (225, 145).
        assert_eq!(text("A").map(|t| t.x), Some(225 + 5 - 2));
        assert_eq!(text("Smooth").unwrap().x, 210 + 10 + 6);
        // The selected tab in its art, the other with a border; the icon
        // (art named by a value) as a sprite.
        let regions: Vec<([i32; 4], (i32, i32))> =
            draws.iter().filter_map(|d| if let Draw::Element(Element::Image { region, at, .. }) = d { Some((region.rect, *at)) } else { None }).collect();
        assert!(regions.contains(&([1, 1, 50, 20], (200, 100))));
        assert!(regions.contains(&([60, 1, 50, 20], (260, 100))));
        assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Sprite { name, at: (225, 145), .. }) if name == "Panel@icon_a/0")));
        // Its icon centred on its point (15, 5) of it.
        assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Sprite { name, at: (238, 149), .. }) if name == "Dot/0")));
        // The track: its left part and its right end, the knob at 1.0:
        // 110 · 0.2 = 22 in.
        assert!(regions.contains(&([0, 0, 117, 10], (210, 250))));
        assert!(regions.contains(&([197, 0, 3, 10], (327, 250))));
        assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Fill { rect: [232, 250, 10, 10], rgb: [0, 200, 0], .. }))));
        // A disabled choice: its own art, greyed text colour (no text here).
        s.values.insert("mode".into(), Value::Text("z".into()));
        let draws = panels.draw(p, &s, AREA, &estimate);
        assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Fill { rect: [210, 140, 50, 20], alpha: 100, .. }))));
        // Tab one: the other items are not drawn.
        let draws = panels.draw(p, &state("one"), AREA, &estimate);
        assert_eq!(draws.iter().filter(|d| matches!(d, Draw::Text(_))).count(), 2);
    }

    struct NoFiles;
    impl render_desc::ImageLoader for NoFiles {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err(format!("{source}: no files"))
        }
    }

    #[test]
    fn generated_art_compiles_into_its_sprite() {
        let panels = Panels::parse(PANELS).unwrap();
        let text = format!("grid = {{ layout = \"orthogonal\", step = [8, 8] }}\n{}", panels.description());
        let compiled = render_desc::Description::parse(&text).unwrap().compile(&NoFiles, 256).unwrap();
        assert!(compiled.set(&generated_sprite("icon_a")).is_some());
        assert_eq!(generated_sprite("icon_a"), "Panel@icon_a/0");
    }

    #[test]
    fn preference_rulers_follow_handle_centres_and_stay_inside_each_slider() {
        let panels = Panels::parse(include_str!("../../../assets/civ3/preferences.toml")).unwrap();
        let panel = panels.panel("preferences").unwrap();
        let mut state = PanelState::default();
        let area = [0, 0, 1024, 768];
        let mut count = 0;
        for (index, item) in panel.items.iter().enumerate() {
            let Kind::Slider { control, min, max, knob, ticks, .. } = &item.kind else { continue };
            count += 1;
            assert!(!ticks.is_empty(), "{control} needs a ruler");
            let (w, h) = item.size.unwrap();
            let (x, y) = item.at;
            let kw = panels.art_named(knob.as_ref().unwrap(), &state).unwrap().size.unwrap().0 as i32;
            for &(value, label) in ticks {
                let cx = x + slider_center(w as i32, kw, (value - min) / (max - min));
                assert!(cx >= x + kw / 2 && cx <= x + w as i32 - kw / 2, "{control}: {value}");
                state.values.insert(control.clone(), Value::Number(value));
                let draws = panels.draw_item(panel, &state, area, &estimate, index, item);
                let (top, bottom, len) = ruler_marks(y, h as i32, label);
                for mark in [top, bottom] {
                    assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Fill { rect, .. })
                        if *rect == [cx, mark, 1, len])), "{control}: tick at {value}");
                }
                assert!(draws.iter().any(|d| matches!(d, Draw::Element(Element::Image { at, region, .. })
                    if region.source == "Art/SCROLL.PCX" && at.0 + kw / 2 == cx)), "{control}: handle at {value}");
                for draw in &draws {
                    if let Draw::Text(t) = draw {
                        if t.style.height != 9 { continue; }
                        let m = estimate(&t.style, &t.text);
                        assert_eq!(t.clip, [x, y, x + w as i32, y + h as i32]);
                        assert!(t.x >= x && t.x + m.width <= x + w as i32, "{control}: {}", t.text);
                        assert!(t.y - m.ascent >= y && t.y + m.descent <= y + h as i32);
                        assert!(t.text.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '×' || c == '%'));
                    }
                }
            }
        }
        assert_eq!(count, 15);
    }

    #[test]
    fn zoom_preferences_are_separate_and_controls_fit_without_overlap() {
        let panels = Panels::parse(include_str!("../../../assets/civ3/preferences.toml")).unwrap();
        let panel = panels.panel("preferences").unwrap();
        for menu_zoom in [false, true] {
            for tab in ["layout", "zoom"] {
                for mode in ["Off", "Floating131", "Floating171", "Mouse111", "Mouse131", "TopStrip"] {
                    let mut state = PanelState::default();
                    state.values.insert("tab".into(), Value::Text(tab.into()));
                    state.values.insert("layout.zoom".into(), Value::Text(mode.into()));
                    state.values.insert("layout.split".into(), Value::Text("Vertical1To1".into()));
                    state.values.insert("layout.menu_zoom".into(), Value::On(menu_zoom));
                    state.values.insert("layout.zoom_controls".into(), Value::On(menu_zoom || mode != "Off"));
                    state.values.insert("layout.zoom_lens_controls".into(), Value::On(menu_zoom || !matches!(mode, "Off" | "TopStrip")));
                    let looks = panels.looks(panel, &state, [0, 0, 1024, 768], &estimate);
                    for (i, look) in looks.iter().enumerate() {
                        let [x, y, w, h] = look.rect;
                        assert!(x >= 49 && x+w <= 973 && y >= 40 && y+h <= 690, "{tab} {mode}: {:?}", look.rect);
                        for other in &looks[..i] {
                            let [a, b, c, d] = other.rect;
                            assert!(x+w <= a || a+c <= x || y+h <= b || b+d <= y, "{tab} {mode}: {:?} overlaps {:?}", look.rect, other.rect);
                        }
                    }
                    for item in &panel.items {
                        if item.control().is_some_and(|k| k.starts_with("layout.zoom") || k == "layout.menu_zoom") {
                            let cond = item.show_if.as_ref().unwrap();
                            let mut layout = state.clone();
                            layout.values.insert("tab".into(), Value::Text("layout".into()));
                            assert!(!cond.holds(&layout.values), "zoom control on Layout");
                        }
                    }
                }
            }
        }
        let menu_zoom = panel.items.iter().find(|i| i.control() == Some("layout.menu_zoom")).unwrap();
        assert!(matches!(menu_zoom.kind, Kind::Checkbox { .. }));
        let mut state = PanelState::default();
        state.values.insert("tab".into(), Value::Text("zoom".into()));
        assert!(menu_zoom.show_if.as_ref().unwrap().holds(&state.values));
        let item = panel.items.iter().find(|i| i.control() == Some("layout.zoom_magnification_step")).unwrap();
        let Kind::Slider { tick_labels, .. } = &item.kind else { panic!("magnification slider") };
        assert_eq!(tick_labels.as_ref().unwrap(), &["1.5×", "2×", "3×", "4×"]);
    }

    #[test]
    fn preferences_undo_controls_fit_beside_values_and_tooltips_stay_in_the_panel() {
        let panels = Panels::parse(include_str!("../../../assets/civ3/preferences.toml")).unwrap();
        let panel = panels.panel("preferences").unwrap();
        let area = [200, 100, 1024, 768];
        let mut state = PanelState::default();
        for item in &panel.items {
            if let Kind::Slider { control, max, .. } = &item.kind {
                state.values.insert(control.clone(), Value::Number(*max));
                state.values.insert(format!("dialog.changed.{control}"), Value::On(true));
            }
        }
        for key in ["resolution.scale_menus", "display.limit_fps", "layout.zoom_controls", "layout.zoom_lens_controls"] {
            state.values.insert(key.into(), Value::On(true));
        }
        state.values.insert("layout.zoom".into(), Value::Text("Floating131".into()));
        state.values.insert("layout.zoom_magnification_label".into(), Value::Text("4×".into()));
        state.values.insert("layout.split".into(), Value::Text("Vertical1To1".into()));
        let mut checked = 0;
        let overlap = |a: [i32; 4], b: [i32; 4]| a[0] < b[0]+b[2] && b[0] < a[0]+a[2] && a[1] < b[1]+b[3] && b[1] < a[1]+a[3];
        for tab in ["resolution", "layout", "zoom"] {
            state.values.insert("tab".into(), Value::Text(tab.into()));
            let looks = panels.looks(panel, &state, area, &estimate);
            for (i, look) in looks.iter().enumerate() {
                assert!(looks[..i].iter().all(|other| !overlap(look.rect, other.rect)), "{tab}: {:?} overlaps a control", look.rect);
            }
            let undos: Vec<_> = panel.items.iter().enumerate().filter(|(_, i)| i.control() == Some("dialog.undo") && Panels::visible(i, &state)).collect();
            for (index, undo) in undos {
                checked += 1;
                let Kind::Choice { value: control, .. } = &undo.kind else { unreachable!() };
                let rect = panels.rect(undo, &state, panels.origin(panel, area)).unwrap();
                let (n, slider) = panel.items.iter().enumerate().find(|(_, i)| matches!(&i.kind, Kind::Slider { control: c, .. } if c == control)).unwrap();
                for draw in panels.draw_item(panel, &state, area, &estimate, n, slider) {
                    if let Draw::Text(t) = draw {
                        let m = estimate(&t.style, &t.text);
                        assert!(!overlap(rect, [t.x, t.y-m.ascent, m.width, m.ascent+m.descent]), "{control}: undo overlaps {}", t.text);
                    }
                }
                state.hover = Some(index);
                let draws = panels.draw(panel, &state, area, &estimate);
                let look = panels.looks(panel, &state, area, &estimate).into_iter().find(|l| l.id == id_hash(&panel.name, &index.to_string())).unwrap();
                assert!(draws.windows(look.draws.len()).any(|w| w == look.draws), "{control}: hovered control and tooltip must form one replacement range");
                let Draw::Text(t) = draws.last().unwrap() else { panic!("tooltip must paint above all items") };
                assert_eq!(t.text, "Undo to current state");
                let m = estimate(&t.style, &t.text);
                assert!(t.x >= area[0] && t.x + m.width <= area[0]+area[2]);
                assert!(t.y-m.ascent >= area[1] && t.y+m.descent <= area[1]+area[3]);
            }
            state.hover = None;
        }
        assert_eq!(checked, 13);
    }

    #[test]
    fn ruler_steps_must_be_finite_positive_and_bounded() {
        for step in ["0", "-1", "inf", "nan", "0.00001"] {
            let text = PANELS.replace("slider = \"scale\"", &format!("slider = \"scale\"\ntick_step = {step}"));
            assert!(Panels::parse(&text).is_err(), "tick step {step}");
        }
    }

    #[test]
    fn accessibility_controls_are_visible_hit_testable_and_fit() {
        let panels = Panels::parse(include_str!("../../../assets/civ3/preferences.toml")).unwrap();
        let p = panels.panel("preferences").unwrap();
        let mut state = PanelState::default();
        state.values.insert("tab".into(), Value::Text("accessibility".into()));
        state.values.insert("accessibility.unit_rings".into(), Value::On(true));
        state.values.insert("accessibility.active_unit_arrow".into(), Value::On(true));
        for item in p.items.iter().filter(|i| i.control().is_some_and(|k| k.starts_with("accessibility."))) {
            assert!(Panels::visible(item, &state));
            assert!(Panels::enabled(item, &state));
            let [x, y, w, h] = panels.rect(item, &state, (0, 0)).unwrap();
            assert!(x >= 70 && y >= 95 && x+w <= 952 && y+h <= 655);
            let hit = panels.hit(p, &state, [0, 0, 1024, 768], x+w/2, y+h/2);
            assert_eq!(hit, Some(p.items.iter().position(|i| std::ptr::eq(i, item)).unwrap()));
        }
        let looks = panels.looks(p, &state, [0, 0, 1024, 768], &estimate);
        for (i, a) in looks.iter().enumerate() {
            let [x, y, w, h] = a.rect;
            for b in &looks[..i] {
                let [l, t, bw, bh] = b.rect;
                assert!(x+w <= l || l+bw <= x || y+h <= t || t+bh <= y, "{:?} overlaps {:?}", a.rect, b.rect);
            }
        }
        let bob = p.items.iter().find(|i| i.control() == Some("accessibility.arrow_bobbing")).unwrap();
        state.values.insert("accessibility.active_unit_arrow".into(), Value::On(false));
        assert!(!Panels::visible(bob, &state));
        for tab in ["civ3", "resolution", "layout", "zoom", "clean_map"] {
            state.values.insert("tab".into(), Value::Text(tab.into()));
            assert!(p.items.iter().filter(|i| i.control().is_some_and(|k| k.starts_with("accessibility."))).all(|i| !Panels::visible(i, &state)));
        }
    }

    #[test]
    fn base_ui_slider_centres_line_up_with_pixel_scale_buttons() {
        let panels = Panels::parse(include_str!("../../../assets/civ3/preferences.toml")).unwrap();
        let panel = panels.panel("preferences").unwrap();
        let item = panel.items.iter().find(|i| i.control() == Some("resolution.base_ui_scale")).unwrap();
        let Kind::Slider { min, max, knob, .. } = &item.kind else { panic!("base scale slider") };
        let kw = panels.art_named(knob.as_ref().unwrap(), &PanelState::default()).unwrap().size.unwrap().0 as i32;
        let centre = |v| item.at.0 + slider_center(item.size.unwrap().0 as i32, kw, (v - min) / (max - min));
        for button in &panel.items {
            let Kind::Choice { control, value } = &button.kind else { continue };
            if control != "resolution.pixel_scale" { continue; }
            let k = value.parse::<f32>().unwrap();
            assert_eq!(centre(k), button.at.0 + button.size.unwrap().0 as i32 / 2);
            if k < *max { assert_eq!(centre(k + 0.5), (centre(k) + centre(k + 1.0)) / 2); }
        }
    }

    #[test]
    fn numbers_format_without_noise() {
        assert_eq!(format_number(1.0, 2, false), "1");
        assert_eq!(format_number(1.25, 2, false), "1.25");
        assert_eq!(format_number(0.35, 2, true), "35");
        assert_eq!(format_number(3.0, 0, false), "3");
        assert_eq!(snap(1.3333, 0.5, 3.0, 0.25), 1.25);
        assert_eq!(substitute("{a}-{b}-{", |k| (k == "a").then(|| "x".into())), "x--{");
    }

    #[test]
    fn alt_slider_transactions_preview_commit_and_restore_exact_original_value() {
        use scene::SliderAction;
        let panels = Panels::parse(PANELS).unwrap();
        let panel = panels.panel("p").unwrap();
        let mut state = state("two");
        // This value is deliberately between ordinary drag steps. Cancelling
        // must restore its bits rather than quantize it through a pixel.
        let original = 1.123_456_7f32;
        state.values.insert("scale".into(), Value::Number(original));
        let slider = panels.looks(panel, &state, AREA, &estimate).into_iter().find(|c| matches!(c.kind, scene::InteractableKind::Slider { .. })).unwrap();
        assert_eq!(slider.rect, [210, 250, 120, 10]);
        assert!(matches!(slider.kind, scene::InteractableKind::Slider { track: [215, 255, 110, 1], .. }));
        let edit = |state: &mut PanelState, action, value| panels.input(panel, state, AREA, &Input::Slider { id: slider.id, action, value });
        assert!(edit(&mut state, SliderAction::Begin, 900_000).events.is_empty());
        assert_eq!(state.values["scale"], Value::Number(original));
        assert_eq!(edit(&mut state, SliderAction::Preview, 600_000).events, [PanelEvent::Slid { control: "scale".into(), value: 2.0 }]);
        assert_eq!(edit(&mut state, SliderAction::Cancel, 0).events, [PanelEvent::Slid { control: "scale".into(), value: original }]);
        assert_eq!(state.values["scale"], Value::Number(original));
        edit(&mut state, SliderAction::Begin, 0);
        edit(&mut state, SliderAction::Preview, 1_000_000);
        assert!(edit(&mut state, SliderAction::Commit, 0).events.is_empty());
        assert_eq!(state.values["scale"], Value::Number(3.0));
        assert!(edit(&mut state, SliderAction::Cancel, 0).events.is_empty());
        assert_eq!(state.values["scale"], Value::Number(3.0));
    }

    #[test]
    fn keyboard_steps_use_the_slider_step_clamp_and_join_an_open_alt_edit() {
        use scene::SliderAction;
        let panels = Panels::parse(PANELS).unwrap();
        let panel = panels.panel("p").unwrap();
        let mut state = state("two");
        state.values.insert("scale".into(), Value::Number(1.0));
        let slider = panels.looks(panel, &state, AREA, &estimate).into_iter().find(|c| matches!(c.kind, scene::InteractableKind::Slider { .. })).unwrap();
        let edit = |state: &mut PanelState, action, value| panels.input(panel, state, AREA, &Input::Slider { id: slider.id, action, value });
        let slid = |value| vec![PanelEvent::Slid { control: "scale".into(), value }];
        assert_eq!(edit(&mut state, SliderAction::Step, 1).events, slid(1.25));
        assert_eq!(edit(&mut state, SliderAction::Step, -2).events, slid(0.75));
        // Clamped at both ends; nothing at an end.
        assert_eq!(edit(&mut state, SliderAction::Step, -10).events, slid(0.5));
        assert!(edit(&mut state, SliderAction::Step, -1).events.is_empty());
        assert_eq!(edit(&mut state, SliderAction::Step, 100).events, slid(3.0));
        assert!(edit(&mut state, SliderAction::Step, 1).events.is_empty());
        // Within an Alt edit, Cancel undoes the steps too.
        edit(&mut state, SliderAction::Begin, 1_000_000);
        assert_eq!(edit(&mut state, SliderAction::Step, -1).events, slid(2.75));
        assert_eq!(edit(&mut state, SliderAction::Cancel, 0).events, slid(3.0));
        // A hidden slider takes no step.
        state.values.insert("tab".into(), Value::Text("one".into()));
        assert!(edit(&mut state, SliderAction::Step, -1).events.is_empty());
    }

    #[test]
    fn alt_slider_restores_before_tabs_or_controls_hide_and_ignores_inactive_requests() {
        use scene::SliderAction;
        let panels = Panels::parse(PANELS).unwrap();
        let panel = panels.panel("p").unwrap();
        let mut state = state("two");
        state.values.insert("scale".into(), Value::Number(1.0));
        let slider = panels.looks(panel, &state, AREA, &estimate).into_iter().find(|c| matches!(c.kind, scene::InteractableKind::Slider { .. })).unwrap();
        let edit = |state: &mut PanelState, action, value| panels.input(panel, state, AREA, &Input::Slider { id: slider.id, action, value });
        assert!(edit(&mut state, SliderAction::Preview, 900_000).events.is_empty());
        edit(&mut state, SliderAction::Begin, 0);
        edit(&mut state, SliderAction::Preview, 1_000_000);
        state.values.insert("tab".into(), Value::Text("one".into()));
        let routed = edit(&mut state, SliderAction::Preview, 900_000);
        assert_eq!(routed.events, [PanelEvent::Slid { control: "scale".into(), value: 1.0 }]);
        assert!(state.slider_edit.is_none());
        assert!(edit(&mut state, SliderAction::Begin, 0).events.is_empty());
        assert!(state.slider_edit.is_none(), "a hidden slider cannot start editing");
        assert_eq!(state.values["scale"], Value::Number(1.0));
        state.values.insert("tab".into(), Value::Text("two".into()));
        edit(&mut state, SliderAction::Begin, 0);
        edit(&mut state, SliderAction::Preview, 1_000_000);
        let routed = panels.input(panel, &mut state, AREA, &Input::MouseButton { button: Button::Left, down: true, x: 205, y: 105 });
        assert_eq!(routed.events, [PanelEvent::Slid { control: "scale".into(), value: 1.0 }]);
        assert!(state.slider_edit.is_none());
    }

    #[test]
    fn disabling_an_alt_slider_restores_it_and_removes_its_target() {
        use scene::SliderAction;
        let panels = Panels::parse(&PANELS.replace("slider = \"scale\"", "slider = \"scale\"\nenable_if = \"!locked\"")).unwrap();
        let panel = panels.panel("p").unwrap();
        let mut state = state("two");
        state.values.insert("scale".into(), Value::Number(1.25));
        let slider = panels.looks(panel, &state, AREA, &estimate).into_iter().find(|c| matches!(c.kind, scene::InteractableKind::Slider { .. })).unwrap();
        panels.input(panel, &mut state, AREA, &Input::Slider { id: slider.id, action: SliderAction::Begin, value: 0 });
        panels.input(panel, &mut state, AREA, &Input::Slider { id: slider.id, action: SliderAction::Preview, value: 1_000_000 });
        state.values.insert("locked".into(), Value::On(true));
        let cancelled = panels.input(panel, &mut state, AREA, &Input::Slider { id: slider.id, action: SliderAction::Preview, value: 0 });
        assert_eq!(cancelled.events, [PanelEvent::Slid { control: "scale".into(), value: 1.25 }]);
        assert!(state.slider_edit.is_none());
        assert!(!panels.looks(panel, &state, AREA, &estimate).iter().any(|c| c.id == slider.id));
        assert!(panels.input(panel, &mut state, AREA, &Input::Slider { id: slider.id, action: SliderAction::Begin, value: 0 }).events.is_empty());
        assert!(state.slider_edit.is_none());
    }

    #[test]
    fn dependent_checkboxes_commit_with_enabled_controls_but_independent_toggles_predict() {
        let measure = |style: &TextStyle, text: &str| estimate(style, text);
        let dependent = Panels::parse(&PANELS.replace("mode=a|b & !locked", "smooth")).unwrap();
        let panel = dependent.panel("p").unwrap();
        let checkbox_id = id_hash(&panel.name, &panel.items.iter().position(|item| matches!(&item.kind, Kind::Checkbox { control } if control == "smooth")).unwrap().to_string());
        let looks = dependent.looks(panel, &state("two"), AREA, &measure);
        let checkbox = looks.iter().find(|look| look.id == checkbox_id).unwrap();
        assert_eq!(checkbox.select, scene::Select::None, "dependent checkbox selection belongs to the same command group as its enabled controls");
        assert!(checkbox.states.iter().any(|(state, _)| state.pointer == scene::PointerState::Hover));
        let independent = Panels::parse(PANELS).unwrap();
        let panel = independent.panel("p").unwrap();
        assert_eq!(independent.looks(panel, &state("two"), AREA, &measure).iter().find(|look| look.id == checkbox_id).unwrap().select, scene::Select::Toggle);
    }

}
