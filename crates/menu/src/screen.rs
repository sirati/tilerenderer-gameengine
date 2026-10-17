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
/// Palette entries taken from another image: `count` from `first` on (an
/// image drawn through another colour table).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaletteFrom {
    /// The image, as the loader names it.
    pub image: String,
    pub first: u32,
    pub count: u32,
}

impl std::fmt::Display for PaletteFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{},{}", self.image, self.first, self.count)
    }
}

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
    /// Palette entries taken from another image, if drawn through another
    /// colour table.
    pub palette: Option<PaletteFrom>,
}

/// One element of a screen, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Image { region: Region, at: (i32, i32), clip: [i32; 4] },
    /// The same atlas region with a uniform opacity, applied at draw time.
    OpacityImage { region: Region, at: (i32, i32), clip: [i32; 4], opacity: u8 },
    /// `rect`: x, y, width, height.
    /// `alpha` 255: opaque.
    Fill { rect: [i32; 4], rgb: [u8; 3], alpha: u8, clip: [i32; 4] },
    /// Region `rect` (x, y, w, h) of a picture made at run time (`name`,
    /// `size`), whose pixels come with the scene (see [`patch`]).
    Dynamic { name: String, size: (u32, u32), rect: [i32; 4], at: (i32, i32), clip: [i32; 4] },
    /// A sprite of the description by name, its top-left at `at` (art the
    /// description has anyway, e.g. animation frames). `scale_den`: 0 for
    /// an ordinary sprite, else a scale blend ([`FreeSprite::scale_den`]);
    /// `palette`: the named palette its indexed art's remapped indices are
    /// drawn with (empty: none).
    Sprite { name: String, at: (i32, i32), clip: [i32; 4], scale_den: u32, palette: String },
}


impl Element {
    /// Whether the current compiled art can draw this visible element.
    /// Empty or fully clipped elements need no art. A requested palette
    /// must exist rather than silently falling back to the sprite's own.
    pub fn ready(&self, compiled: &Compiled) -> bool {
        if self.is_empty() { return true; }
        let (name, clip, bounds) = match self {
            Element::Image { region, at, clip } | Element::OpacityImage { region, at, clip, .. } =>
                (region_sprite(region), clip, Some((at.0, at.1, region.rect[2], region.rect[3]))),
            Element::Fill { rect, rgb, alpha, clip } => {
                if *alpha == 0 { return true; }
                (fill_sprite([rgb[0], rgb[1], rgb[2], *alpha], (rect[2] as u32, rect[3] as u32)), clip, Some((rect[0], rect[1], rect[2], rect[3])))
            }
            Element::Dynamic { name, size, rect, at, clip } => {
                if size.0 == 0 || size.1 == 0 { return true; }
                (dynamic_sprite(name, *size), clip, Some((at.0, at.1, rect[2], rect[3])))
            }
            Element::Sprite { name, clip, palette, .. } => {
                if clip[0] >= clip[2] || clip[1] >= clip[3] { return true; }
                if !palette.is_empty() && compiled.palette(palette).is_none() { return false; }
                (name.clone(), clip, None)
            }
        };
        if clip[0] >= clip[2] || clip[1] >= clip[3] { return true; }
        if let Some((x, y, w, h)) = bounds {
            if i64::from(x) >= i64::from(clip[2]) || i64::from(y) >= i64::from(clip[3])
                || i64::from(x) + i64::from(w) <= i64::from(clip[0]) || i64::from(y) + i64::from(h) <= i64::from(clip[1]) { return true; }
        }
        compiled.set_available(&name)
    }

    /// Whether it covers no pixels (nothing to draw or describe).
    pub fn is_empty(&self) -> bool {
        match self {
            Element::Image { region, .. } => region.rect[2] <= 0 || region.rect[3] <= 0,
            Element::OpacityImage { region, opacity, .. } => *opacity == 0 || region.rect[2] <= 0 || region.rect[3] <= 0,
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
                Element::Image { region, .. } | Element::OpacityImage { region, .. } => self.regions.insert(region.clone()),
                Element::Fill { rect, rgb, alpha, .. } => {
                    self.fills.insert(([rgb[0], rgb[1], rgb[2], *alpha], (rect[2].max(0) as u32, rect[3].max(0) as u32)))
                }
                Element::Dynamic { name, size, .. } => self.dynamic.insert((name.clone(), *size)),
                Element::Sprite { .. } => false,
            };
        }
        new
    }

    /// What this has that `other` does not.
    pub fn minus(&self, other: &ScreenArt) -> ScreenArt {
        ScreenArt {
            regions: self.regions.difference(&other.regions).cloned().collect(),
            fills: self.fills.difference(&other.fills).cloned().collect(),
            dynamic: self.dynamic.difference(&other.dynamic).cloned().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.fills.is_empty() && self.dynamic.is_empty()
    }

    /// One line per region, fill and run-time picture (tab-separated
    /// fields), for keeping a list of what screens showed (see
    /// [`ScreenArt::parse_lines`]).
    pub fn to_lines(&self) -> String {
        let mut out = String::new();
        for r in &self.regions {
            let [x, y, w, h] = r.rect;
            let (mask, (mx, my)) = r.mask.as_ref().map_or(("-", (0, 0)), |(m, d)| (m.as_str(), *d));
            let (palette, first, count) = r.palette.as_ref().map_or(("-", 0, 0), |p| (p.image.as_str(), p.first, p.count));
            let _ = writeln!(out, "region\t{}\t{x},{y},{w},{h}\t{}\t{mask}\t{mx},{my}\t{palette}\t{first},{count}", r.source, if r.keyed { "k" } else { "o" });
        }
        for ([r, g, b, a], (w, h)) in &self.fills {
            let _ = writeln!(out, "fill\t{r},{g},{b},{a}\t{w}x{h}");
        }
        for (name, (w, h)) in &self.dynamic {
            let _ = writeln!(out, "dynamic\t{name}\t{w}x{h}");
        }
        out
    }

    /// The art of the lines of [`ScreenArt::to_lines`] among `text` (other
    /// lines are left out).
    pub fn parse_lines(text: &str) -> ScreenArt {
        fn nums<T: std::str::FromStr>(s: &str, sep: char) -> Option<Vec<T>> {
            s.split(sep).map(|v| v.trim().parse().ok()).collect()
        }
        let mut out = ScreenArt::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["region", source, rect, keyed, mask, offset, palette, range] => {
                    let (Some(rect), Some(offset), Some(range)) = (nums::<i32>(rect, ','), nums::<i32>(offset, ','), nums::<u32>(range, ',')) else { continue };
                    let (Ok(rect), [mx, my], [first, count]) = (<[i32; 4]>::try_from(rect), offset.as_slice(), range.as_slice()) else { continue };
                    out.regions.insert(Region {
                        source: source.to_string(),
                        rect,
                        keyed: *keyed == "k",
                        mask: (*mask != "-").then(|| (mask.to_string(), (*mx, *my))),
                        palette: (*palette != "-").then(|| PaletteFrom { image: palette.to_string(), first: *first, count: *count }),
                    });
                }
                ["fill", rgba, size] => {
                    let (Some(c), Some(wh)) = (nums::<u8>(rgba, ','), nums::<u32>(size, 'x')) else { continue };
                    if let (Ok(c), [w, h]) = (<[u8; 4]>::try_from(c), wh.as_slice()) {
                        out.fills.insert((c, (*w, *h)));
                    }
                }
                ["dynamic", name, size] => {
                    if let Some([w, h]) = nums::<u32>(size, 'x').as_deref().and_then(|v| <[u32; 2]>::try_from(v).ok()) {
                        out.dynamic.insert((name.to_string(), (w, h)));
                    }
                }
                _ => {}
            }
        }
        out
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
        type Key<'a> = (&'a str, bool, Option<&'a (String, (i32, i32))>, Option<&'a PaletteFrom>);
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
                let _ = writeln!(mask, "palette = {}\npalette_range = [{}, {}]", toml_str(&p.image), p.first, p.count);
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
                let id = compiled.sprite(name)?;
                compiled.atlas.sprites.get(id.0 as usize).map(|s| (s.anchor_x, s.anchor_y))
            };
            let (scale_den, palette) = match e {
                Element::Sprite { scale_den, palette, .. } => (*scale_den, if palette.is_empty() { 0 } else { compiled.palette(palette).unwrap_or(0) }),
                _ => (0, 0),
            };
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
                Element::Image { region, at, clip } | Element::OpacityImage { region, at, clip, .. } => (region_sprite(region), *at, *clip),
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
                tint: match e { Element::OpacityImage { opacity, .. } => (255 - u32::from(*opacity)) << 24, _ => 0 },
                zoom_art: 0,
                zoom: (0, 0),
                zoom_offset: (0, 0),
                clip: Some(clip),
                not_in_view: None,
                glyph: None,
                palette: palette,
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

    #[test]
    fn art_lists_round_trip() {
        let mut art = ScreenArt::default();
        art.regions.insert(Region { source: "Art/Advisors/a b.pcx".into(), rect: [1, -2, 30, 40], keyed: true, mask: None, palette: None });
        art.regions.insert(Region {
            source: "Art/x.pcx".into(),
            rect: [0, 0, 5, 5],
            keyed: false,
            mask: Some(("Art/m.pcx".into(), (-3, 4))),
            palette: Some(PaletteFrom { image: "Art/p.pcx".into(), first: 16, count: 32 }),
        });
        art.fills.insert(([1, 2, 3, 128], (640, 1)));
        art.dynamic.insert(("minimap".into(), (230, 107)));
        assert_eq!(ScreenArt::parse_lines(&format!("# a comment\n{}font\tLucida\n", art.to_lines())), art);
    }


    struct Plain;
    impl render_desc::ImageLoader for Plain {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Ok(vec![tile_render::RgbaImage::from_fn(64, 64, |x, y| [x as u8, y as u8, 0, 255])])
        }
        fn palette(&self, _: &str, _: &toml::Table) -> Result<Vec<[u8; 4]>, String> {
            Ok(vec![[1, 2, 3, 255]; 256])
        }
    }

    #[test]
    fn readiness_requires_each_element_and_its_requested_palette() {
        let region = Region { source: "panel.pcx".into(), rect: [0, 0, 8, 6], keyed: true, mask: None, palette: None };
        let elements = vec![
            Element::Image { region: region.clone(), at: (1, 2), clip: [0, 0, 80, 60] },
            Element::OpacityImage { region: region.clone(), at: (1, 2), clip: [0, 0, 80, 60], opacity: 128 },
            Element::Fill { rect: [2, 3, 4, 5], rgb: [1, 2, 3], alpha: 255, clip: [0, 0, 80, 60] },
            Element::Dynamic { name: "minimap".into(), size: (8, 6), rect: [0, 0, 8, 6], at: (1, 2), clip: [0, 0, 80, 60] },
            Element::Sprite { name: region_sprite(&region), at: (1, 2), clip: [0, 0, 80, 60], scale_den: 0, palette: String::new() },
        ];
        let compile = |art: &ScreenArt, extra: &str| {
            let text = format!("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n{}\n{extra}", art.description("", "", |_, _| String::new()));
            render_desc::Description::parse(&text).unwrap().compile(&Plain, 256).unwrap()
        };
        let empty = compile(&ScreenArt::default(), "");
        assert!(elements.iter().all(|element| !element.ready(&empty)));
        let mut art = ScreenArt::default();
        art.add(&elements);
        let compiled = compile(&art, "[palettes.loaded]\nsource = \"palette\"\n[sets.empty]\nframes = []\n");
        assert!(elements.iter().all(|element| element.ready(&compiled)));
        assert!(elements.iter().all(|element| element.ready(&compiled.without_pixels())));
        let mut sprite = elements[4].clone();
        if let Element::Sprite { palette, .. } = &mut sprite { *palette = "missing".into(); }
        assert!(!sprite.ready(&compiled), "a named palette must not silently fall back to row zero");
        if let Element::Sprite { palette, .. } = &mut sprite { *palette = "loaded".into(); }
        assert!(sprite.ready(&compiled));
        if let Element::Sprite { name, palette, .. } = &mut sprite { *name = "empty".into(); palette.clear(); }
        assert!(!sprite.ready(&compiled), "an empty named animation has no drawable frame");
    }

    #[test]
    fn oversized_opaque_screen_art_is_not_ready_because_its_placeholder_exists() {
        let region = Region { source: "solid:255,10,20,255:8x2".into(), rect: [0, 0, 8, 2], keyed: false, mask: None, palette: None };
        let image = Element::Image { region: region.clone(), at: (0, 0), clip: [0, 0, 8, 2] };
        let sprite = Element::Sprite { name: region_sprite(&region), at: (0, 0), clip: [0, 0, 8, 2], scale_den: 0, palette: String::new() };
        let mut art = ScreenArt::default();
        art.add(std::slice::from_ref(&image));
        let description = render_desc::Description::parse(&format!("grid = {{ layout = \"orthogonal\", step = [1, 1] }}\n{}", art.description("", "", |_, _| String::new()))).unwrap();
        let small = description.compile(&Plain, 4).unwrap();
        assert!(small.set(&region_sprite(&region)).is_some(), "oversized placeholders retain their names and ids");
        assert!(!image.ready(&small) && !sprite.ready(&small));
        assert!(!image.ready(&small.without_pixels()));
        let complete = description.compile(&Plain, 16).unwrap();
        assert!(image.ready(&complete) && sprite.ready(&complete));
    }

    #[test]
    fn invisible_elements_do_not_wait_for_art() {
        let empty = render_desc::Description::parse("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n").unwrap().compile(&Plain, 256).unwrap();
        let region = Region { source: "missing.pcx".into(), rect: [0, 0, 8, 6], keyed: true, mask: None, palette: None };
        let elements = [
            Element::Image { region: Region { rect: [0, 0, 0, 6], ..region.clone() }, at: (0, 0), clip: [0, 0, 80, 60] },
            Element::Image { region: region.clone(), at: (100, 0), clip: [0, 0, 80, 60] },
            Element::OpacityImage { region: region.clone(), at: (0, 0), clip: [0, 0, 80, 60], opacity: 0 },
            Element::Fill { rect: [0, 0, 8, 6], rgb: [1, 2, 3], alpha: 0, clip: [0, 0, 80, 60] },
            Element::Dynamic { name: "missing".into(), size: (8, 6), rect: [0, 0, 0, 6], at: (0, 0), clip: [0, 0, 80, 60] },
            Element::Sprite { name: "missing".into(), at: (0, 0), clip: [5, 5, 5, 8], scale_den: 0, palette: "missing".into() },
        ];
        assert!(elements.iter().all(|element| element.ready(&empty)));
        let visible = Element::Image { region, at: (i32::MAX - 2, 0), clip: [i32::MAX - 3, 0, i32::MAX, 10] };
        assert!(!visible.ready(&empty), "visibility arithmetic must not overflow at screen coordinate limits");
    }

    #[test]
    fn fading_images_share_one_atlas_region() {
        let region = Region { source: "bubble.pcx".into(), rect: [0, 0, 16, 16], keyed: true, mask: None, palette: None };
        let elements: Vec<_> = (1..=255).map(|opacity| Element::OpacityImage { region: region.clone(), at: (0, 0), clip: [0, 0, 16, 16], opacity }).collect();
        let mut art = ScreenArt::default();
        assert!(art.add(&elements));
        assert_eq!(art.regions.len(), 1);
        let text = format!("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n{}", art.description("", "", |_, _| String::new()));
        let compiled = render_desc::Description::parse(&text).unwrap().compile(&Plain, 256).unwrap();
        let draws = sprites(&elements, &compiled, u32::MAX);
        assert_eq!(draws.len(), 255);
        assert!(draws.iter().all(|draw| draw.set == draws[0].set));
        for (draw, opacity) in draws.iter().zip(1..=255_u32) {
            assert_eq!(255 - (draw.tint >> 24), opacity);
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
