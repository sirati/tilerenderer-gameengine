//! Render descriptions: what a game's art files contain and how they are
//! drawn, written in TOML instead of hard-coded.
//!
//! A description is game agnostic. It names *images* (loaded by an
//! [`ImageLoader`] the game supplies, e.g. a PCX reader), slices them into
//! *sheets* of sprites, groups sprites into *sets* (one frame = static,
//! several = an animation), and declares the *layers* of the tile world. It
//! compiles into a `tile_render` atlas, sprite sets and layer table; a driver
//! then fills cells by set *name*, resolved once to numbers with
//! [`Compiled::value`].
//!
//! ```toml
//! [grid]
//! layout = "staggered"   # or "orthogonal"
//! step = [64, 32]
//!
//! [images.grassland]
//! source = "Art/Terrain/xtgc.pcx"
//! transparent = [255]            # loader-specific options are free-form
//!
//! [sheets.xtgc]
//! image = "grassland"
//! cell = [128, 64]               # sprite size
//! grid = [9, 9]                  # columns, rows; sprites "xtgc/0" .. "xtgc/80"
//! anchor = [0, 0]                # draw offset from the cell origin
//!
//! [sets."river/3"]
//! frames = ["mtnrivers/3"]       # sprite names; several = animation
//!
//! [[layers]]
//! name = "terrain"
//! sets = ["xtgc/*"]              # sets drawable on this layer; `*` expands
//! ```

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use tile_render::{AtlasBuilder, Blend, ColorModel, Grid, LayerSpec, Layout, RgbaImage, SpriteId, SpriteSetId, SpriteSets};

#[derive(Debug, thiserror::Error)]
pub enum DescError {
    #[error("TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("image {name:?}: {message}")]
    Image { name: String, message: String },
    #[error("unknown {kind} {name:?}")]
    Unknown { kind: &'static str, name: String },
    #[error("sheet {sheet:?}: {message}")]
    Sheet { sheet: String, message: String },
    #[error(transparent)]
    Atlas(#[from] tile_render::atlas::AtlasError),
}

/// Loads the images a description names. `options` are the image's TOML
/// keys other than `source`, for the loader to interpret.
pub trait ImageLoader {
    fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String>;
}

/// The parsed TOML.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Description {
    pub grid: GridDesc,
    /// Options passed to the loader for every image (an image's own keys
    /// override them).
    #[serde(default)]
    pub loader: toml::Table,
    /// Options for the load that finds a sprite's box when its sheet says
    /// `sample_from = "box"` (see [`SampleFrom`]): they override `loader`
    /// and the image's own, and the pixels the loader then makes transparent
    /// (alpha 0) are the blank ones. A game's sprite format can call fewer
    /// pixels blank than its renderer draws transparent.
    #[serde(default)]
    pub blank_loader: toml::Table,
    #[serde(default)]
    pub images: BTreeMap<String, ImageDesc>,
    #[serde(default)]
    pub sheets: BTreeMap<String, SheetDesc>,
    #[serde(default)]
    pub sets: BTreeMap<String, SetDesc>,
    #[serde(default)]
    pub layers: Vec<LayerDesc>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridDesc {
    pub layout: LayoutDesc,
    pub step: [u32; 2],
    /// Compose in channels of this many bits (4..=7), e.g. 5 for a game
    /// with an RGB555 frame buffer; see `tile_render::ColorModel`.
    #[serde(default)]
    pub color_bits: Option<u32>,
}

#[derive(Copy, Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayoutDesc {
    Orthogonal,
    Staggered,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImageDesc {
    pub source: String,
    /// Everything else, for the loader — except `blank = { ... }`: loader
    /// options for the load that finds sprite boxes (see
    /// [`Description::blank_loader`]), over that table's, for an image the
    /// game's sprite format treats differently (e.g. one whose palette
    /// entries 0 and 1 it clears first).
    #[serde(flatten)]
    pub options: toml::Table,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetDesc {
    pub image: String,
    /// Which of the image's frames (multi-frame sources, e.g. animations).
    #[serde(default)]
    pub frame: usize,
    /// Size of one sprite.
    pub cell: [u32; 2],
    /// Columns × rows of sprites (default: as many as fit).
    pub grid: Option<[u32; 2]>,
    /// Pixel position of sprite (0, 0) in the image.
    #[serde(default)]
    pub origin: [u32; 2],
    /// Gap between sprites.
    #[serde(default)]
    pub spacing: [u32; 2],
    /// Where the sprite's top-left goes relative to the cell origin.
    #[serde(default)]
    pub anchor: [i32; 2],
    /// Only these sprite indices (default: all).
    pub only: Option<Vec<u32>>,
    /// Name sprites `"{group}/{first_index + i}"` instead of
    /// `"{sheet}/{i}"`, so that several files form one numbered collection
    /// (e.g. a game's sprite array spanning nine sheets).
    pub group: Option<String>,
    #[serde(default)]
    pub first_index: u32,
    /// Index step between consecutive sprites of the sheet (2 interleaves
    /// two sheets into one collection).
    #[serde(default = "one")]
    pub index_stride: u32,
    /// Sprites shrunk by this factor: every n-th pixel of every n-th row
    /// (nearest neighbour), size rounded down — how games draw a zoomed-out
    /// view; where from: [`SheetDesc::sample_from`].
    #[serde(default = "one")]
    pub downscale: u32,
    /// Where the shrunk sprite's samples start.
    #[serde(default)]
    pub sample_from: SampleFrom,
    /// How a zoomed-out view (see [`Description::scaled_down`]) treats it.
    #[serde(default)]
    pub scaling: SheetScaling,
    /// Each sprite replaced by a band `width` pixels wide just outside its
    /// art, in `color` (a ring around a ring); the sprite grows by `width` on
    /// every side, its anchor moves accordingly.
    pub outline: Option<OutlineDesc>,
    /// See [`Description::parse`].
    pub variants: Option<Vec<i64>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutlineDesc {
    pub width: u32,
    pub color: [u8; 4],
}

/// One frame of a set: a sprite name, or `{ sheet, index }`-style shorthand
/// `"sheet/index"`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDesc {
    pub frames: Vec<String>,
    /// Blend when drawn as a free sprite (layers have their own).
    #[serde(default)]
    pub blend: BlendDesc,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerDesc {
    pub name: String,
    /// Set names drawable on this layer; a value is an index into this list
    /// after expansion. `"sheet/*"` expands to every sprite of a sheet as a
    /// single-frame set named `"sheet/i"`; `"prefix*"` matches named sets.
    pub sets: Vec<String>,
    #[serde(default)]
    pub offset: [i32; 2],
    #[serde(default = "one")]
    pub ticks_per_frame: u32,
    #[serde(default)]
    pub per_cell_phase: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Render pass; layers must be listed in pass order. Each pass sweeps all
    /// visible cells before the next one starts.
    #[serde(default)]
    pub pass: u32,
    /// Point (relative to the cell origin) a camera cull rectangle is
    /// tested against.
    #[serde(default)]
    pub cull_offset: [i32; 2],
    /// How the layer combines with what is below; all layers of a pass
    /// must agree.
    #[serde(default)]
    pub blend: BlendDesc,
}

/// `blend = "alpha"` (default) or `blend = { scale = 16 }` (the
/// denominator), see `tile_render::Blend`.
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum BlendDesc {
    #[default]
    Alpha,
    Scale(u32),
}

/// `source = "solid:R,G,B,A:WxH"`: a single-colour image, made without the
/// loader (bars, lines).
fn solid(spec: &str) -> Result<RgbaImage, String> {
    let err = || format!("solid image {spec:?}: expected R,G,B,A:WxH");
    let (rgba, size) = spec.split_once(':').ok_or_else(err)?;
    let c: Vec<u8> = rgba.split(',').map(|v| v.trim().parse().map_err(|_| err())).collect::<Result<_, _>>()?;
    let (w, h) = size.split_once('x').ok_or_else(err)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| err())?, h.parse().map_err(|_| err())?);
    let [r, g, b, a] = <[u8; 4]>::try_from(c).map_err(|_| err())?;
    Ok(RgbaImage::from_fn(w, h, |_, _| [r, g, b, a]))
}

/// `source = "arrow:R,G,B,A:WxH"`: an arrow pointing down, `W × H` pixels
/// with its outline (one black pixel around it): a shaft over a head,
/// the tip at the bottom middle.
fn arrow(spec: &str) -> Result<RgbaImage, String> {
    let (color, (w, h)) = shape_spec(spec, "arrow")?;
    let (iw, ih) = (w as i32 - 2, h as i32 - 2);
    let head = ih / 2;
    let inside = |x: i32, y: i32| {
        let (x, y) = (x - 1, y - 1);
        if x < 0 || y < 0 || x >= iw || y >= ih {
            return false;
        }
        let c2 = 2 * x + 1 - iw; // twice the distance from the middle
        if y < ih - head {
            c2.abs() <= iw / 3
        } else {
            // The head narrows from the full width to the tip.
            c2.abs() * head <= iw * (ih - y)
        }
    };
    Ok(outlined(w, h, color, inside))
}

/// `source = "crosshair:R,G,B,A:WxH"`: a cross through the middle with a
/// gap there, outlined like `arrow:`.
fn crosshair(spec: &str) -> Result<RgbaImage, String> {
    let (color, (w, h)) = shape_spec(spec, "crosshair")?;
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let gap = (w.min(h) as i32 / 8).max(1);
    let inside = |x: i32, y: i32| {
        let on_margin = x == 0 || y == 0 || x == w as i32 - 1 || y == h as i32 - 1;
        let horizontal = (y - cy).abs() <= 0 && (x - cx).abs() > gap;
        let vertical = (x - cx).abs() <= 0 && (y - cy).abs() > gap;
        !on_margin && (horizontal || vertical)
    };
    Ok(outlined(w, h, color, inside))
}

fn shape_spec(spec: &str, kind: &str) -> Result<([u8; 4], (u32, u32)), String> {
    let err = || format!("{kind} image {spec:?}: expected R,G,B,A:WxH (at least 4x4)");
    let (rgba, size) = spec.split_once(':').ok_or_else(err)?;
    let c: Vec<u8> = rgba.split(',').map(|v| v.trim().parse().map_err(|_| err())).collect::<Result<_, _>>()?;
    let (w, h) = size.split_once('x').ok_or_else(err)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| err())?, h.parse().map_err(|_| err())?);
    let color = <[u8; 4]>::try_from(c).map_err(|_| err())?;
    if w < 4 || h < 4 {
        return Err(err());
    }
    Ok((color, (w, h)))
}

/// `color` where `inside`, black on the pixels next to those, clear elsewhere.
fn outlined(w: u32, h: u32, color: [u8; 4], inside: impl Fn(i32, i32) -> bool) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if inside(x, y) {
            color
        } else if [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| inside(x + dx, y + dy)) {
            [0, 0, 0, 255]
        } else {
            [0; 4]
        }
    })
}

/// The band `width` pixels wide outside `sprite`'s art (its opaque pixels
/// and what they enclose), in `color`, on an image grown by `width` on
/// every side.
fn outline(sprite: &RgbaImage, width: u32, color: [u8; 4]) -> RgbaImage {
    let t = width as i32;
    let (w, h) = (sprite.width as i32 + 2 * t, sprite.height as i32 + 2 * t);
    let opaque = |x: i32, y: i32| {
        let (sx, sy) = (x - t, y - t);
        sx >= 0 && sy >= 0 && sx < sprite.width as i32 && sy < sprite.height as i32 && sprite.get(sx as u32, sy as u32)[3] != 0
    };
    // Outside: reachable from the border without crossing the art.
    let mut outside = vec![false; (w * h) as usize];
    let mut stack: Vec<(i32, i32)> = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)])).collect();
    while let Some((x, y)) = stack.pop() {
        if x < 0 || y < 0 || x >= w || y >= h || outside[(y * w + x) as usize] || opaque(x, y) {
            continue;
        }
        outside[(y * w + x) as usize] = true;
        stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
    }
    let near_art = |x: i32, y: i32| (-t..=t).any(|dy| (-t..=t).any(|dx| dx * dx + dy * dy <= t * t + t && opaque(x + dx, y + dy)));
    RgbaImage::from_fn(w as u32, h as u32, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if outside[(y * w + x) as usize] && near_art(x, y) { color } else { [0; 4] }
    })
}

/// `source = "line:R,G,B,A:DX,DY:W"`: a line `W` pixels thick from a start
/// point to the point `(DX, DY)` away, mostly horizontal (`|DY| <= |DX|`),
/// rasterized like simple software renderers do: every column from the
/// start's x up to (not including) the end's x — counted `j = 0, 1, ...`
/// from the start — gets `W` pixels from row `j·|DY|/|DX|` (rounded down)
/// on, towards the end. The image's top-left is [`line_origin`] relative to
/// the start, so a sheet with that anchor places the line at its start.
fn line(spec: &str) -> Result<RgbaImage, String> {
    let (color, (dx, dy), w) = line_spec(spec)?;
    let pixels = line_pixels(dx, dy, w);
    let (ox, oy) = line_origin(dx, dy, w);
    let width = pixels.iter().map(|p| p.0 - ox + 1).max().unwrap_or(0) as u32;
    let height = pixels.iter().map(|p| p.1 - oy + 1).max().unwrap_or(0) as u32;
    let mut img = RgbaImage::from_fn(width, height, |_, _| [0; 4]);
    for (x, y) in pixels {
        img.put((x - ox) as u32, (y - oy) as u32, color);
    }
    Ok(img)
}

fn line_spec(spec: &str) -> Result<([u8; 4], (i32, i32), i32), String> {
    let err = || format!("line image {spec:?}: expected R,G,B,A:DX,DY:W with |DY| <= |DX|");
    let mut parts = spec.split(':');
    let mut numbers = |n: usize| -> Result<Vec<i32>, String> {
        let v: Vec<i32> = parts.next().ok_or_else(err)?.split(',').map(|v| v.trim().parse().map_err(|_| err())).collect::<Result<_, _>>()?;
        if v.len() == n { Ok(v) } else { Err(err()) }
    };
    let c = numbers(4)?;
    let d = numbers(2)?;
    let w = numbers(1)?[0];
    let color = c.iter().map(|&v| u8::try_from(v).map_err(|_| err())).collect::<Result<Vec<_>, _>>()?;
    if d[1].abs() > d[0].abs() || w < 1 {
        return Err(err());
    }
    Ok(([color[0], color[1], color[2], color[3]], (d[0], d[1]), w))
}

/// The pixels of a `line:` image relative to its start.
fn line_pixels(dx: i32, dy: i32, w: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for j in 0..dx.abs() {
        let x = if dx < 0 { -1 - j } else { j };
        let row = j * dy.abs() / dx.abs();
        for t in 0..w {
            out.push((x, if dy < 0 { -(row + t) } else { row + t }));
        }
    }
    out
}

/// Top-left of a `line:` image relative to its start (the anchor of a
/// sheet that draws the line from its start point).
pub fn line_origin(dx: i32, dy: i32, w: i32) -> (i32, i32) {
    let pixels = line_pixels(dx, dy, w);
    (pixels.iter().map(|p| p.0).min().unwrap_or(0), pixels.iter().map(|p| p.1).min().unwrap_or(0))
}

fn blend_of(desc: BlendDesc) -> Option<Blend> {
    match desc {
        BlendDesc::Alpha => Some(Blend::Alpha),
        BlendDesc::Scale(d) if (1..=tile_render_shared::MAX_SCALE_DENOMINATOR).contains(&d) => Some(Blend::Scale { denominator: d }),
        BlendDesc::Scale(_) => None,
    }
}

/// `{v}` and `{v:02}` in `s` replaced by `v`.
fn substitute(s: &str, v: i64) -> String {
    s.replace("{v:02}", &format!("{v:02}")).replace("{v}", &v.to_string())
}

fn substitute_value(value: &toml::Value, v: i64) -> toml::Value {
    match value {
        toml::Value::String(s) => toml::Value::String(substitute(s, v)),
        toml::Value::Array(a) => toml::Value::Array(a.iter().map(|x| substitute_value(x, v)).collect()),
        toml::Value::Table(t) => toml::Value::Table(t.iter().map(|(k, x)| (k.clone(), substitute_value(x, v))).collect()),
        other => other.clone(),
    }
}

fn one() -> u32 {
    1
}

impl Description {
    /// Parses a description and expands *variants*: an image or sheet with
    /// `variants = [0, 1, ...]` stands for one copy per value, named
    /// `"name@value"`, in which `{v}` (or zero-padded `{v:02}`) in every
    /// string is replaced by the value — e.g. a sprite sheet recoloured with
    /// each player's palette:
    ///
    /// ```toml
    /// [images.border]
    /// source = "Art/border.pcx"
    /// palette = "Art/Palettes/player{v:02}.pcx"
    /// variants = [0, 1, 2]
    ///
    /// [sheets.border]
    /// image = "border@{v}"
    /// group = "Border@{v}"   # sprites "Border@0/0", "Border@1/0", ...
    /// variants = [0, 1, 2]
    /// ```
    pub fn parse(text: &str) -> Result<Self, DescError> {
        let mut desc: Description = toml::from_str(text)?;
        let mut images = BTreeMap::new();
        for (name, mut img) in std::mem::take(&mut desc.images) {
            match img.options.remove("variants") {
                None => {
                    images.insert(name, img);
                }
                Some(values) => {
                    let values: Vec<i64> = values.try_into().map_err(|e: toml::de::Error| DescError::Toml(e))?;
                    for v in values {
                        let source = substitute(&img.source, v);
                        let options = img.options.iter().map(|(k, o)| (k.clone(), substitute_value(o, v))).collect();
                        images.insert(format!("{name}@{v}"), ImageDesc { source, options });
                    }
                }
            }
        }
        desc.images = images;
        let mut sheets = BTreeMap::new();
        for (name, sheet) in std::mem::take(&mut desc.sheets) {
            match &sheet.variants {
                None => {
                    sheets.insert(name, sheet);
                }
                Some(values) => {
                    for &v in values {
                        let mut copy = sheet.clone();
                        copy.variants = None;
                        copy.image = substitute(&sheet.image, v);
                        copy.group = sheet.group.as_ref().map(|g| substitute(g, v));
                        sheets.insert(format!("{name}@{v}"), copy);
                    }
                }
            }
        }
        desc.sheets = sheets;
        Ok(desc)
    }

    /// The frames of image `name`; with `blank`, as the load that finds
    /// boxes makes them (`blank_loader`, then the image's own `blank`).
    fn load_frames(&self, loader: &dyn ImageLoader, name: &str, img: &ImageDesc, blank: bool) -> Result<Vec<RgbaImage>, DescError> {
        let mut options = self.loader.clone();
        options.extend(img.options.clone());
        // (`blank` is no loader option.)
        let image_blank = match options.remove("blank") {
            Some(toml::Value::Table(table)) => table,
            _ => toml::Table::new(),
        };
        if blank {
            options.extend(self.blank_loader.clone());
            options.extend(image_blank);
        }
        let generators: [(&str, fn(&str) -> Result<RgbaImage, String>); 4] = [("solid:", solid), ("line:", line), ("arrow:", arrow), ("crosshair:", crosshair)];
        let generated = generators.iter().find_map(|(prefix, make)| img.source.strip_prefix(prefix).map(make));
        let image_error = |message: String| DescError::Image { name: name.to_owned(), message };
        Ok(match generated {
            Some(image) => vec![image.map_err(image_error)?],
            None => loader.load(&img.source, &options).map_err(image_error)?,
        })
    }

    /// Loads every image, slices the sheets and builds the tile renderer's
    /// tables. `page_size` is the atlas page size (e.g. 4096).
    pub fn compile(&self, loader: &dyn ImageLoader, page_size: u32) -> Result<Compiled, DescError> {
        let mut atlas = AtlasBuilder::new(page_size);
        let mut images: HashMap<&str, Vec<RgbaImage>> = HashMap::new();
        for (name, img) in &self.images {
            images.insert(name, self.load_frames(loader, name, img, false)?);
        }
        // The images again as the sprite format sees them, for the sheets
        // that sample from their boxes (only those shrunk need them).
        let mut blank_images: HashMap<&str, Vec<RgbaImage>> = HashMap::new();

        // Sprites, named "sheet/index".
        let mut sprites: BTreeMap<String, SpriteId> = BTreeMap::new();
        let mut sheet_len: HashMap<&str, u32> = HashMap::new();
        for (sheet_name, sheet) in &self.sheets {
            let err = |message: String| DescError::Sheet { sheet: sheet_name.clone(), message };
            let frames = images.get(sheet.image.as_str()).ok_or(DescError::Unknown { kind: "image", name: sheet.image.clone() })?;
            let img = frames.get(sheet.frame).ok_or_else(|| err(format!("image has no frame {}", sheet.frame)))?;
            let boxed = sheet.sample_from == SampleFrom::Box && sheet.downscale > 1;
            let blank_img = if boxed {
                let name = sheet.image.as_str();
                if !blank_images.contains_key(name) {
                    blank_images.insert(name, self.load_frames(loader, name, &self.images[name], true)?);
                }
                let blank = blank_images[name].get(sheet.frame).ok_or_else(|| err(format!("image has no frame {}", sheet.frame)))?;
                if (blank.width, blank.height) != (img.width, img.height) {
                    return Err(err(format!("the image loads as {}×{} for its box but as {}×{} to draw", blank.width, blank.height, img.width, img.height)));
                }
                Some(blank)
            } else {
                None
            };
            let [cw, ch] = sheet.cell;
            let [ox, oy] = sheet.origin;
            let [sx, sy] = sheet.spacing;
            if cw == 0 || ch == 0 {
                return Err(err(format!("empty cell {cw}×{ch}")));
            }
            let fit = |total: u32, o: u32, c: u32, s: u32| if total < o + c { 0 } else { (total - o + s) / (c + s) };
            let [cols, rows] = sheet.grid.unwrap_or_else(|| [fit(img.width, ox, cw, sx), fit(img.height, oy, ch, sy)]);
            if ox + cols * (cw + sx) - sx > img.width || oy + rows * (ch + sy) - sy > img.height {
                return Err(err(format!("{cols}×{rows} cells of {cw}×{ch} exceed the {}×{} image", img.width, img.height)));
            }
            for i in 0..cols * rows {
                if sheet.only.as_ref().is_some_and(|o| !o.contains(&i)) {
                    continue;
                }
                let (c, r) = (i % cols, i / cols);
                let (x, y) = (ox + c * (cw + sx), oy + r * (ch + sy));
                let shape = |cell: RgbaImage| match &sheet.outline {
                    Some(o) => (outline(&cell, o.width, o.color), o.width as i32),
                    None => (cell, 0),
                };
                let (sprite, grow) = shape(img.crop(x, y, cw, ch));
                let sprite = match blank_img {
                    Some(blank) => downscale_from_box(&sprite, sheet.downscale, opaque_box(&shape(blank.crop(x, y, cw, ch)).0)),
                    None if sheet.downscale > 1 => downscale(&sprite, sheet.downscale),
                    None => sprite,
                };
                let id = atlas.add(&sprite, (sheet.anchor[0] - grow, sheet.anchor[1] - grow))?;
                let name = match &sheet.group {
                    Some(g) => format!("{g}/{}", sheet.first_index + i * sheet.index_stride),
                    None => format!("{sheet_name}/{i}"),
                };
                if sprites.insert(name.clone(), id).is_some() {
                    return Err(err(format!("sprite {name} defined twice")));
                }
            }
            let group = sheet.group.as_deref().unwrap_or(sheet_name);
            let len = sheet_len.entry(group).or_insert(0);
            *len = (*len).max(sheet.first_index + cols * rows * sheet.index_stride);
        }

        // Sets: explicit ones, plus an implicit single-frame set per sprite.
        let mut sets = SpriteSets::default();
        let mut set_ids: BTreeMap<String, SpriteSetId> = BTreeMap::new();
        let mut set_blend: HashMap<SpriteSetId, Blend> = HashMap::new();
        for (name, &sprite) in &sprites {
            set_ids.insert(name.clone(), sets.single(sprite));
        }
        for (name, set) in &self.sets {
            let blend = blend_of(set.blend).ok_or_else(|| DescError::Unknown { kind: "blend of set", name: name.clone() })?;
            let frames = set
                .frames
                .iter()
                .map(|f| sprites.get(f).copied().ok_or(DescError::Unknown { kind: "sprite", name: f.clone() }))
                .collect::<Result<Vec<_>, _>>()?;
            let id = sets.add(&frames);
            set_ids.insert(name.clone(), id);
            set_blend.insert(id, blend);
        }

        // Layers and their value tables.
        let mut layers = Vec::new();
        let mut values = HashMap::new();
        for layer in &self.layers {
            let mut names = Vec::new();
            for pattern in &layer.sets {
                if let Some(prefix) = pattern.strip_suffix('*') {
                    let mut matched: Vec<&String> = set_ids.keys().filter(|n| n.starts_with(prefix)).collect();
                    // "sheet/*" in sprite-index order, not lexicographic.
                    if let Some(sheet) = prefix.strip_suffix('/').filter(|s| sheet_len.contains_key(s)) {
                        matched.sort_by_key(|n| n[sheet.len() + 1..].parse::<u32>().unwrap_or(u32::MAX));
                    }
                    names.extend(matched.into_iter().cloned());
                } else if set_ids.contains_key(pattern) {
                    names.push(pattern.clone());
                } else {
                    return Err(DescError::Unknown { kind: "set", name: pattern.clone() });
                }
            }
            // Value 0 is always "nothing", so an all-zero layer is empty.
            let mut lut = vec![SpriteSetId::EMPTY];
            for (i, n) in names.iter().enumerate() {
                values.insert((layer.name.clone(), n.clone()), i as u32 + 1);
                lut.push(set_ids[n]);
            }
            let mut spec = LayerSpec::new(layer.name.clone(), lut);
            spec.offset = (layer.offset[0], layer.offset[1]);
            spec.ticks_per_frame = layer.ticks_per_frame.max(1);
            spec.per_cell_phase = layer.per_cell_phase;
            spec.hidden = layer.hidden;
            spec.pass = layer.pass;
            spec.cull_offset = (layer.cull_offset[0], layer.cull_offset[1]);
            spec.blend = blend_of(layer.blend)
                .ok_or_else(|| DescError::Unknown { kind: "blend (scale denominator 1..=32) of layer", name: layer.name.clone() })?;
            if layers.last().is_some_and(|l: &LayerSpec| l.pass > spec.pass) {
                return Err(DescError::Unknown { kind: "pass order for layer", name: layer.name.clone() });
            }
            layers.push(spec);
        }

        Ok(Compiled {
            layout: match self.grid.layout {
                LayoutDesc::Orthogonal => Layout::Orthogonal,
                LayoutDesc::Staggered => Layout::Staggered,
            },
            step: (self.grid.step[0], self.grid.step[1]),
            color: match self.grid.color_bits {
                None => ColorModel::Direct,
                Some(bits @ 4..=7) => ColorModel::Quantized { bits },
                Some(bits) => return Err(DescError::Unknown { kind: "grid colour depth (4..=7 bits)", name: bits.to_string() }),
            },
            sprite_names: {
                let mut names = vec![String::new(); sprites.len()];
                for (n, id) in &sprites {
                    names[id.0 as usize] = n.clone();
                }
                names
            },
            atlas: atlas.finish(),
            sets,
            layers,
            values,
            set_ids,
            set_blend,
        })
    }
}

/// A description turned into renderer tables.
pub struct Compiled {
    pub layout: Layout,
    pub step: (u32, u32),
    pub color: ColorModel,
    pub atlas: tile_render::Atlas,
    pub sets: SpriteSets,
    pub layers: Vec<LayerSpec>,
    /// Name of every sprite, by `SpriteId`.
    pub sprite_names: Vec<String>,
    values: HashMap<(String, String), u32>,
    set_ids: BTreeMap<String, SpriteSetId>,
    set_blend: HashMap<SpriteSetId, Blend>,
}

impl Compiled {
    /// A copy without the atlas pixels (sprite placements, names, sets and
    /// layers kept): what everything but the atlas upload needs, at a
    /// fraction of the memory.
    pub fn without_pixels(&self) -> Compiled {
        Compiled {
            layout: self.layout,
            step: self.step,
            color: self.color,
            atlas: tile_render::Atlas { page_size: self.atlas.page_size, pages: Vec::new(), sprites: self.atlas.sprites.clone() },
            sets: self.sets.clone(),
            layers: self.layers.clone(),
            sprite_names: self.sprite_names.clone(),
            values: self.values.clone(),
            set_ids: self.set_ids.clone(),
            set_blend: self.set_blend.clone(),
        }
    }

    /// The cell value that draws set `set` on layer `layer` (0 = nothing).
    pub fn value(&self, layer: &str, set: &str) -> Option<u32> {
        self.values.get(&(layer.to_owned(), set.to_owned())).copied()
    }

    /// The set (sprite, or named animation) for a free sprite, and how it
    /// blends.
    pub fn set(&self, name: &str) -> Option<(SpriteSetId, Blend)> {
        let id = *self.set_ids.get(name)?;
        Some((id, self.set_blend.get(&id).copied().unwrap_or_default()))
    }

    pub fn layer_index(&self, layer: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == layer)
    }

    /// An empty world of `cols × rows` cells with all layers.
    pub fn world(&self, cols: u32, rows: u32, wrap_x: bool, wrap_y: bool) -> tile_render::TileWorld {
        let grid = Grid { cols, rows, layout: self.layout, wrap_x, wrap_y, step: self.step };
        let mut world = tile_render::TileWorld::new(grid, self.sets.clone());
        world.color = self.color;
        for l in &self.layers {
            world.add_layer(l.clone());
        }
        world
    }
}


/// Every `n`-th pixel of every `n`-th row, from the top-left.
fn downscale(img: &RgbaImage, n: u32) -> RgbaImage {
    RgbaImage::from_fn(img.width / n, img.height / n, |x, y| img.get(x * n, y * n))
}

/// The smallest box `[x, y, width, height]` holding every pixel of `img`
/// that is not blank (alpha 0); `None` if all are.
fn opaque_box(img: &RgbaImage) -> Option<[u32; 4]> {
    let mut bounds: Option<[u32; 4]> = None; // left, top, right, bottom (inclusive)
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] != 0 {
                let [l, t, r, b] = bounds.get_or_insert([x, y, x, y]);
                (*l, *t, *r, *b) = ((*l).min(x), (*t).min(y), (*r).max(x), (*b).max(y));
            }
        }
    }
    bounds.map(|[l, t, r, b]| [l, t, r - l + 1, b - t + 1])
}

/// `img` shrunk by `n` the way a game does that keeps its sprites cropped to
/// `bounds` (their box, [`opaque_box`]): it samples the *box* at every
/// `n`-th pixel from the box's top-left, covering `box size / n` pixels
/// (rounded down), and draws the result at the box's origin divided by `n`
/// (rounded down). Relative to the cell, pixel `(x, y)` of the result is
/// `(n·x + box_x mod n, n·y + box_y mod n)` of it, wherever that lies in
/// the covered rectangle `[box_x / n, box_x / n + box_w / n)` (likewise in y);
/// the rest is clear. The cell's own size, rounded down, stays the size of
/// the result, so anchors keep their meaning.
fn downscale_from_box(img: &RgbaImage, n: u32, bounds: Option<[u32; 4]>) -> RgbaImage {
    let (w, h) = (img.width / n, img.height / n);
    let Some([bx, by, bw, bh]) = bounds else { return RgbaImage::new(w, h) };
    let (x0, y0) = (bx / n, by / n);
    RgbaImage::from_fn(w, h, |x, y| match (x.checked_sub(x0), y.checked_sub(y0)) {
        (Some(i), Some(j)) if i < bw / n && j < bh / n => img.get(bx + n * i, by + n * j),
        _ => [0; 4],
    })
}

/// Where the samples of a shrunk sprite start (`sample_from = "..."`).
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SampleFrom {
    /// At the cell's top-left.
    #[default]
    Cell,
    /// At the top-left of the sprite's *box*, the smallest rectangle holding
    /// its non-blank pixels (see [`Description::blank_loader`]), as a game
    /// does that keeps sprites cropped to it: the sampling phase is then the
    /// box origin modulo the factor, so it varies from sprite to sprite, and
    /// only `box size / n` pixels (rounded down) are drawn, from the box's
    /// origin divided by `n` (rounded down) on.
    Box,
}

/// How [`Description::scaled_down`] treats a sheet (`scaling = "..."`).
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SheetScaling {
    /// Shrunk, anchor divided.
    #[default]
    Scale,
    /// Shrunk, anchor kept.
    ScaleKeepAnchor,
    /// As it is (e.g. screen-space art).
    Keep,
}

impl Description {
    /// The same world `n` times smaller (a zoomed-out view): grid steps and
    /// offsets divided by `n`, sprites shrunk by [`SheetDesc::downscale`]
    /// (from where each sheet's [`SheetDesc::sample_from`] says) with their
    /// anchors divided by `n` (rounded toward zero), as each sheet's
    /// `scaling` says.
    pub fn scaled_down(&self, n: u32) -> Description {
        let n = n.max(1);
        let ni = n as i32;
        let mut d = self.clone();
        d.grid.step = d.grid.step.map(|s| s / n);
        for sheet in d.sheets.values_mut() {
            match sheet.scaling {
                SheetScaling::Keep => continue,
                SheetScaling::Scale => sheet.anchor = sheet.anchor.map(|a| a / ni),
                SheetScaling::ScaleKeepAnchor => {}
            }
            sheet.downscale *= n;
        }
        for layer in &mut d.layers {
            layer.offset = layer.offset.map(|o| o / ni);
            layer.cull_offset = layer.cull_offset.map(|o| o / ni);
        }
        d
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_go_around_the_outside() {
        // A 2-pixel ring (a hollow square) gets a 1-pixel band outside it,
        // none inside.
        let ring = RgbaImage::from_fn(8, 8, |x, y| if x < 2 || y < 2 || x > 5 || y > 5 { [9, 9, 9, 255] } else { [0; 4] });
        let o = outline(&ring, 1, [1, 2, 3, 255]);
        assert_eq!((o.width, o.height), (10, 10));
        assert_eq!(o.get(0, 5), [1, 2, 3, 255]);
        assert_eq!(o.get(1, 5), [0; 4]);
        assert_eq!(o.get(5, 5), [0; 4]);
        // (Width 1 still rounds the corners: the diagonal neighbour is in.)
        assert_eq!(o.get(0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn arrows_point_down_and_are_outlined() {
        let a = arrow("255,255,0,255:21x30").unwrap();
        let row = |y: u32| (0..21).map(|x| a.get(x, y)).collect::<Vec<_>>();
        // Shaft at the top, middle third; the tip at the bottom middle.
        assert_eq!(row(1)[10], [255, 255, 0, 255]);
        assert_eq!(row(1)[2], [0; 4]);
        assert_eq!(row(28)[10], [255, 255, 0, 255]);
        assert_eq!(row(28)[9], [0, 0, 0, 255]);
        assert_eq!(row(29)[10], [0, 0, 0, 255]);
        assert_eq!(row(29)[9], [0; 4]);
        // The head is wider than the shaft.
        let wide = |y: u32| row(y).iter().filter(|p| **p == [255, 255, 0, 255]).count();
        assert!(wide(15) > 2 * wide(5));
        assert!(crosshair("1,2,3,255:15x15").unwrap().get(7, 7) == [0; 4]);
        assert!(arrow("1,2,3,4:2x9").is_err());
    }

    struct Checker;
    impl ImageLoader for Checker {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            let seed = source.len() as u8;
            Ok(vec![RgbaImage::from_fn(40, 20, |x, y| [x as u8, y as u8, seed, 255])])
        }
    }

    #[test]
    fn compiles_sheets_sets_and_layers() {
        let d = Description::parse(
            r#"
            grid = { layout = "staggered", step = [64, 32] }
            [images.a]
            source = "a.pcx"
            transparent = [255]
            [sheets.s]
            image = "a"
            cell = [10, 10]
            grid = [4, 2]
            anchor = [0, -4]
            [sets.anim]
            frames = ["s/1", "s/2", "s/3"]
            [[layers]]
            name = "base"
            sets = ["s/*"]
            [[layers]]
            name = "top"
            sets = ["anim"]
            ticks_per_frame = 5
            "#,
        )
        .unwrap();
        let c = d.compile(&Checker, 256).unwrap();
        assert_eq!(c.atlas.sprites.len(), 8);
        assert_eq!(c.value("base", "s/0"), Some(1));
        assert_eq!(c.value("base", "s/7"), Some(8));
        assert_eq!(c.value("top", "anim"), Some(1));
        assert_eq!(c.layers[1].ticks_per_frame, 5);
        // Sprite s/5 is column 1, row 1 of the image.
        let s5 = c.atlas.sprites[5];
        let page = &c.atlas.pages[s5.page as usize];
        assert_eq!(page.get(s5.atlas_x, s5.atlas_y), [10, 10, 5, 255]);
        assert_eq!((s5.anchor_x, s5.anchor_y), (0, -4));
        let mut w = c.world(3, 3, true, false);
        w.set(tile_render::LayerId(0), 1, 1, c.value("base", "s/3").unwrap());
    }

    #[test]
    fn expands_variants() {
        let d = Description::parse(
            r#"
            grid = { layout = "orthogonal", step = [10, 10] }
            [images.b]
            source = "b{v:02}.pcx"
            palette = "p{v}.pcx"
            variants = [3, 12]
            [sheets.b]
            image = "b@{v}"
            cell = [10, 10]
            grid = [2, 1]
            group = "B@{v}"
            variants = [3, 12]
            [[layers]]
            name = "l"
            sets = ["B@*"]
            "#,
        )
        .unwrap();
        assert_eq!(d.images["b@3"].source, "b03.pcx");
        assert_eq!(d.images["b@12"].options["palette"].as_str(), Some("p12.pcx"));
        assert!(!d.images["b@3"].options.contains_key("variants"));
        let c = d.compile(&Checker, 256).unwrap();
        assert_eq!(c.atlas.sprites.len(), 4);
        assert!(c.value("l", "B@12/1").is_some());
        // Sources differ in length only by the padded value: "b03.pcx" vs "b12.pcx".
        let s = c.atlas.sprites[c.sprite_names.iter().position(|n| n == "B@3/0").unwrap()];
        assert_eq!(c.atlas.pages[s.page as usize].get(s.atlas_x, s.atlas_y)[2], 7);
    }

    #[test]
    fn rejects_unknown_names() {
        let d = Description::parse("grid = { layout = \"orthogonal\", step = [1, 1] }\n[[layers]]\nname = \"x\"\nsets = [\"nope\"]").unwrap();
        assert!(matches!(d.compile(&Checker, 64), Err(DescError::Unknown { kind: "set", .. })));
    }

    #[test]
    fn lines_follow_the_column_rule() {
        // Civ III's tile-edge lines (measured): 2 pixels per column, rows
        // j/2 and j/2 + 1 from the start, the end column excluded.
        for (dx, dy) in [(-64, -32), (-64, 32), (64, 32), (64, -32)] {
            let img = line(&format!("1,2,3,128:{dx},{dy}:2")).unwrap();
            let (ox, oy) = line_origin(dx, dy, 2);
            assert_eq!((img.width, img.height), (64, 33));
            assert_eq!((ox, oy), (dx.min(0), dy.min(0)));
            let mut n = 0;
            for j in 0..64 {
                let x = if dx < 0 { -1 - j } else { j };
                for t in 0..2 {
                    let y = dy.signum() * (j / 2 + t);
                    assert_eq!(img.get((x - ox) as u32, (y - oy) as u32), [1, 2, 3, 128], "({dx}, {dy}) column {j}");
                    n += 1;
                }
            }
            assert_eq!(img.pixels.iter().filter(|p| p[3] != 0).count(), n);
        }
        assert!(line("1,2,3,4:10,20:2").is_err());
    }

    #[test]
    fn scaled_down_keeps_even_pixels_and_halves_offsets() {
        let d = Description::parse(
            "[grid]\nlayout = \"staggered\"\nstep = [64, 32]\n\
             [images.a]\nsource = \"a\"\n\
             [sheets.s]\nimage = \"a\"\ncell = [5, 3]\ngrid = [1, 1]\nanchor = [-19, -15]\n\
             [sheets.f]\nimage = \"a\"\ncell = [4, 4]\ngrid = [1, 1]\nanchor = [0, -12]\nscaling = \"scale_keep_anchor\"\n\
             [sheets.u]\nimage = \"a\"\ncell = [4, 4]\ngrid = [1, 1]\nanchor = [3, 3]\nscaling = \"keep\"\n\
             [[layers]]\nname = \"l\"\nsets = [\"s/*\", \"f/*\", \"u/*\"]\ncull_offset = [0, -32]\n",
        )
        .unwrap()
        .scaled_down(2);
        assert_eq!(d.grid.step, [32, 16]);
        assert_eq!(d.layers[0].cull_offset, [0, -16]);
        let c = d.compile(&Checker, 64).unwrap();
        let sprite = |name: &str| c.atlas.sprites[c.sprite_names.iter().position(|n| n == name).unwrap()];
        let s = sprite("s/0");
        assert_eq!((s.width, s.height, s.anchor_x, s.anchor_y), (2, 1, -9, -7));
        let f = sprite("f/0");
        assert_eq!((f.width, f.height, f.anchor_x, f.anchor_y), (2, 2, 0, -12));
        let u = sprite("u/0");
        assert_eq!((u.width, u.height, u.anchor_x, u.anchor_y), (4, 4, 3, 3));
        // Pixel (1, 0) of the shrunk sprite is (2, 0) of the cell.
        let page = &c.atlas.pages[s.page as usize];
        assert_eq!(page.get(s.atlas_x + 1, s.atlas_y)[0], 2);
    }

    /// Pixels `(x, y, ·)` of 8 × 4: red = column, green = row. Column 0 has
    /// palette index 254 and column 7 index 255; the options' `transparent`
    /// indices (default 254 and 255) are clear, the others opaque.
    struct Columns;
    impl ImageLoader for Columns {
        fn load(&self, _: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            let clear: Vec<i64> = match options.get("transparent").and_then(|t| t.as_array()) {
                Some(list) => list.iter().filter_map(|v| v.as_integer()).collect(),
                None => vec![254, 255],
            };
            Ok(vec![RgbaImage::from_fn(8, 4, |x, y| {
                let index = [254, 1, 2, 3, 4, 5, 6, 255][x as usize];
                if clear.contains(&index) { [0; 4] } else { [x as u8, y as u8, 7, 255] }
            })])
        }
    }

    /// The shrunk sprite `s/0` of `text` (a description with one 8 × 4 sheet
    /// `s`) as columns of red values, `None` where clear, per row.
    fn shrunk(text: &str, n: u32) -> Vec<Vec<Option<u8>>> {
        let c = Description::parse(text).unwrap().scaled_down(n).compile(&Columns, 64).unwrap();
        let s = c.atlas.sprites[c.sprite_names.iter().position(|name| name == "s/0").unwrap()];
        let page = &c.atlas.pages[s.page as usize];
        (0..s.height).map(|y| (0..s.width).map(|x| Some(page.get(s.atlas_x + x, s.atlas_y + y)).filter(|p| p[3] != 0).map(|p| p[0])).collect()).collect()
    }

    const BOXED: &str = "[grid]\nlayout = \"orthogonal\"\nstep = [8, 4]\n\
        [images.a]\nsource = \"a\"\n\
        [sheets.s]\nimage = \"a\"\ncell = [8, 4]\ngrid = [1, 1]\n";

    #[test]
    fn sampling_from_the_cell_takes_even_columns() {
        // Columns 1..=6 are opaque: the shrunk sprite shows 0 (clear), 2, 4, 6.
        assert_eq!(shrunk(BOXED, 2)[0], [None, Some(2), Some(4), Some(6)]);
    }

    #[test]
    fn sampling_from_the_box_follows_the_boxs_origin() {
        // The box is columns 1..=6 (origin 1, width 6), all four rows. Odd
        // origin: samples are columns 1, 3, 5 from the box's origin divided
        // by 2, rounded down (0); a box of 6 columns covers 3 pixels, so the
        // fourth is clear although the cell's column 6 is opaque.
        let rows = shrunk(&format!("{BOXED}sample_from = \"box\"\n"), 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], [Some(1), Some(3), Some(5), None]);
        // Even origin in y (0): rows 0 and 2.
        assert_eq!(rows[1].len(), 4);
    }

    #[test]
    fn the_box_origin_is_divided_and_the_samples_follow_its_parity() {
        // Columns 3.. only (1 and 2 are made clear): origin 3 -> pixel 1
        // holds column 3, pixel 2 column 5, nothing before or after.
        let text = format!("{BOXED}sample_from = \"box\"\n[blank_loader]\ntransparent = [254, 255, 1, 2]\n");
        let rows = shrunk(&text, 2);
        assert_eq!(rows[0], [None, Some(3), Some(5), None]);
    }

    #[test]
    fn rows_are_sampled_from_the_boxs_top_too() {
        // A box starting at row 1 (rows 1..4, height 3) of a 4-row cell: one
        // pixel row (3 / 2) at row 1 / 2 = 0, sampling cell row 1.
        struct Rows;
        impl ImageLoader for Rows {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
                Ok(vec![RgbaImage::from_fn(2, 4, |x, y| if y == 0 { [0; 4] } else { [x as u8, y as u8, 0, 255] })])
            }
        }
        let text = "[grid]\nlayout = \"orthogonal\"\nstep = [8, 4]\n[images.a]\nsource = \"a\"\n\
                    [sheets.s]\nimage = \"a\"\ncell = [2, 4]\ngrid = [1, 1]\nsample_from = \"box\"\n";
        let c = Description::parse(text).unwrap().scaled_down(2).compile(&Rows, 64).unwrap();
        let s = c.atlas.sprites[0];
        assert_eq!((s.width, s.height), (1, 2));
        let page = &c.atlas.pages[s.page as usize];
        // Pixel row 0 holds cell row 1 (green 1), row 1 is past the box's 3 / 2 rows.
        assert_eq!(page.get(s.atlas_x, s.atlas_y), [0, 1, 0, 255]);
        assert_eq!(page.get(s.atlas_x, s.atlas_y + 1)[3], 0);
    }

    #[test]
    fn the_blank_load_decides_the_box() {
        // Drawn with 254 and 255 clear, but only 255 blank (the game's sprite
        // format): column 0 is in the box, so the origin is even and the
        // samples are columns 0, 2, 4 (column 0 itself is clear).
        let text = format!("{BOXED}sample_from = \"box\"\n[blank_loader]\ntransparent = [255]\n");
        assert_eq!(shrunk(&text, 2)[0], [None, Some(2), Some(4), None]);
        // An image's own `blank` overrides it: 254 blank again -> odd origin.
        let text = format!("[blank_loader]\ntransparent = [255]\n{}", BOXED.replace("source = \"a\"\n", "source = \"a\"\nblank = { transparent = [254, 255] }\n"));
        assert_eq!(shrunk(&format!("{text}sample_from = \"box\"\n"), 2)[0], [Some(1), Some(3), Some(5), None]);
    }

    #[test]
    fn boxes_change_nothing_at_full_size() {
        let text = format!("{BOXED}sample_from = \"box\"\n[blank_loader]\ntransparent = [255]\n");
        let c = Description::parse(&text).unwrap().compile(&Columns, 64).unwrap();
        let s = c.atlas.sprites[0];
        assert_eq!((s.width, s.height, s.anchor_x, s.anchor_y), (8, 4, 0, 0));
        let page = &c.atlas.pages[s.page as usize];
        assert_eq!(page.get(s.atlas_x + 3, s.atlas_y + 2), [3, 2, 7, 255]);
    }

    #[test]
    fn boxes_are_found_and_empty_ones_stay_clear() {
        let img = RgbaImage::from_fn(6, 5, |x, y| if (2..=4).contains(&x) && (1..=2).contains(&y) { [1, 1, 1, 9] } else { [7, 7, 7, 0] });
        assert_eq!(opaque_box(&img), Some([2, 1, 3, 2]));
        assert_eq!(opaque_box(&RgbaImage::new(3, 3)), None);
        assert!(downscale_from_box(&img, 2, None).pixels.iter().all(|p| p[3] == 0));
        // Sampling reaches only what the box covers: 3 / 2 = 1 column, 2 / 2 = 1 row.
        let small = downscale_from_box(&img, 2, opaque_box(&img));
        assert_eq!((small.width, small.height), (3, 2));
        assert_eq!(small.pixels.iter().filter(|p| p[3] != 0).count(), 1);
        assert_eq!(small.get(1, 0), [1, 1, 1, 9]);
    }
}
