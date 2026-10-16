//! Screens composed at run time: regions of image files, solid fills and
//! glyph text at screen positions, each clipped to a rectangle — what a
//! mirror of another program's interface produces frame by frame. The
//! art they need is generated as a render description, so only regions
//! actually shown are loaded.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use render_desc::Compiled;
use scene::FreeSprite;

/// A region of an image file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Region {
    /// Image source as the loader names it.
    pub source: String,
    /// x, y, width, height.
    pub rect: [i32; 4],
    /// Whether the image's transparent colours are transparent (or the
    /// region is opaque).
    pub keyed: bool,
    /// An alpha mask image and the offset from a region pixel to its mask
    /// pixel, if blended through one.
    pub mask: Option<(String, (i32, i32))>,
    /// An image whose palette replaces the source's (it is drawn through
    /// another colour table), as the loader names it.
    pub palette: Option<String>,
}

/// One element of a screen, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Image { region: Region, at: (i32, i32), clip: [i32; 4] },
    /// `rect`: x, y, width, height.
    /// `alpha` 255: opaque.
    Fill { rect: [i32; 4], rgb: [u8; 3], alpha: u8, clip: [i32; 4] },
    /// Region `rect` (x, y, w, h) of a picture made at run time (`name`,
    /// `size`), whose pixels come with the scene (see [`patch`]).
    Dynamic { name: String, size: (u32, u32), rect: [i32; 4], at: (i32, i32), clip: [i32; 4] },
    /// A sprite of the description by name, its top-left at `at` (art the
    /// description has anyway, e.g. animation frames). `scale_den`: 0 for
    /// an ordinary sprite, else a scale blend ([`FreeSprite::scale_den`]).
    Sprite { name: String, at: (i32, i32), clip: [i32; 4], scale_den: u32 },
}


impl Element {
    /// Whether it covers no pixels (nothing to draw or describe).
    pub fn is_empty(&self) -> bool {
        match self {
            Element::Image { region, .. } => region.rect[2] <= 0 || region.rect[3] <= 0,
            Element::Fill { rect, .. } | Element::Dynamic { rect, .. } => rect[2] <= 0 || rect[3] <= 0,
            Element::Sprite { .. } => false,
        }
    }
}
/// The art a set of screens needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenArt {
    pub regions: BTreeSet<Region>,
    /// Fill colours (RGBA) and sizes.
    pub fills: BTreeSet<([u8; 4], (u32, u32))>,
    /// Run-time pictures: name and size.
    pub dynamic: BTreeSet<(String, (u32, u32))>,
}

/// The sprite name of a region.
fn region_sprite(r: &Region) -> String {
    let [x, y, w, h] = r.rect;
    let mask = r.mask.as_ref().map(|(m, (dx, dy))| format!("%{m}+{dx},{dy}")).unwrap_or_default();
    let palette = r.palette.as_ref().map(|p| format!("~{p}")).unwrap_or_default();
    format!("Screen@{}#{x},{y},{w},{h}{}{mask}{palette}/0", r.source, if r.keyed { "k" } else { "" })
}

fn fill_sprite(c: [u8; 4], (w, h): (u32, u32)) -> String {
    format!("ScreenFill@{},{},{},{}#{w}x{h}/0", c[0], c[1], c[2], c[3])
}

fn dynamic_sprite(name: &str, (w, h): (u32, u32)) -> String {
    format!("ScreenDynamic@{name}#{w}x{h}/0")
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl ScreenArt {
    /// Adds what `elements` need; whether anything was new.
    pub fn add(&mut self, elements: &[Element]) -> bool {
        let mut new = false;
        for e in elements.iter().filter(|e| !e.is_empty()) {
            new |= match e {
                Element::Image { region, .. } => self.regions.insert(region.clone()),
                Element::Fill { rect, rgb, alpha, .. } => {
                    self.fills.insert(([rgb[0], rgb[1], rgb[2], *alpha], (rect[2].max(0) as u32, rect[3].max(0) as u32)))
                }
                Element::Dynamic { name, size, .. } => self.dynamic.insert((name.clone(), *size)),
                Element::Sprite { .. } => false,
            };
        }
        new
    }

    /// Whether everything `other` needs is here.
    pub fn covers(&self, other: &ScreenArt) -> bool {
        other.regions.is_subset(&self.regions) && other.fills.is_subset(&self.fills) && other.dynamic.is_subset(&self.dynamic)
    }

    /// Render description entries for this art. `keyed_options` /
    /// `opaque_options`: loader options (TOML lines) of keyed and opaque
    /// images, e.g. which palette entries are transparent; `mask_options`:
    /// those blending an image through a mask (file, offset).
    pub fn description(&self, keyed_options: &str, opaque_options: &str, mask_options: impl Fn(&str, (i32, i32)) -> String) -> String {
        let mut out = String::from("\n# Screen art (generated).\n");
        type Key<'a> = (&'a str, bool, Option<&'a (String, (i32, i32))>, Option<&'a String>);
        let images: BTreeSet<Key> = self.regions.iter().map(|r| (r.source.as_str(), r.keyed, r.mask.as_ref(), r.palette.as_ref())).collect();
        let index: std::collections::HashMap<Key, usize> = images.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        for (n, r) in self.regions.iter().enumerate() {
            let image = format!("screen_{}{}", index[&(r.source.as_str(), r.keyed, r.mask.as_ref(), r.palette.as_ref())], if r.keyed { "k" } else { "o" });
            let [x, y, w, h] = r.rect;
            let _ = writeln!(
                out,
                "[sheets.screen_region_{n}]\nimage = \"{image}\"\norigin = [{x}, {y}]\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n",
                toml_str(region_sprite(r).trim_end_matches("/0"))
            );
        }
        for (i, (source, keyed, mask, palette)) in images.iter().enumerate() {
            let options = if *keyed { keyed_options } else { opaque_options };
            let mut mask = mask.map(|(m, d)| mask_options(m, *d)).unwrap_or_default();
            if let Some(p) = palette {
                let _ = writeln!(mask, "palette = {}", toml_str(p));
            }
            let _ = writeln!(out, "[images.screen_{i}{}]\nsource = {}\n{options}\n{mask}\n", if *keyed { "k" } else { "o" }, toml_str(source));
        }
        for (n, (name, (w, h))) in self.dynamic.iter().enumerate() {
            let _ = writeln!(
                out,
                "[images.screen_dynamic_{n}]\nsource = \"solid:0,0,0,255:{w}x{h}\"\n\n[sheets.screen_dynamic_{n}]\nimage = \"screen_dynamic_{n}\"\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n",
                toml_str(dynamic_sprite(name, (*w, *h)).trim_end_matches("/0"))
            );
        }
        for (n, &([r, g, b, a], (w, h))) in self.fills.iter().enumerate() {
            let _ = writeln!(
                out,
                "[images.screen_fill_{n}]\nsource = \"solid:{r},{g},{b},{a}:{w}x{h}\"\n\n[sheets.screen_fill_{n}]\nimage = \"screen_fill_{n}\"\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n",
                toml_str(fill_sprite([r, g, b, a], (w, h)).trim_end_matches("/0"))
            );
        }
        out
    }
}

/// The free sprites drawing `elements` (screen space) after render pass
/// `after_pass` (`u32::MAX`: on top of everything); elements whose art is
/// not compiled are left out.
pub fn sprites(elements: &[Element], compiled: &Compiled, after_pass: u32) -> Vec<FreeSprite> {
    elements
        .iter()
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            // Sprites are placed by their anchor; elements give their top-left.
            let anchor = |name: &str| {
                let id = compiled.sprite_names.iter().position(|n| n == name)?;
                compiled.atlas.sprites.get(id).map(|s| (s.anchor_x, s.anchor_y))
            };
            let scale_den = if let Element::Sprite { scale_den, .. } = e { *scale_den } else { 0 };
            let (name, (x, y), clip) = match e {
                Element::Sprite { name, at: (x, y), clip, .. } => {
                    let (ax, ay) = anchor(name).unwrap_or((0, 0));
                    (name.clone(), (x - ax, y - ay), *clip)
                }
                // The whole picture, moved so the region lands at `at`, clipped
                // to the region.
                Element::Dynamic { name, size, rect: [rx, ry, rw, rh], at: (x, y), clip } => {
                    let region = [*x, *y, x + rw, y + rh];
                    let clip = [clip[0].max(region[0]), clip[1].max(region[1]), clip[2].min(region[2]), clip[3].min(region[3])];
                    (dynamic_sprite(name, *size), (x - rx, y - ry), clip)
                }
                Element::Image { region, at, clip } => (region_sprite(region), *at, *clip),
                Element::Fill { rect, rgb, alpha, clip } => {
                    (fill_sprite([rgb[0], rgb[1], rgb[2], *alpha], (rect[2].max(0) as u32, rect[3].max(0) as u32)), (rect[0], rect[1]), *clip)
                }
            };
            let (set, _) = compiled.set(&name)?;
            Some(FreeSprite {
                set: set.raw(),
                x,
                y,
                screen: true,
                after_pass,
                phase: 0,
                ticks_per_frame: 1,
                scale_den,
                tint: 0,
                zoom_art: 0,
                zoom: (0, 0),
                zoom_offset: (0, 0),
                clip: Some(clip),
                not_in_view: None,
            })
        })
        .collect()
}


/// The scene patch giving run-time picture `name` (`size`) its pixels
/// (row-major RGBA), if the description has it.
pub fn patch(compiled: &Compiled, name: &str, size: (u32, u32), rgba: Vec<u8>) -> Option<scene::Patch> {
    let sprite = dynamic_sprite(name, size);
    compiled.sprite_names.contains(&sprite).then_some(())?;
    (rgba.len() == (size.0 * size.1 * 4) as usize).then(|| scene::Patch { sprite, width: size.0, height: size.1, rgba })
}
#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl render_desc::ImageLoader for Plain {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Ok(vec![tile_render::RgbaImage::from_fn(64, 64, |x, y| [x as u8, y as u8, 0, 255])])
        }
    }

    #[test]
    fn screens_compile_and_place_their_art() {
        let region = |source: &str, keyed| Region { source: source.into(), rect: [8, 4, 16, 10], keyed, mask: None, palette: None };
        let elements = vec![
            Element::Image { region: region("b.pcx", true), at: (100, 50), clip: [0, 0, 640, 480] },
            Element::Image { region: region("a.pcx", false), at: (3, 4), clip: [0, 0, 10, 10] },
            Element::Fill { rect: [5, 6, 7, 8], rgb: [1, 2, 3], alpha: 128, clip: [0, 0, 640, 480] },
        ];
        let mut art = ScreenArt::default();
        assert!(art.add(&elements));
        assert!(!art.add(&elements));
        let text = format!("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n{}", art.description("", "", |_, _| String::new()));
        let compiled = render_desc::Description::parse(&text).unwrap().compile(&Plain, 256).unwrap();
        let sprites = sprites(&elements, &compiled, u32::MAX);
        assert_eq!(sprites.len(), 3);
        assert_eq!((sprites[1].x, sprites[1].y, sprites[1].clip), (3, 4, Some([0, 0, 10, 10])));
        assert!(sprites.iter().all(|s| s.screen));
    }
}
