//! Menus drawn on top of a tile world: images, buttons and text placed in
//! screen space, described in TOML, turned into free sprites for the
//! renderer, with hit testing that turns pointer input into button events.
//!
//! Game agnostic: a menu only names sprites and sets of a render
//! description (`render_desc`), so the same crate draws any game's menus
//! from that game's art.
//!
//! Three kinds of screens:
//! - menus (this module): images and buttons from the render description's
//!   sets, text from glyph sets, placed in the viewport (below);
//! - panels ([`panel`]): controls bound to named values (choices and tabs,
//!   checkboxes, sliders), images and text in a named face, shown and
//!   enabled by conditions on the values, drawn as run-time screen elements
//!   (regions of image files, fills, sprites) and text the driver draws with
//!   its own fonts, with input routing (what is the panel's, what lies
//!   below);
//! - run-time screens ([`screen`]): elements a driver makes frame by frame
//!   (a mirror of another program's interface), and the render description
//!   entries their art needs.
//!
//! ```toml
//! [fonts.small]
//! glyphs = "font_small"        # set "font_small/<codepoint>" per character
//! height = 12
//! advance = 7                  # fixed advance, or `widths = { 105 = 3 }`
//!
//! [menus.title]
//! size = [1024, 768]           # placed by `anchor` within the viewport
//! anchor = "center"
//!
//! [[menus.title.items]]
//! image = "title_background"
//! at = [0, 0]
//!
//! [[menus.title.items]]
//! button = "new_game"
//! at = [400, 300]
//! size = [200, 30]
//! normal = "buttons/0"
//! hover = "buttons/1"          # optional, like `pressed`
//! text = "New Game"
//! font = "small"
//! ```

use std::collections::BTreeMap;

pub mod panel;
pub mod screen;

use render_desc::Compiled;
use scene::{Button, FreeSprite, Input};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum MenuError {
    #[error("TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("menu {menu:?}: unknown {kind} {name:?}")]
    Unknown { menu: String, kind: &'static str, name: String },
}

/// Where a menu sits in the viewport.
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Top-left of a `size` box anchored in `viewport`.
    pub fn place(self, size: (u32, u32), viewport: (u32, u32)) -> (i32, i32) {
        let free = (viewport.0 as i32 - size.0 as i32, viewport.1 as i32 - size.1 as i32);
        let (fx, fy) = match self {
            Anchor::TopLeft => (0, 0),
            Anchor::Top => (1, 0),
            Anchor::TopRight => (2, 0),
            Anchor::Left => (0, 1),
            Anchor::Center => (1, 1),
            Anchor::Right => (2, 1),
            Anchor::BottomLeft => (0, 2),
            Anchor::Bottom => (1, 2),
            Anchor::BottomRight => (2, 2),
        };
        // Integer halves round down, like games centring a bitmap.
        (free.0 * fx / 2, free.1 * fy / 2)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontDesc {
    /// Prefix of the glyph sets: glyph for `c` is set `"{glyphs}/{c as u32}"`.
    pub glyphs: String,
    pub height: u32,
    /// Advance of every glyph without an entry in `widths`.
    #[serde(default)]
    pub advance: u32,
    #[serde(default)]
    pub widths: BTreeMap<u32, u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDesc {
    /// An image: a set name.
    pub image: Option<String>,
    /// A button: its id.
    pub button: Option<String>,
    /// Text alone, or a button's label.
    pub text: Option<String>,
    pub font: Option<String>,
    pub at: [i32; 2],
    /// A button's clickable size.
    #[serde(default)]
    pub size: [u32; 2],
    pub normal: Option<String>,
    pub hover: Option<String>,
    pub pressed: Option<String>,
    /// Where a button's label starts, relative to `at` (default: centred).
    pub text_at: Option<[i32; 2]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuDesc {
    pub size: [u32; 2],
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub items: Vec<ItemDesc>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenusDesc {
    #[serde(default)]
    pub fonts: BTreeMap<String, FontDesc>,
    #[serde(default)]
    pub menus: BTreeMap<String, MenuDesc>,
}

/// A sprite set resolved for drawing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Art {
    set: u32,
    scale_den: u32,
}

#[derive(Clone, Debug)]
struct Font {
    glyphs: BTreeMap<char, Art>,
    height: u32,
    advance: u32,
    widths: BTreeMap<u32, u32>,
}

impl Font {
    fn advance(&self, c: char) -> u32 {
        self.widths.get(&(c as u32)).copied().unwrap_or(self.advance)
    }

    fn width(&self, text: &str) -> u32 {
        text.chars().map(|c| self.advance(c)).sum()
    }
}

#[derive(Clone, Debug)]
enum Item {
    Image { art: Art, at: (i32, i32) },
    Text { text: String, font: usize, at: (i32, i32) },
    Button { id: String, at: (i32, i32), size: (u32, u32), normal: Option<Art>, hover: Option<Art>, pressed: Option<Art>, label: Option<(String, usize, (i32, i32))> },
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub name: String,
    size: (u32, u32),
    anchor: Anchor,
    items: Vec<Item>,
}

/// What a click did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    /// A button was pressed and released over it.
    Clicked(String),
}

/// Menus resolved against a compiled render description.
#[derive(Clone, Debug)]
pub struct Menus {
    fonts: Vec<Font>,
    menus: BTreeMap<String, Menu>,
}

impl Menus {
    pub fn parse(text: &str, compiled: &Compiled) -> Result<Self, MenuError> {
        let desc: MenusDesc = toml::from_str(text)?;
        Self::new(&desc, compiled)
    }

    pub fn new(desc: &MenusDesc, compiled: &Compiled) -> Result<Self, MenuError> {
        let art = |menu: &str, name: &str| -> Result<Art, MenuError> {
            let (set, blend) =
                compiled.set(name).ok_or_else(|| MenuError::Unknown { menu: menu.into(), kind: "set", name: name.into() })?;
            let scale_den = blend.scale_den();
            Ok(Art { set: set.raw(), scale_den })
        };
        let mut font_index = BTreeMap::new();
        let mut fonts = Vec::new();
        for (name, f) in &desc.fonts {
            let prefix = format!("{}/", f.glyphs);
            let mut glyphs = BTreeMap::new();
            for code in 0..0x1_0000u32 {
                let Some(c) = char::from_u32(code) else { continue };
                if let Some((set, _)) = compiled.set(&format!("{prefix}{code}")) {
                    glyphs.insert(c, Art { set: set.raw(), scale_den: 0 });
                }
            }
            font_index.insert(name.clone(), fonts.len());
            fonts.push(Font { glyphs, height: f.height, advance: f.advance, widths: f.widths.clone() });
        }
        let mut menus = BTreeMap::new();
        for (name, m) in &desc.menus {
            let font = |item: &ItemDesc| -> Result<usize, MenuError> {
                let f = item.font.as_deref().unwrap_or("default");
                font_index.get(f).copied().ok_or_else(|| MenuError::Unknown { menu: name.clone(), kind: "font", name: f.into() })
            };
            let optional = |s: &Option<String>| s.as_deref().map(|s| art(name, s)).transpose();
            let mut items = Vec::new();
            for item in &m.items {
                let at = (item.at[0], item.at[1]);
                if let Some(id) = &item.button {
                    let size = (item.size[0], item.size[1]);
                    let label = match &item.text {
                        Some(text) => {
                            let f = font(item)?;
                            let w = fonts[f].width(text) as i32;
                            let rel = item.text_at.map_or(
                                ((size.0 as i32 - w) / 2, (size.1 as i32 - fonts[f].height as i32) / 2),
                                |[x, y]| (x, y),
                            );
                            Some((text.clone(), f, rel))
                        }
                        None => None,
                    };
                    items.push(Item::Button {
                        id: id.clone(),
                        at,
                        size,
                        normal: optional(&item.normal)?,
                        hover: optional(&item.hover)?,
                        pressed: optional(&item.pressed)?,
                        label,
                    });
                } else if let Some(image) = &item.image {
                    items.push(Item::Image { art: art(name, image)?, at });
                } else if let Some(text) = &item.text {
                    items.push(Item::Text { text: text.clone(), font: font(item)?, at });
                }
            }
            menus.insert(name.clone(), Menu { name: name.clone(), size: (m.size[0], m.size[1]), anchor: m.anchor, items });
        }
        Ok(Self { fonts, menus })
    }

    pub fn menu(&self, name: &str) -> Option<&Menu> {
        self.menus.get(name)
    }

    /// The sprites of `menu` in `viewport`, drawn after pass `after_pass`
    /// (usually: on top of everything).
    pub fn sprites(&self, menu: &Menu, state: &MenuState, viewport: (u32, u32), after_pass: u32) -> Vec<FreeSprite> {
        let (ox, oy) = menu.anchor.place(menu.size, viewport);
        let mut out = Vec::new();
        let mut put = |art: Art, (x, y): (i32, i32)| {
            out.push(FreeSprite {
                set: art.set,
                x: ox + x,
                y: oy + y,
                screen: true,
                after_pass,
                phase: 0,
                ticks_per_frame: 1,
                scale_den: art.scale_den,
                blend: scene::SpriteBlend::Alpha,
                tint: 0,
                zoom_art: 0,
                zoom: (0, 0),
                zoom_offset: (0, 0),
                clip: None,
                not_in_view: None,
                glyph: None, solid: None,
                palette: 0,
                category: 0,
            });
        };
        let text = |font: &Font, s: &str, (x, y): (i32, i32), put: &mut dyn FnMut(Art, (i32, i32))| {
            let mut pen = x;
            for c in s.chars() {
                if let Some(&g) = font.glyphs.get(&c) {
                    put(g, (pen, y));
                }
                pen += font.advance(c) as i32;
            }
        };
        for item in &menu.items {
            match item {
                Item::Image { art, at } => put(*art, *at),
                Item::Text { text: s, font, at } => text(&self.fonts[*font], s, *at, &mut put),
                Item::Button { id, at, normal, hover, pressed, label, .. } => {
                    let look = if state.pressed.as_deref() == Some(id.as_str()) && state.hover.as_deref() == Some(id.as_str()) {
                        pressed.or(*hover).or(*normal)
                    } else if state.hover.as_deref() == Some(id.as_str()) {
                        hover.or(*normal)
                    } else {
                        *normal
                    };
                    if let Some(art) = look {
                        put(art, *at);
                    }
                    if let Some((s, font, rel)) = label {
                        text(&self.fonts[*font], s, (at.0 + rel.0, at.1 + rel.1), &mut put);
                    }
                }
            }
        }
        out
    }

    /// The buttons of `menu` as [`Menus::sprites`] draws them now and in
    /// their other states (normal, hover, pressed), for a renderer to draw
    /// those itself at once (`scene::Looks`); ids as
    /// [`Menus::interactables`] gives them.
    pub fn looks(&self, menu: &Menu, state: &MenuState, viewport: (u32, u32), after_pass: u32) -> Vec<ButtonLooks> {
        use scene::{LookState, PointerState};
        menu.items
            .iter()
            .filter_map(|item| {
                let Item::Button { id, .. } = item else { return None };
                let one = Menu { items: vec![item.clone()], ..menu.clone() };
                let in_state = |pointer: PointerState| {
                    let over = (pointer != PointerState::Normal).then(|| id.clone());
                    let pressed = (pointer == PointerState::Pressed).then(|| id.clone());
                    self.sprites(&one, &MenuState { hover: over, pressed }, viewport, after_pass)
                };
                let pointer = match (state.hover.as_deref() == Some(id.as_str()), state.pressed.as_deref() == Some(id.as_str())) {
                    (true, true) => PointerState::Pressed,
                    (true, false) => PointerState::Hover,
                    _ => PointerState::Normal,
                };
                let shown = LookState { pointer, selected: false };
                let states = [PointerState::Normal, PointerState::Hover, PointerState::Pressed]
                    .into_iter()
                    .filter(|&p| p != pointer)
                    .map(|p| (LookState { pointer: p, selected: false }, in_state(p)))
                    .collect();
                Some(ButtonLooks { id: id_hash(&menu.name, id), shown, sprites: self.sprites(&one, state, viewport, after_pass), states })
            })
            .collect()
    }

    /// The button of `menu` under viewport position (x, y).
    pub fn hit<'m>(&self, menu: &'m Menu, viewport: (u32, u32), x: i32, y: i32) -> Option<&'m str> {
        let (ox, oy) = menu.anchor.place(menu.size, viewport);
        let (x, y) = (x - ox, y - oy);
        menu.items.iter().rev().find_map(|item| match item {
            Item::Button { id, at, size, .. }
                if x >= at.0 && y >= at.1 && x < at.0 + size.0 as i32 && y < at.1 + size.1 as i32 =>
            {
                Some(id.as_str())
            }
            _ => None,
        })
    }

    /// The buttons of `menu` in `viewport` as things to point at (in the
    /// viewport's pixels, ids from the buttons' ids, the same for the same id).
    pub fn interactables(&self, menu: &Menu, viewport: (u32, u32)) -> Vec<scene::Interactable> {
        let (ox, oy) = menu.anchor.place(menu.size, viewport);
        menu.items
            .iter()
            .filter_map(|item| match item {
                Item::Button { id, at, size, .. } => Some(scene::Interactable { region: None, alt_right_click: false, anchor: None,
                    rect: [ox + at.0, oy + at.1, size.0 as i32, size.1 as i32],
                    kind: scene::InteractableKind::Button,
                    id: id_hash(&menu.name, id),
                }),
                _ => None,
            })
            .collect()
    }

    /// Updates hover/pressed state from pointer input; reports clicks.
    pub fn input(&self, menu: &Menu, state: &mut MenuState, viewport: (u32, u32), input: &Input) -> Option<MenuEvent> {
        match *input {
            Input::MouseMove { x, y } => {
                state.hover = self.hit(menu, viewport, x, y).map(str::to_owned);
                None
            }
            Input::MouseButton { button: Button::Left, down: true, x, y } => {
                state.hover = self.hit(menu, viewport, x, y).map(str::to_owned);
                state.pressed = state.hover.clone();
                None
            }
            Input::MouseButton { button: Button::Left, down: false, x, y } => {
                let over = self.hit(menu, viewport, x, y).map(str::to_owned);
                let pressed = state.pressed.take();
                state.hover = over.clone();
                (pressed.is_some() && pressed == over).then(|| MenuEvent::Clicked(over.unwrap()))
            }
            _ => None,
        }
    }
}

/// A stable id for button `id` of menu `menu` (FNV-1a, top bit set so it
/// stays clear of a game's object addresses).
fn id_hash(menu: &str, id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in menu.bytes().chain([0]).chain(id.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h | 1 << 63
}

/// A menu's button as drawn now and in its other states (see
/// [`Menus::looks`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonLooks {
    pub id: u64,
    pub shown: scene::LookState,
    pub sprites: Vec<FreeSprite>,
    pub states: Vec<(scene::LookState, Vec<FreeSprite>)>,
}

/// Pointer state of an open menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub hover: Option<String>,
    pub pressed: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_desc::{Description, ImageLoader};
    use tile_render::RgbaImage;

    struct Solid;
    impl ImageLoader for Solid {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Ok(vec![RgbaImage::from_fn(64, 16, |x, _| [x as u8, 0, 0, 255])])
        }
    }

    fn compiled() -> Compiled {
        Description::parse(
            r#"
            grid = { layout = "orthogonal", step = [8, 8] }
            [images.ui]
            source = "ui"
            [sheets.buttons]
            image = "ui"
            cell = [32, 16]
            [sheets.font]
            image = "ui"
            cell = [8, 8]
            grid = [2, 1]
            group = "font"
            first_index = 65
            "#,
        )
        .unwrap()
        .compile(&Solid, 256)
        .unwrap()
    }

    const MENUS: &str = r#"
        [fonts.default]
        glyphs = "font"
        height = 8
        advance = 6
        [menus.m]
        size = [100, 50]
        anchor = "center"
        [[menus.m.items]]
        button = "ok"
        at = [10, 10]
        size = [32, 16]
        normal = "buttons/0"
        hover = "buttons/1"
        text = "AB"
    "#;

    #[test]
    fn places_hits_and_clicks() {
        let c = compiled();
        let menus = Menus::parse(MENUS, &c).unwrap();
        let m = menus.menu("m").unwrap();
        let viewport = (300, 150);
        // Centred: origin (100, 50); button at (110, 60) 32x16.
        assert_eq!(menus.hit(m, viewport, 110, 60), Some("ok"));
        assert_eq!(menus.hit(m, viewport, 142, 60), None);
        let mut state = MenuState::default();
        let normal = menus.sprites(m, &state, viewport, 9);
        // Button art + two glyphs, the label centred: (32 - 12) / 2 = 10.
        assert_eq!(normal.len(), 3);
        assert_eq!((normal[0].x, normal[0].y), (110, 60));
        assert_eq!((normal[1].x, normal[1].y, normal[2].x), (120, 64, 126));
        assert!(normal.iter().all(|s| s.screen && s.after_pass == 9));
        menus.input(m, &mut state, viewport, &Input::MouseMove { x: 115, y: 65 });
        let hover = menus.sprites(m, &state, viewport, 9);
        assert_ne!(hover[0].set, normal[0].set);
        let down = Input::MouseButton { button: Button::Left, down: true, x: 115, y: 65 };
        let up = Input::MouseButton { button: Button::Left, down: false, x: 116, y: 66 };
        assert_eq!(menus.input(m, &mut state, viewport, &down), None);
        assert_eq!(menus.input(m, &mut state, viewport, &up), Some(MenuEvent::Clicked("ok".into())));
        // Released elsewhere: no click.
        menus.input(m, &mut state, viewport, &down);
        let away = Input::MouseButton { button: Button::Left, down: false, x: 5, y: 5 };
        assert_eq!(menus.input(m, &mut state, viewport, &away), None);
    }

    #[test]
    fn buttons_are_interactables_where_they_are_hit() {
        let c = compiled();
        let menus = Menus::parse(MENUS, &c).unwrap();
        let m = menus.menu("m").unwrap();
        let items = menus.interactables(m, (300, 150));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rect, [110, 60, 32, 16]);
        assert_eq!(items[0].kind, scene::InteractableKind::Button);
        let [x, y, w, h] = items[0].rect;
        assert_eq!(menus.hit(m, (300, 150), x + w - 1, y + h - 1), Some("ok"));
        // The same button keeps its id wherever the menu is.
        assert_eq!(menus.interactables(m, (640, 480))[0].id, items[0].id);
        assert_ne!(id_hash("m", "ok"), id_hash("m", "cancel"));
        assert_ne!(id_hash("a", "bc"), id_hash("ab", "c"));
        // Its looks: as drawn now (a part of the menu's sprites) and the
        // other states.
        let state = MenuState { hover: Some("ok".into()), pressed: None };
        let looks = menus.looks(m, &state, (300, 150), u32::MAX);
        assert_eq!(looks.len(), 1);
        assert_eq!(looks[0].id, items[0].id);
        assert_eq!(looks[0].shown.pointer, scene::PointerState::Hover);
        let all = menus.sprites(m, &state, (300, 150), u32::MAX);
        assert!(all.windows(looks[0].sprites.len()).any(|w| w == &looks[0].sprites[..]));
        let normal = &looks[0].states.iter().find(|(l, _)| l.pointer == scene::PointerState::Normal).unwrap().1;
        let all_normal = menus.sprites(m, &MenuState::default(), (300, 150), u32::MAX);
        assert!(all_normal.windows(normal.len()).any(|w| w == &normal[..]));
    }
}
