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

/// How an [`Element::Indexed`] region's palette indices are drawn, where
/// the blit that drew it treats them other than an ordinary colour-keyed
/// one does. With `shade` empty, the indices in `transparent` are not
/// drawn and the others in their (palette's) colours. Otherwise the region
/// darkens what is below it instead of covering it: index `i` to `k`/16 of
/// each colour channel for each `(i, k)` in `shade` (a scale blend); the
/// other indices are not drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Indices {
    /// A selected frame of an animated indexed source.
    pub frame: Option<u32>,
    /// Native fixed-point resize, applied before any destination clip.
    pub resize: Option<IndexedResize>,
    /// Palette colours weighted by their original palette indices / 256.
    pub palette_alpha: Option<PaletteAlpha>,
    pub transparent: Vec<u8>,
    pub shade: Vec<(u8, u8)>,
}

/// A palette-driven blend ramp. Entries matching `skip` are filler. The
/// remaining colours, in palette order, map to `first..first + count`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaletteAlpha {
    pub source: String,
    pub first: u8,
    pub count: u8,
    pub skip: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexedResize {
    pub size: (u32, u32),
    pub sample_step: [u32; 2],
}

impl Indices {
    pub fn size(&self, rect: [i32; 4]) -> (i32, i32) {
        self.resize.map_or((rect[2], rect[3]), |r| (r.size.0 as i32, r.size.1 as i32))
    }
    /// Loader options of an image drawn so.
    fn options(&self) -> String {
        let list = |v: &mut dyn Iterator<Item = u8>| v.map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
        if self.shade.is_empty() {
            let mut out = format!("transparent = [{}]", list(&mut self.transparent.iter().copied()));
            if let Some(p) = &self.palette_alpha {
                out.push_str(&format!("\npalette_alpha = {{ source = {}, first = {}, count = {}, skip = {:?} }}", toml_str(&p.source), p.first, p.count, p.skip));
            }
            return out;
        }
        let shaded = |i: u8| self.shade.iter().any(|&(s, _)| s == i);
        let rgba = self.shade.iter().map(|(i, k)| format!("{i} = [{k}, 0, 0, 255]")).collect::<Vec<_>>().join(", ");
        format!("transparent = [{}]\nrgba = {{ {rgba} }}", list(&mut (0..=255u8).filter(|&i| !shaded(i))))
    }

    /// A short name of the rule, for sprite names.
    fn key(&self) -> String {
        let t: Vec<String> = self.transparent.iter().map(u8::to_string).collect();
        let s: Vec<String> = self.shade.iter().map(|(i, k)| format!("{i}:{k}")).collect();
        let mut key = format!("!{}*{}", t.join(","), s.join(","));
        if let Some(frame) = self.frame { key.push_str(&format!("!frame{frame}")); }
        if let Some(p) = &self.palette_alpha { key.push_str(&format!("!alpha{}:{},{},{:?}", p.source, p.first, p.count, p.skip)); }
        if let Some(r) = self.resize { key.push_str(&format!("!resize{}x{}:{},{}", r.size.0, r.size.1, r.sample_step[0], r.sample_step[1])); }
        key
    }
}

/// One element of a screen, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Image { region: Region, at: (i32, i32), clip: [i32; 4] },
    /// The same atlas region with a uniform opacity, applied at draw time.
    OpacityImage { region: Region, at: (i32, i32), clip: [i32; 4], opacity: u8 },
    /// A file region resized once in the atlas to `size`. Opacity is
    /// applied at draw time; the clip never changes the resize samples.
    /// `sample_step` gives the native 16.16 source increments.
    ResizedImage { region: Region, at: (i32, i32), size: (i32, i32), sample_step: [u32; 2], clip: [i32; 4], opacity: u8 },
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
    /// An existing smoke-only sprite with raw palette-index /256 alpha.
    PaletteAlphaSprite { name: String, at: (i32, i32), clip: [i32; 4] },
    /// A file region whose palette indices are drawn as `indices` says
    /// (e.g. a shadow darkening what is below it). The region's `keyed`
    /// and `mask` play no part.
    Indexed { region: Region, indices: Indices, at: (i32, i32), clip: [i32; 4] },
}


impl Element {
    /// Whether the current compiled art can draw this visible element.
    /// Empty or fully clipped elements need no art. A requested palette
    /// must exist rather than silently falling back to the sprite's own.
    pub fn ready(&self, compiled: &Compiled) -> bool {
        self.ready_with_backgrounds(compiled, |_| Vec::new())
    }

    /// Readiness with the same source-region policy used by description
    /// and draw generation. A partial split must be complete as a pair.
    pub fn ready_with_backgrounds(&self, compiled: &Compiled, policy: impl Fn(&Region) -> Vec<[i32; 4]>) -> bool {
        if self.is_empty() { return true; }
        let (name, clip, bounds) = match self {
            Element::Image { region, at, clip } | Element::OpacityImage { region, at, clip, .. } =>
                (region_sprite(region), clip, Some((at.0, at.1, region.rect[2], region.rect[3]))),
            Element::ResizedImage { region, at, size, sample_step, clip, .. } =>
                (resized_region_sprite(region, (size.0 as u32, size.1 as u32), *sample_step), clip, Some((at.0, at.1, size.0, size.1))),
            Element::Fill { .. } => return true,
            Element::Dynamic { name, size, rect, at, clip } => {
                if size.0 == 0 || size.1 == 0 { return true; }
                (dynamic_sprite(name, *size), clip, Some((at.0, at.1, rect[2], rect[3])))
            }
            Element::PaletteAlphaSprite { name, clip, .. } => (name.clone(), clip, None),
            Element::Sprite { name, clip, palette, .. } => {
                if clip[0] >= clip[2] || clip[1] >= clip[3] { return true; }
                if !palette.is_empty() && compiled.palette(palette).is_none() { return false; }
                (name.clone(), clip, None)
            }
            Element::Indexed { region, indices, at, clip } => {
                let (w, h) = indices.size(region.rect);
                (indexed_sprite(region, indices), clip, Some((at.0, at.1, w, h)))
            },
        };
        if clip[0] >= clip[2] || clip[1] >= clip[3] { return true; }
        if let Some((x, y, w, h)) = bounds {
            if i64::from(x) >= i64::from(clip[2]) || i64::from(y) >= i64::from(clip[3])
                || i64::from(x) + i64::from(w) <= i64::from(clip[0]) || i64::from(y) + i64::from(h) <= i64::from(clip[1]) { return true; }
        }
        compiled.set_available(&name) && match self {
            Element::Image { region, .. } | Element::OpacityImage { region, .. } if matches!(background(region, policy(region)), Background::Partial(_)) => compiled.set_available(&background_sprite(region)),
            _ => true,
        }
    }

    /// Whether it covers no pixels (nothing to draw or describe).
    pub fn is_empty(&self) -> bool {
        match self {
            Element::Image { region, .. } => region.rect[2] <= 0 || region.rect[3] <= 0,
            Element::OpacityImage { region, opacity, .. } => *opacity == 0 || region.rect[2] <= 0 || region.rect[3] <= 0,
            Element::ResizedImage { region, size, opacity, .. } => *opacity == 0 || region.rect[2] <= 0 || region.rect[3] <= 0 || size.0 <= 0 || size.1 <= 0,
            Element::Fill { rect, .. } | Element::Dynamic { rect, .. } => rect[2] <= 0 || rect[3] <= 0,
            Element::Sprite { .. } | Element::PaletteAlphaSprite { .. } => false,
            Element::Indexed { region, indices, .. } => region.rect[2] <= 0 || region.rect[3] <= 0 || indices.size(region.rect).0 <= 0 || indices.size(region.rect).1 <= 0,
        }
    }
}
/// The art a set of screens needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenArt {
    pub regions: BTreeSet<Region>,
    /// Source regions, destination sizes and 16.16 sample increments.
    pub resized_regions: BTreeSet<(Region, (u32, u32), [u32; 2])>,
    /// Fill colours (RGBA) and sizes.
    pub fills: BTreeSet<([u8; 4], (u32, u32))>,
    /// Run-time pictures: name and size.
    pub dynamic: BTreeSet<(String, (u32, u32))>,
    /// Regions drawn by their palette indices' own rules (see
    /// [`Element::Indexed`]); the regions' `keyed` true and `mask` none.
    pub indexed: BTreeSet<(Region, Indices)>,
}

/// The sprite name of a region.
fn region_sprite(r: &Region) -> String {
    let [x, y, w, h] = r.rect;
    let mask = r.mask.as_ref().map(|(m, (dx, dy))| format!("%{m}+{dx},{dy}")).unwrap_or_default();
    let palette = r.palette.as_ref().map(|p| format!("~{p}")).unwrap_or_default();
    format!("Screen@{}#{x},{y},{w},{h}{}{mask}{palette}/0", r.source, if r.keyed { "k" } else { "" })
}

/// The sprite name of an [`Element::Indexed`] region.
fn indexed_sprite(r: &Region, indices: &Indices) -> String {
    let [x, y, w, h] = r.rect;
    let palette = r.palette.as_ref().map(|p| format!("~{p}")).unwrap_or_default();
    format!("ScreenIndexed@{}#{x},{y},{w},{h}{}{palette}/0", r.source, indices.key())
}

/// The region an [`Element::Indexed`] is described by (its `keyed` and
/// `mask` play no part).
fn indexed_region(r: &Region) -> Region {
    Region { keyed: true, mask: None, ..r.clone() }
}

fn resized_region_sprite(r: &Region, (w, h): (u32, u32), [sx, sy]: [u32; 2]) -> String {
    format!("{}>{w}x{h}@{sx},{sy}/0", region_sprite(r).trim_end_matches("/0"))
}

/// The coverage of an explicit background policy in one image crop.
/// Partial coverage is split by complementary alpha masks on the original
/// pixel grid. Geometry and input bounds stay unchanged at every scale.
#[derive(Debug, PartialEq, Eq)]
enum Background {
    None,
    Whole,
    Partial(Vec<[u32; 4]>),
}

fn background(region: &Region, rectangles: Vec<[i32; 4]>) -> Background {
    let [x, y, w, h] = region.rect;
    if w <= 0 || h <= 0 { return Background::None; }
    let mut kept = Vec::new();
    for [tx, ty, tw, th] in rectangles {
        if tw <= 0 || th <= 0 { continue; }
        let (l, t) = (i64::from(x).max(i64::from(tx)), i64::from(y).max(i64::from(ty)));
        let (r, b) = ((i64::from(x) + i64::from(w)).min(i64::from(tx) + i64::from(tw)), (i64::from(y) + i64::from(h)).min(i64::from(ty) + i64::from(th)));
        if l >= r || t >= b { continue; }
        let rect = [(l - i64::from(x)) as u32, (t - i64::from(y)) as u32, (r - l) as u32, (b - t) as u32];
        if rect == [0, 0, w as u32, h as u32] { return Background::Whole; }
        if !kept.contains(&rect) { kept.push(rect); }
    }
    if kept.is_empty() { Background::None } else { Background::Partial(kept) }
}

fn background_sprite(region: &Region) -> String {
    format!("{}#background/0", region_sprite(region).trim_end_matches("/0"))
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
                Element::ResizedImage { region, size, sample_step, .. } => self.resized_regions.insert((region.clone(), (size.0 as u32, size.1 as u32), *sample_step)),
                Element::Fill { .. } => false,
                Element::Dynamic { name, size, .. } => self.dynamic.insert((name.clone(), *size)),
                Element::Sprite { .. } | Element::PaletteAlphaSprite { .. } => false,
                Element::Indexed { region, indices, .. } => self.indexed.insert((indexed_region(region), indices.clone())),
            };
        }
        new
    }

    /// What this has that `other` does not.
    pub fn minus(&self, other: &ScreenArt) -> ScreenArt {
        ScreenArt {
            regions: self.regions.difference(&other.regions).cloned().collect(),
            resized_regions: self.resized_regions.difference(&other.resized_regions).cloned().collect(),
            fills: self.fills.difference(&other.fills).cloned().collect(),
            dynamic: self.dynamic.difference(&other.dynamic).cloned().collect(),
            indexed: self.indexed.difference(&other.indexed).cloned().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.resized_regions.is_empty() && self.fills.is_empty() && self.dynamic.is_empty() && self.indexed.is_empty()
    }

    /// One line per region, fill and run-time picture (tab-separated
    /// fields), for keeping a list of what screens showed (see
    /// [`ScreenArt::parse_lines`]).
    pub fn to_lines(&self) -> String {
        let mut out = String::new();
        for (r, resized) in self.regions.iter().map(|r| (r, None)).chain(self.resized_regions.iter().map(|(r, size, step)| (r, Some((size, step))))) {
            let [x, y, w, h] = r.rect;
            let (mask, (mx, my)) = r.mask.as_ref().map_or(("-", (0, 0)), |(m, d)| (m.as_str(), *d));
            let (palette, first, count) = r.palette.as_ref().map_or(("-", 0, 0), |p| (p.image.as_str(), p.first, p.count));
            let kind = if resized.is_some() { "resized_region" } else { "region" };
            let _ = write!(out, "{kind}\t{}\t{x},{y},{w},{h}\t{}\t{mask}\t{mx},{my}\t{palette}\t{first},{count}", r.source, if r.keyed { "k" } else { "o" });
            if let Some(((w, h), [sx, sy])) = resized { let _ = write!(out, "\t{w}x{h}\t{sx},{sy}"); }
            out.push('\n');
        }
        for (r, indices) in &self.indexed {
            let [x, y, w, h] = r.rect;
            let (palette, first, count) = r.palette.as_ref().map_or(("-", 0, 0), |p| (p.image.as_str(), p.first, p.count));
            let transparent = indices.transparent.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
            let shade = indices.shade.iter().map(|(i, k)| format!("{i}:{k}")).collect::<Vec<_>>().join(",");
            let _ = write!(out, "indexed\t{}\t{x},{y},{w},{h}\t{palette}\t{first},{count}\t{transparent}\t{shade}", r.source);
            if indices.resize.is_some() || indices.palette_alpha.is_some() || indices.frame.is_some() {
                let resize = indices.resize.map_or_else(|| "-".into(), |r| format!("{}, {}, {}, {}", r.size.0, r.size.1, r.sample_step[0], r.sample_step[1]).replace(' ', ""));
                let alpha = indices.palette_alpha.as_ref().map_or_else(|| "-".into(), |p| format!("{},{},{},{},{}", p.first, p.count, p.skip[0], p.skip[1], p.skip[2]));
                let source = indices.palette_alpha.as_ref().map_or("-", |p| p.source.as_str());
                let _ = write!(out, "\t{resize}\t{alpha}\t{source}");
                if let Some(frame) = indices.frame { let _ = write!(out, "\t{frame}"); }
            }
            out.push('\n');
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
                ["region", source, rect, keyed, mask, offset, palette, range]
                | ["resized_region", source, rect, keyed, mask, offset, palette, range, _, _] => {
                    let (Some(rect), Some(offset), Some(range)) = (nums::<i32>(rect, ','), nums::<i32>(offset, ','), nums::<u32>(range, ',')) else { continue };
                    let (Ok(rect), [mx, my], [first, count]) = (<[i32; 4]>::try_from(rect), offset.as_slice(), range.as_slice()) else { continue };
                    let region = Region {
                        source: source.to_string(),
                        rect,
                        keyed: *keyed == "k",
                        mask: (*mask != "-").then(|| (mask.to_string(), (*mx, *my))),
                        palette: (*palette != "-").then(|| PaletteFrom { image: palette.to_string(), first: *first, count: *count }),
                    };
                    if let Some(size) = f.get(8) {
                        if let Some([w, h]) = nums::<u32>(size, 'x').as_deref().and_then(|v| <[u32; 2]>::try_from(v).ok()) {
                            if let Some([sx, sy]) = f.get(9).and_then(|v| nums::<u32>(v, ',')).as_deref().and_then(|v| <[u32; 2]>::try_from(v).ok()) {
                                if w > 0 && h > 0 { out.resized_regions.insert((region, (w, h), [sx, sy])); }
                            }
                        }
                    } else {
                        out.regions.insert(region);
                    }
                }
                ["indexed", source, rect, palette, range, transparent, shade, extra @ ..] => {
                    let (Some(rect), Some(range)) = (nums::<i32>(rect, ','), nums::<u32>(range, ',')) else { continue };
                    let (Ok(rect), [first, count]) = (<[i32; 4]>::try_from(rect), range.as_slice()) else { continue };
                    let transparent = if transparent.is_empty() { Some(Vec::new()) } else { nums::<u8>(transparent, ',') };
                    let shade = if shade.is_empty() { Some(Vec::new()) } else {
                        shade.split(',').map(|pair| {
                            let values = nums::<u8>(pair, ':')?;
                            let [index, level] = values.as_slice() else { return None };
                            (*level <= 16).then_some((*index, *level))
                        }).collect::<Option<Vec<_>>>()
                    };
                    let (Some(transparent), Some(shade)) = (transparent, shade) else { continue };
                    let region = Region {
                        source: source.to_string(), rect, keyed: true, mask: None,
                        palette: (*palette != "-").then(|| PaletteFrom { image: palette.to_string(), first: *first, count: *count }),
                    };
                    let mut indices = Indices { transparent, shade, ..Default::default() };
                    match extra {
                        [] => {},
                        [resize, alpha, source] | [resize, alpha, source, _] => {
                            if let [_, _, _, frame] = extra {
                                let Ok(frame) = frame.parse() else { continue };
                                indices.frame = Some(frame);
                            }
                            if *resize != "-" {
                                let Some(v) = nums::<u32>(resize, ',') else { continue };
                                let [w, h, sx, sy] = v.as_slice() else { continue };
                                if *w == 0 || *h == 0 || *w > i32::MAX as u32 || *h > i32::MAX as u32 { continue; }
                                indices.resize = Some(IndexedResize { size: (*w, *h), sample_step: [*sx, *sy] });
                            }
                            if *alpha != "-" {
                                let Some(v) = nums::<u8>(alpha, ',') else { continue };
                                let [first, count, r, g, b] = v.as_slice() else { continue };
                                if *count == 0 || u16::from(*first) + u16::from(*count) > 256 || *source == "-" { continue; }
                                indices.palette_alpha = Some(PaletteAlpha { source: source.to_string(), first: *first, count: *count, skip: [*r, *g, *b] });
                            }
                        },
                        _ => continue,
                    }
                    out.indexed.insert((region, indices));
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
        other.regions.is_subset(&self.regions) && other.resized_regions.is_subset(&self.resized_regions) && other.fills.is_subset(&self.fills) && other.dynamic.is_subset(&self.dynamic)
            && other.indexed.is_subset(&self.indexed)
    }

    /// Render description entries for this art. `keyed_options` /
    /// `opaque_options`: loader options (TOML lines) of keyed and opaque
    /// images, e.g. which palette entries are transparent; `mask_options`:
    /// those blending an image through a mask (file, offset).
    pub fn description(&self, keyed_options: &str, opaque_options: &str, mask_options: impl Fn(&str, (i32, i32)) -> String) -> String {
        self.description_with_backgrounds(keyed_options, opaque_options, mask_options, |_| Vec::new())
    }

    /// As [`Self::description`], with an explicit caller-owned policy of
    /// pure source background rectangles. Mixed pictures retain one pixel
    /// grid and split into complementary nearest/background alpha masks.
    pub fn description_with_backgrounds(&self, keyed_options: &str, opaque_options: &str, mask_options: impl Fn(&str, (i32, i32)) -> String, policy: impl Fn(&Region) -> Vec<[i32; 4]>) -> String {
        let mut out = String::from("\n# Screen art (generated).\n");
        type Key<'a> = (&'a str, bool, Option<&'a (String, (i32, i32))>, Option<&'a PaletteFrom>);
        let images: BTreeSet<Key> = self.regions.iter().chain(self.resized_regions.iter().map(|(r, _, _)| r)).map(|r| (r.source.as_str(), r.keyed, r.mask.as_ref(), r.palette.as_ref())).collect();
        let index: std::collections::HashMap<Key, usize> = images.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        for (n, r) in self.regions.iter().enumerate() {
            let image = format!("screen_{}{}", index[&(r.source.as_str(), r.keyed, r.mask.as_ref(), r.palette.as_ref())], if r.keyed { "k" } else { "o" });
            let [x, y, w, h] = r.rect;
            let _ = writeln!(
                out,
                "[sheets.screen_region_{n}]\nimage = \"{image}\"\norigin = [{x}, {y}]\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n",
                toml_str(region_sprite(r).trim_end_matches("/0"))
            );
            match background(r, policy(r)) {
                Background::None => {},
                Background::Whole => out.push_str("sampling = \"ui-background\"\n\n"),
                Background::Partial(rectangles) => {
                    let _ = writeln!(out, "alpha_clear = {rectangles:?}\n");
                    let _ = writeln!(out, "[sheets.screen_background_region_{n}]\nimage = \"{image}\"\norigin = [{x}, {y}]\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\nsampling = \"ui-background\"\nalpha_keep = {rectangles:?}\n", toml_str(background_sprite(r).trim_end_matches("/0")));
                },
            }
        }
        for (n, (r, (dw, dh), [sx, sy])) in self.resized_regions.iter().enumerate() {
            let image = format!("screen_{}{}", index[&(r.source.as_str(), r.keyed, r.mask.as_ref(), r.palette.as_ref())], if r.keyed { "k" } else { "o" });
            let [x, y, w, h] = r.rect;
            let _ = writeln!(
                out,
                "[sheets.screen_resized_region_{n}]\nimage = \"{image}\"\norigin = [{x}, {y}]\ncell = [{w}, {h}]\ngrid = [1, 1]\nresize = [{dw}, {dh}]\nresize_step = [{sx}, {sy}]\ngroup = {}\nscaling = \"keep\"\n",
                toml_str(resized_region_sprite(r, (*dw, *dh), [*sx, *sy]).trim_end_matches("/0"))
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
        // One image per file, colours and rule; a sheet per region.
        let indexed_images: BTreeSet<(&str, Option<&PaletteFrom>, &Indices)> = self.indexed.iter().map(|(r, i)| (r.source.as_str(), r.palette.as_ref(), i)).collect();
        let indexed_index: std::collections::HashMap<_, usize> = indexed_images.iter().enumerate().map(|(n, k)| (*k, n)).collect();
        for (n, (r, indices)) in self.indexed.iter().enumerate() {
            let [x, y, w, h] = r.rect;
            let _ = writeln!(
                out,
                "[sheets.screen_indexed_region_{n}]\nimage = \"screen_indexed_{}\"\norigin = [{x}, {y}]\ncell = [{w}, {h}]\ngrid = [1, 1]\ngroup = {}\nscaling = \"keep\"\n",
                indexed_index[&(r.source.as_str(), r.palette.as_ref(), indices)],
                toml_str(indexed_sprite(r, indices).trim_end_matches("/0"))
            );
            if let Some(frame) = indices.frame { let _ = writeln!(out, "frame = {frame}\n"); }
            if let Some(r) = indices.resize {
                let _ = writeln!(out, "resize = [{}, {}]\nresize_step = [{}, {}]\n", r.size.0, r.size.1, r.sample_step[0], r.sample_step[1]);
            }
        }
        for (n, (source, palette, indices)) in indexed_images.iter().enumerate() {
            let mut palette = palette.map(|p| format!("palette = {}\npalette_range = [{}, {}]\n", toml_str(&p.image), p.first, p.count)).unwrap_or_default();
            // The existing atlas resizer consumes RGBA. Loader overrides retain
            // raw palette-alpha channels before that nearest-neighbor resize.
            if indices.resize.is_some() { palette.push_str("indexed = false\n"); }
            if let Some(frame) = indices.frame { let _ = writeln!(palette, "frames = [{frame}]"); }
            let _ = writeln!(out, "[images.screen_indexed_{n}]\nsource = {}\n{}\n{palette}", toml_str(source), indices.options());
        }
        out
    }
}

/// The free sprites drawing `elements` (screen space) after render pass
/// `after_pass` (`u32::MAX`: on top of everything); elements whose art is
/// not compiled are left out.
pub fn sprites(elements: &[Element], compiled: &Compiled, after_pass: u32) -> Vec<FreeSprite> {
    sprites_with_backgrounds(elements, compiled, after_pass, |_| Vec::new())
}

/// Draws the same elements, splitting only the source regions explicitly
/// named by `policy`. Both halves retain the original position, clip,
/// opacity and order before following controls or text.
pub fn sprites_with_backgrounds(elements: &[Element], compiled: &Compiled, after_pass: u32, policy: impl Fn(&Region) -> Vec<[i32; 4]>) -> Vec<FreeSprite> {
    let mut out = Vec::new();
    let ordinary: Vec<_> = elements
        .iter()
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            if let Element::Fill { rect, rgb, alpha, clip } = e {
                if *alpha == 0 { return None; }
                return Some((e, FreeSprite {
                    x: rect[0], y: rect[1], screen: true, after_pass, clip: Some(*clip),
                    solid: Some(scene::SolidRect { width: rect[2] as u32, height: rect[3] as u32, rgba: [rgb[0], rgb[1], rgb[2], *alpha] }),
                    ..Default::default()
                }));
            }
            // Sprites are placed by their anchor; elements give their top-left.
            let anchor = |name: &str| {
                let id = compiled.sprite(name)?;
                compiled.atlas.sprites.get(id.0 as usize).map(|s| (s.anchor_x, s.anchor_y))
            };
            let (scale_den, palette) = match e {
                Element::Sprite { scale_den, palette, .. } => (*scale_den, if palette.is_empty() { 0 } else { compiled.palette(palette).unwrap_or(0) }),
                // A shade: the scale blend its rule's levels are sixteenths of.
                Element::Indexed { indices, .. } if !indices.shade.is_empty() => (16, 0),
                _ => (0, 0),
            };
            let (name, (x, y), clip) = match e {
                Element::Sprite { name, at: (x, y), clip, .. } | Element::PaletteAlphaSprite { name, at: (x, y), clip } => {
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
                Element::Indexed { region, indices, at, clip } => (indexed_sprite(region, indices), *at, *clip),
                Element::ResizedImage { region, at, size, sample_step, clip, .. } => (resized_region_sprite(region, (size.0 as u32, size.1 as u32), *sample_step), *at, *clip),
                Element::Fill { .. } => return None,
            };
            let (set, _) = compiled.set(&name)?;
            Some((e, FreeSprite {
                set: set.raw(),
                x,
                y,
                screen: true,
                after_pass,
                phase: 0,
                ticks_per_frame: 1,
                scale_den,
                blend: if matches!(e, Element::PaletteAlphaSprite { .. }) || matches!(e, Element::Indexed { indices, .. } if indices.palette_alpha.is_some()) { scene::SpriteBlend::PaletteAlpha256 } else { scene::SpriteBlend::Alpha },
                tint: match e { Element::OpacityImage { opacity, .. } | Element::ResizedImage { opacity, .. } => (255 - u32::from(*opacity)) << 24, _ => 0 },
                zoom_art: 0,
                zoom: (0, 0),
                zoom_offset: (0, 0),
                clip: Some(clip),
                not_in_view: None,
                glyph: None, solid: None, output_rect: None,
                palette: palette,
                category: 0,
            }))
        })
        .collect();
    for (element, sprite) in ordinary {
        if let Element::Image { region, .. } | Element::OpacityImage { region, .. } = element {
            if matches!(background(region, policy(region)), Background::Partial(_)) {
                let Some((set, _)) = compiled.set(&background_sprite(region)) else { continue };
                out.extend([sprite, FreeSprite { set: set.raw(), ..sprite }]);
                continue;
            }
        }
        out.push(sprite);
    }
    out
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
        art.regions.insert(Region { source: "Art/Reports/a b.pcx".into(), rect: [1, -2, 30, 40], keyed: true, mask: None, palette: None });
        art.regions.insert(Region {
            source: "Art/x.pcx".into(),
            rect: [0, 0, 5, 5],
            keyed: false,
            mask: Some(("Art/m.pcx".into(), (-3, 4))),
            palette: Some(PaletteFrom { image: "Art/p.pcx".into(), first: 16, count: 32 }),
        });
        art.fills.insert(([1, 2, 3, 128], (640, 1)));
        art.dynamic.insert(("minimap".into(), (230, 107)));
        art.resized_regions.insert((art.regions.iter().next().unwrap().clone(), (13, 7), [151236, 187245]));
        assert_eq!(ScreenArt::parse_lines(&format!("# a comment\n{}font\tLucida\n", art.to_lines())), art);
    }


    #[test]
    fn generated_background_metadata_preserves_copies_crops_and_preload_round_trip() {
        let background = Region {
            source: "example-background.pcx".into(),
            rect: [0, 0, 1023, 327], keyed: false, mask: None, palette: None,
        };
        let cropped = Region { rect: [17, 29, 43, 31], ..background.clone() };
        let masked = Region { mask: Some(("mask.pcx".into(), (3, 5))), ..cropped.clone() };
        let button = Region { source: "example-controls.pcx".into(), rect: [2, 3, 24, 18], keyed: true, mask: None, palette: None };
        let elements = [
            Element::Image { region: background.clone(), at: (0, 0), clip: [0, 0, 1023, 327] },
            Element::Image { region: cropped.clone(), at: (100, 200), clip: [100, 200, 143, 231] },
            Element::OpacityImage { region: cropped.clone(), at: (300, 400), clip: [305, 406, 333, 426], opacity: 128 },
            Element::Image { region: masked.clone(), at: (10, 20), clip: [0, 0, 1023, 327] },
            Element::Image { region: button.clone(), at: (10, 20), clip: [0, 0, 1023, 327] },
            Element::ResizedImage { region: cropped.clone(), at: (10, 20), size: (30, 20), sample_step: [93934, 101580], clip: [0, 0, 1023, 327], opacity: 255 },
        ];
        let mut art = ScreenArt::default();
        art.add(&elements);
        assert_eq!(art.regions.len(), 4, "copies share the source region regardless of placement, clipping and opacity");
        let recovered = ScreenArt::parse_lines(&art.to_lines());
        assert_eq!(recovered, art);
        let description = |art: &ScreenArt| art.description_with_backgrounds("transparent = [254, 255]", "transparent = []", |source, (x, y)| {
            format!("mask = {}\nmask_offset = [{x}, {y}]", toml_str(source))
        }, |region| if region.source == "example-background.pcx" && region.mask.is_none() && region.palette.is_none() { vec![[0, 0, 1023, 327]] } else { Vec::new() });
        let generated = description(&art);
        assert_eq!(description(&recovered), generated, "preload art and a live capture classify the same source crops");
        let table: toml::Table = generated.parse().unwrap();
        let sheets = table["sheets"].as_table().unwrap();
        let sheet = |name: String| sheets.values().find(|sheet| sheet["group"].as_str() == Some(name.trim_end_matches("/0"))).unwrap();
        assert_eq!(sheet(region_sprite(&background))["sampling"].as_str(), Some("ui-background"));
        let crop_sheet = sheet(region_sprite(&cropped));
        assert_eq!(crop_sheet["sampling"].as_str(), Some("ui-background"));
        assert_eq!(crop_sheet["origin"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [17, 29]);
        assert_eq!(crop_sheet["cell"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [43, 31]);
        for name in [region_sprite(&masked), region_sprite(&button), resized_region_sprite(&cropped, (30, 20), [93934, 101580])] {
            assert!(sheet(name).get("sampling").is_none(), "masked, mixed and already resized art keep nearest sampling");
        }
        let images = table["images"].as_table().unwrap();
        let masked_image = &images[sheet(region_sprite(&masked))["image"].as_str().unwrap()];
        assert_eq!(masked_image["mask"].as_str(), Some("mask.pcx"));
        assert_eq!(masked_image["mask_offset"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [3, 5]);
        let text = format!("grid = {{ layout = \"orthogonal\", step = [1, 1] }}\n{generated}");
        assert!(render_desc::Description::parse(&text).is_ok());
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
        assert!(elements.iter().all(|element| matches!(element, Element::Fill { .. }) == element.ready(&empty)));
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
    fn resized_palette_alpha_rules_round_trip_and_keep_native_samples_and_typed_mode() {
        let region = Region { source: "piece picture.pcx".into(), rect: [8, 4, 9, 5], keyed: true, mask: None, palette: None };
        let resize = Some(IndexedResize { size: (5, 3), sample_step: [117964, 109226] });
        let smoke = Indices { frame: Some(0), transparent: (0..=255).collect(), resize, palette_alpha: Some(PaletteAlpha { source: "Art/Smoke Palette.pcx".into(), first: 224, count: 16, skip: [0, 0, 255] }), ..Default::default() };
        let shades = Indices { shade: vec![(249, 10)], resize, ..Default::default() };
        let elements = [Element::Indexed { region: region.clone(), indices: smoke.clone(), at: (2, 3), clip: [4, 3, 7, 6] }, Element::Indexed { region, indices: shades, at: (9, 3), clip: [9, 3, 14, 6] }];
        let mut art = ScreenArt::default();
        art.add(&elements);
        let mut original = smoke.clone(); original.resize = None;
        if let Element::Indexed { region, .. } = &elements[0] { art.indexed.insert((indexed_region(region), original)); }
        let recovered = ScreenArt::parse_lines(&art.to_lines());
        assert_eq!(recovered, art);
        let description = art.description("", "", |_, _| String::new());
        assert!(description.contains("resize = [5, 3]"));
        assert!(description.contains("resize_step = [117964, 109226]"));
        assert!(description.contains("palette_alpha = { source = \"Art/Smoke Palette.pcx\", first = 224, count = 16, skip = [0, 0, 255] }"));
        let compiled = render_desc::Description::parse(&format!("[grid]\nlayout = 'orthogonal'\nstep = [1, 1]\n{description}")).unwrap().compile(&Plain, 256).unwrap();
        let sprites = sprites(&elements, &compiled, 0);
        assert_eq!(sprites.len(), 2);
        assert_eq!(sprites[0].blend, scene::SpriteBlend::PaletteAlpha256);
        assert_eq!(sprites[1].blend, scene::SpriteBlend::Alpha);
        assert_eq!(sprites[1].scale_den, 16);
        let id = compiled.sprite(&indexed_sprite(match &elements[0] { Element::Indexed { region, .. } => region, _ => unreachable!() }, &smoke)).unwrap();
        let sprite = compiled.atlas.sprites[id.0 as usize];
        assert_eq!((sprite.width, sprite.height), (5, 3));
        let page = &compiled.atlas.pages[sprite.page as usize];
        for y in 0..3 { for x in 0..5 { assert_eq!(page.get(sprite.atlas_x + x, sprite.atlas_y + y), [(8 + x * 117964 / 65536) as u8, (4 + y * 109226 / 65536) as u8, 0, 255]); } }
    }

    #[test]
    fn indexed_art_survives_preload_lists_with_palette_and_blend_rules() {
        let region = Region { source: "Territory.pcx".into(), rect: [3, 7, 48, 49], keyed: true, mask: None,
            palette: Some(PaletteFrom { image: "ntp03.pcx".into(), first: 64, count: 2 }) };
        let elements = [
            Element::Indexed { region: region.clone(), indices: Indices { transparent: vec![0, 1, 224, 255], shade: vec![], ..Default::default() }, at: (12, 13), clip: [0, 0, 100, 100] },
            Element::Indexed { region, indices: Indices { transparent: vec![], shade: vec![(248, 8), (249, 10)], ..Default::default() }, at: (12, 13), clip: [0, 0, 100, 100] },
        ];
        let mut art = ScreenArt::default();
        art.add(&elements);
        let recovered = ScreenArt::parse_lines(&art.to_lines());
        assert_eq!(recovered, art);
        assert_eq!(recovered.description("", "", |_, _| String::new()), art.description("", "", |_, _| String::new()));
        assert!(ScreenArt::parse_lines("indexed\tx\t0,0,1,1\t-\t0,0\t\t248:17\n").is_empty());
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
    fn resized_regions_cache_sizes_and_keep_cropped_pixels_transparency_and_opacity() {
        struct Icons;
        impl render_desc::ImageLoader for Icons {
            fn load(&self, name: &str, options: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
                assert_eq!(name, "icon.pcx");
                assert_eq!(options["transparent"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [254, 255]);
                assert_eq!(options["palette"].as_str(), Some("colors.pcx"));
                assert_eq!(options["palette_range"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [16, 32]);
                assert_eq!(options["mask"].as_str(), Some("mask.pcx"));
                assert_eq!(options["mask_offset"].as_array().unwrap().iter().map(|v| v.as_integer().unwrap()).collect::<Vec<_>>(), [3, 4]);
                Ok(vec![tile_render::RgbaImage::from_fn(32, 32, |x, y| [x as u8, y as u8, 17, if x == 11 && y == 5 { 0 } else { 255 }])])
            }
        }
        let region = Region {
            source: "icon.pcx".into(), rect: [8, 4, 9, 5], keyed: true,
            mask: Some(("mask.pcx".into(), (3, 4))),
            palette: Some(PaletteFrom { image: "colors.pcx".into(), first: 16, count: 32 }),
        };
        let elements = [
            Element::ResizedImage { region: region.clone(), at: (100, 50), size: (5, 3), sample_step: [117964, 109226], clip: [102, 51, 105, 53], opacity: 128 },
            Element::ResizedImage { region: region.clone(), at: (200, 50), size: (5, 3), sample_step: [117964, 109226], clip: [0, 0, 300, 100], opacity: 64 },
            Element::ResizedImage { region: region.clone(), at: (0, 0), size: (7, 4), sample_step: [84260, 81920], clip: [0, 0, 300, 100], opacity: 255 },
            Element::ResizedImage { region: region.clone(), at: (0, 0), size: (5, 3), sample_step: [65536, 65536], clip: [0, 0, 300, 100], opacity: 255 },
        ];
        let mut art = ScreenArt::default();
        assert!(art.add(&elements));
        assert!(!art.add(&elements));
        assert_eq!(art.resized_regions.len(), 3);
        assert!(art.regions.is_empty());
        assert!(!art.is_empty());
        assert_eq!(ScreenArt::parse_lines(&art.to_lines()), art);
        let mut first = ScreenArt::default();
        first.add(&elements[..2]);
        let remaining = art.minus(&first);
        assert_eq!(remaining.resized_regions.len(), 2);
        assert!(art.covers(&first) && !first.covers(&art));
        assert!(art.covers(&remaining));
        let text = format!("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n{}", art.description("transparent = [254, 255]", "transparent = []", |name, (x, y)| format!("mask = \"{name}\"\nmask_offset = [{x}, {y}]\n")));
        let desc = render_desc::Description::parse(&text).unwrap();
        assert_eq!(desc.images.len(), 1, "sizes and opacity share the decoded file");
        let compiled = desc.compile(&Icons, 256).unwrap();
        assert!(elements.iter().all(|element| element.ready(&compiled) && element.ready(&compiled.without_pixels())));
        let sprite = compiled.atlas.sprites[compiled.sprite(&resized_region_sprite(&region, (5, 3), [117964, 109226])).unwrap().0 as usize];
        assert_eq!((sprite.width, sprite.height), (5, 3));
        let page = &compiled.atlas.pages[sprite.page as usize];
        for (y, sy) in [4, 5, 7].into_iter().enumerate() {
            for (x, sx) in [8, 9, 11, 13, 15].into_iter().enumerate() {
                assert_eq!(page.get(sprite.atlas_x + x as u32, sprite.atlas_y + y as u32), [sx, sy, 17, if sx == 11 && sy == 5 { 0 } else { 255 }]);
            }
        }
        let draws = sprites(&elements, &compiled, 7);
        let other = compiled.atlas.sprites[compiled.sprite(&resized_region_sprite(&region, (5, 3), [65536, 65536])).unwrap().0 as usize];
        assert_eq!(compiled.atlas.pages[other.page as usize].get(other.atlas_x + 2, other.atlas_y), [10, 4, 17, 255], "equal dimensions with different steps need different cached pixels");
        assert_eq!(draws.len(), 4);
        assert_eq!(draws[0].set, draws[1].set);
        assert_ne!(draws[0].set, draws[2].set);
        assert_ne!(draws[0].set, draws[3].set);
        assert_eq!((draws[0].x, draws[0].y, draws[0].clip, draws[0].after_pass), (100, 50, Some([102, 51, 105, 53]), 7));
        assert_eq!(draws.iter().map(|s| 255 - (s.tint >> 24)).collect::<Vec<_>>(), [128, 64, 255, 255]);
    }

    #[test]
    fn resized_visibility_and_readiness_use_destination_size() {
        let empty = render_desc::Description::parse("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n").unwrap().compile(&Plain, 256).unwrap();
        let region = Region { source: "icon.pcx".into(), rect: [0, 0, 8, 6], keyed: true, mask: None, palette: None };
        let element = |size, opacity, at| Element::ResizedImage { region: region.clone(), size, sample_step: [26214, 39321], opacity, at, clip: [0, 0, 100, 100] };
        for invisible in [element((0, 3), 255, (0, 0)), element((5, -1), 255, (0, 0)), element((5, 3), 0, (0, 0)), element((5, 3), 255, (-5, 0))] {
            assert!(invisible.ready(&empty));
        }
        let visible = element((20, 10), 255, (-9, 0));
        assert!(!visible.ready(&empty), "the resized image reaches the clip although its source size would not");
        let mut art = ScreenArt::default();
        art.add(std::slice::from_ref(&visible));
        let desc = render_desc::Description::parse(&format!("grid = {{ layout = \"orthogonal\", step = [1, 1] }}\n{}", art.description("", "", |_, _| String::new()))).unwrap();
        assert!(!visible.ready(&desc.compile(&Plain, 16).unwrap()), "an oversized placeholder cannot draw the resized image");
        assert!(visible.ready(&desc.compile(&Plain, 64).unwrap()));
    }

    #[test]
    fn fills_emit_without_requesting_or_compiling_any_art() {
        let fill = Element::Fill { rect: [5, 6, 700, 800], rgb: [1, 2, 3], alpha: 128, clip: [10, 20, 40, 50] };
        let mut art = ScreenArt::default();
        assert!(!art.add(&[fill.clone()]));
        assert!(art.is_empty());
        assert!(!art.description("", "", |_, _| String::new()).contains("solid:"));
        let compiled = render_desc::Description::parse("[grid]\nlayout = \"orthogonal\"\nstep = [1, 1]\n").unwrap().compile(&Plain, 16).unwrap();
        assert!(fill.ready(&compiled));
        let commands = sprites(&[fill], &compiled, 9);
        assert_eq!(commands.len(), 1);
        assert_eq!((commands[0].x, commands[0].y, commands[0].after_pass, commands[0].clip), (5, 6, 9, Some([10, 20, 40, 50])));
        assert_eq!(commands[0].solid, Some(scene::SolidRect { width: 700, height: 800, rgba: [1, 2, 3, 128] }));
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

#[cfg(test)]
#[path = "screen/background_tests.rs"]
mod background_tests;
