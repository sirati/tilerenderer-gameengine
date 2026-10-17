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

    /// The sizes of the frames [`ImageLoader::load`] gives, which a loader
    /// may tell without making the pixels (by default it makes them).
    fn sizes(&self, source: &str, options: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
        Ok(self.load(source, options)?.iter().map(|f| (f.width, f.height)).collect())
    }

    /// The bytes of font file `source` (see [`Description::fonts`]).
    fn font(&self, source: &str) -> Result<Vec<u8>, String> {
        Err(format!("{source}: this loader has no fonts"))
    }
}

/// The sprites of one sheet as compiled (named by the compile: the same
/// art may be another sheet's next time).
struct Sliced {
    sprites: Vec<SlicedSprite>,
    /// How many cells the sheet has (`only` may leave some out).
    cells: u32,
}

struct SlicedSprite {
    /// The cell's index in the sheet.
    index: u32,
    /// The pixels, if compiled with them.
    image: Option<RgbaImage>,
    size: (u32, u32),
    anchor: (i32, i32),
    /// What a view showing the world smaller samples (x, y, width, height;
    /// see `tile_render_shared::SpriteDesc::box_x`): the whole sprite, or
    /// its box (`sample_from = "box"`, known with the pixels only).
    sample_box: [u32; 4],
    /// The cell drawn instead there (`scaled_origin`): its pixels (with
    /// the pixels) and sample box.
    scaled: Option<(Option<RgbaImage>, [u32; 4])>,
}

/// The sheets of the last [`Description::compile_cached`], by what makes
/// them, so that the next compile of a description that has them too (e.g.
/// one that grew) only loads and slices the new ones. Holds the sprites'
/// pixels: about the atlas's size.
#[derive(Default)]
pub struct SheetCache {
    sheets: Option<HashMap<String, std::sync::Arc<Sliced>>>,
}

impl SheetCache {
    /// An empty cache (also for statics).
    pub const fn new() -> Self {
        Self { sheets: None }
    }

    /// How many sheets it holds.
    pub fn len(&self) -> usize {
        self.sheets.as_ref().map_or(0, HashMap::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
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
    /// `sample_from = "box"` (see [`SampleFrom`]; only that load, made once
    /// at compile time, decides the boxes): they override `loader`
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
    /// Font faces text is drawn with from outlines (`tile_render::OutlineGlyph`),
    /// by name: `[fonts."Lucida Sans"] source = "LSANS.TTF"`, loaded with
    /// [`ImageLoader::font`]. A face that does not load is left out (logged):
    /// its text is then not drawn from outlines.
    #[serde(default)]
    pub fonts: BTreeMap<String, FontDesc>,
}

/// A font face (see [`Description::fonts`]).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontDesc {
    /// The font file, as the loader finds it.
    pub source: String,
    /// The face in a font collection.
    #[serde(default)]
    pub index: u32,
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
    /// Where sprite (0, 0) is for a view showing the world smaller
    /// (`tile_render::Camera::downscale`): for art the game keeps in a
    /// second place for its zoomed-out view. Those cells go into the atlas
    /// too, as each sprite's `downscaled` twin.
    pub scaled_origin: Option<[u32; 2]>,
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
    /// Where a view showing the world `n` times smaller
    /// (`tile_render::Camera::downscale`, a game's zoomed-out view) samples
    /// the sprites: every `n`-th pixel of every `n`-th row (nearest
    /// neighbour) from there, from the same art.
    #[serde(default)]
    pub sample_from: SampleFrom,
    /// How such a view draws them.
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

    /// The sizes of image `name`'s frames, without its pixels where the
    /// loader can tell them so (see [`ImageLoader::sizes`]).
    fn frame_sizes(&self, loader: &dyn ImageLoader, name: &str, img: &ImageDesc) -> Result<Vec<(u32, u32)>, DescError> {
        let generated = ["solid:", "line:", "arrow:", "crosshair:"].iter().any(|p| img.source.starts_with(p));
        if generated {
            return Ok(self.load_frames(loader, name, img, false)?.iter().map(|f| (f.width, f.height)).collect());
        }
        let mut options = self.loader.clone();
        options.extend(img.options.clone());
        options.remove("blank");
        loader.sizes(&img.source, &options).map_err(|message| DescError::Image { name: name.to_owned(), message })
    }

    /// Loads the images, slices the sheets and builds the tile renderer's
    /// tables. `page_size` is the atlas page size (e.g. 4096).
    pub fn compile(&self, loader: &dyn ImageLoader, page_size: u32) -> Result<Compiled, DescError> {
        self.compile_with(loader, page_size, true, None)
    }

    /// Like [`Description::compile`], reusing the sprites of every sheet
    /// `cache` has from an earlier compile (the same sheet of the same image,
    /// loaded the same way): only new sheets load their images. The cache
    /// keeps this compile's sheets afterwards, no others.
    pub fn compile_cached(&self, loader: &dyn ImageLoader, page_size: u32, cache: &mut SheetCache) -> Result<Compiled, DescError> {
        self.compile_with(loader, page_size, true, Some(cache))
    }

    /// The tables [`Description::compile`] makes without the pixels: sprite
    /// names, sizes and anchors, sets and layers, as
    /// [`Compiled::without_pixels`] keeps them (the sprites' places in the
    /// atlas are not decided). Only the images' sizes are needed
    /// ([`ImageLoader::sizes`]), not their pixels: for a side that draws
    /// nothing itself.
    pub fn compile_layout(&self, loader: &dyn ImageLoader, page_size: u32) -> Result<Compiled, DescError> {
        self.compile_with(loader, page_size, false, None)
    }

    /// What `sheet`'s sprites are made from, beyond `self.loader` and
    /// `self.blank_loader` (`globals`, the same for all sheets): its image
    /// and how it is cut, not what the sheet, its image or its sprites are
    /// called (names are given in the order things were added, and move as
    /// art is added). For [`SheetCache`].
    fn sheet_key(&self, sheet: &SheetDesc, globals: &str) -> String {
        let shape = SheetDesc { image: String::new(), group: None, first_index: 0, index_stride: 1, ..sheet.clone() };
        format!("{shape:?}|{:?}|{globals}", self.images.get(&sheet.image))
    }

    /// The sprites of sheet `name` (`pixels`: with their pixels, else their
    /// sizes only), from the frames of its image (`frames` loads an image's
    /// frames, `blank` them as the load that finds boxes makes them, `sizes`
    /// gives their sizes).
    fn slice(
        &self,
        name: &str,
        sheet: &SheetDesc,
        pixels: bool,
        frames: &mut dyn FnMut(&str, bool) -> Result<std::rc::Rc<Vec<RgbaImage>>, DescError>,
        sizes: &mut dyn FnMut(&str) -> Result<std::rc::Rc<Vec<(u32, u32)>>, DescError>,
    ) -> Result<Sliced, DescError> {
        let err = |message: String| DescError::Sheet { sheet: name.to_owned(), message };
        let no_frame = || err(format!("image has no frame {}", sheet.frame));
        let all = if pixels { Some(frames(&sheet.image, false)?) } else { None };
        let img = match &all {
            Some(all) => Some(all.get(sheet.frame).ok_or_else(no_frame)?),
            None => None,
        };
        let (iw, ih) = match img {
            Some(img) => (img.width, img.height),
            None => *sizes(&sheet.image)?.get(sheet.frame).ok_or_else(no_frame)?,
        };
        // The images again as the sprite format sees them, for the sheets
        // that sample from their boxes (the boxes are found once, here).
        let boxed = sheet.sample_from == SampleFrom::Box;
        let all_blank = if boxed && pixels { Some(frames(&sheet.image, true)?) } else { None };
        let blank_img = match &all_blank {
            Some(all) => {
                let blank = all.get(sheet.frame).ok_or_else(no_frame)?;
                if (blank.width, blank.height) != (iw, ih) {
                    return Err(err(format!("the image loads as {}×{} for its box but as {iw}×{ih} to draw", blank.width, blank.height)));
                }
                Some(blank)
            }
            None => None,
        };
        let [cw, ch] = sheet.cell;
        let [ox, oy] = sheet.origin;
        let [sx, sy] = sheet.spacing;
        if cw == 0 || ch == 0 {
            return Err(err(format!("empty cell {cw}×{ch}")));
        }
        let fit = |total: u32, o: u32, c: u32, s: u32| if total < o + c { 0 } else { (total - o + s) / (c + s) };
        let [cols, rows] = sheet.grid.unwrap_or_else(|| [fit(iw, ox, cw, sx), fit(ih, oy, ch, sy)]);
        let exceeds = |[ox, oy]: [u32; 2]| ox + cols * (cw + sx) - sx > iw || oy + rows * (ch + sy) - sy > ih;
        if exceeds(sheet.origin) || sheet.scaled_origin.is_some_and(exceeds) {
            return Err(err(format!("{cols}×{rows} cells of {cw}×{ch} exceed the {iw}×{ih} image")));
        }
        let grow = sheet.outline.as_ref().map_or(0, |o| o.width);
        let (gw, gh) = (cw + 2 * grow, ch + 2 * grow);
        let shape = |cell: RgbaImage| match &sheet.outline {
            Some(o) => outline(&cell, o.width, o.color),
            None => cell,
        };
        // A cell's pixels and what a view showing the world smaller samples
        // of it: its box (empty if it is all blank) or all of it.
        let cut = |x: u32, y: u32| -> (Option<RgbaImage>, [u32; 4]) {
            let image = img.map(|img| shape(img.crop(x, y, cw, ch)));
            let sample_box = match blank_img {
                Some(blank) => opaque_box(&shape(blank.crop(x, y, cw, ch))).unwrap_or_default(),
                None => [0, 0, gw, gh],
            };
            (image, sample_box)
        };
        let mut sprites = Vec::new();
        for i in 0..cols * rows {
            if sheet.only.as_ref().is_some_and(|o| !o.contains(&i)) {
                continue;
            }
            let (c, r) = (i % cols, i / cols);
            let at = |[ox, oy]: [u32; 2]| (ox + c * (cw + sx), oy + r * (ch + sy));
            let (x, y) = at(sheet.origin);
            let (image, sample_box) = cut(x, y);
            let scaled = sheet.scaled_origin.map(|o| {
                let (x, y) = at(o);
                cut(x, y)
            });
            let anchor = (sheet.anchor[0] - grow as i32, sheet.anchor[1] - grow as i32);
            sprites.push(SlicedSprite { index: i, image, size: (gw, gh), anchor, sample_box, scaled });
        }
        Ok(Sliced { sprites, cells: cols * rows })
    }

    fn compile_with(&self, loader: &dyn ImageLoader, page_size: u32, pixels: bool, mut cache: Option<&mut SheetCache>) -> Result<Compiled, DescError> {
        use std::rc::Rc;
        // Images are loaded when a sheet needs them, once per compile.
        let mut loaded: HashMap<(String, bool), Rc<Vec<RgbaImage>>> = HashMap::new();
        let mut frames = |name: &str, blank: bool| -> Result<Rc<Vec<RgbaImage>>, DescError> {
            if let Some(f) = loaded.get(&(name.to_owned(), blank)) {
                return Ok(f.clone());
            }
            let img = self.images.get(name).ok_or(DescError::Unknown { kind: "image", name: name.to_owned() })?;
            let f = Rc::new(self.load_frames(loader, name, img, blank)?);
            loaded.insert((name.to_owned(), blank), f.clone());
            Ok(f)
        };
        let mut measured: HashMap<String, Rc<Vec<(u32, u32)>>> = HashMap::new();
        let mut sizes = |name: &str| -> Result<Rc<Vec<(u32, u32)>>, DescError> {
            if let Some(s) = measured.get(name) {
                return Ok(s.clone());
            }
            let img = self.images.get(name).ok_or(DescError::Unknown { kind: "image", name: name.to_owned() })?;
            let s = Rc::new(self.frame_sizes(loader, name, img)?);
            measured.insert(name.to_owned(), s.clone());
            Ok(s)
        };
        let globals = format!("{:?}|{:?}", self.loader, self.blank_loader);
        let mut kept: HashMap<String, std::sync::Arc<Sliced>> = HashMap::new();
        let mut atlas = AtlasBuilder::new(page_size);
        let mut placed: Vec<tile_render_shared::SpriteDesc> = Vec::new();
        // Sprites with a twin for views showing the world smaller
        // (`scaled_origin`): which, and where their twin's cell is.
        let mut twins: Vec<(SpriteId, std::sync::Arc<Sliced>, usize, u32)> = Vec::new();

        // Sprites, named "sheet/index".
        let mut sprites: BTreeMap<String, SpriteId> = BTreeMap::new();
        let mut sheet_len: HashMap<String, u32> = HashMap::new();
        for (sheet_name, sheet) in &self.sheets {
            let sliced = match cache.as_deref_mut() {
                Some(cache) => {
                    let key = self.sheet_key(sheet, &globals);
                    let sliced = match cache.sheets.as_ref().and_then(|c| c.get(&key)) {
                        Some(s) => s.clone(),
                        None => std::sync::Arc::new(self.slice(sheet_name, sheet, pixels, &mut frames, &mut sizes)?),
                    };
                    kept.insert(key, sliced.clone());
                    sliced
                }
                None => std::sync::Arc::new(self.slice(sheet_name, sheet, pixels, &mut frames, &mut sizes)?),
            };
            let scaling = match sheet.scaling {
                SheetScaling::Scale => tile_render_shared::SHRINK,
                SheetScaling::ScaleKeepAnchor => tile_render_shared::SHRINK_KEEP_ANCHOR,
                SheetScaling::Keep => tile_render_shared::KEEP_SIZE,
            };
            for (k, s) in sliced.sprites.iter().enumerate() {
                let id = add_sprite(&mut atlas, &mut placed, page_size, s.image.as_ref(), s.size, s.anchor, scaling, s.sample_box)?;
                if s.scaled.is_some() {
                    twins.push((id, sliced.clone(), k, scaling));
                }
                let name = match &sheet.group {
                    Some(g) => format!("{g}/{}", sheet.first_index + s.index * sheet.index_stride),
                    None => format!("{sheet_name}/{}", s.index),
                };
                if sprites.insert(name.clone(), id).is_some() {
                    return Err(DescError::Sheet { sheet: sheet_name.clone(), message: format!("sprite {name} defined twice") });
                }
            }
            let group = sheet.group.clone().unwrap_or_else(|| sheet_name.clone());
            let len = sheet_len.entry(group).or_insert(0);
            *len = (*len).max(sheet.first_index + sliced.cells * sheet.index_stride);
        }
        // The cells drawn instead in views showing the world smaller, after
        // all named sprites (so that a compile without the pixels numbers
        // those the same).
        for (id, sliced, k, scaling) in twins {
            let s = &sliced.sprites[k];
            let Some((image, sample_box)) = &s.scaled else { continue };
            let twin = add_sprite(&mut atlas, &mut placed, page_size, image.as_ref(), s.size, s.anchor, scaling, *sample_box)?;
            let desc = if pixels { atlas.sprite_mut(id) } else { &mut placed[id.0 as usize] };
            desc.downscaled = twin.0 + 1;
        }
        if let Some(cache) = cache {
            cache.sheets = Some(kept);
        }
        let atlas = if pixels { atlas.finish() } else { tile_render::Atlas { page_size, pages: Vec::new(), sprites: placed } };

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

        // Font faces (those that load).
        let mut fonts = Vec::new();
        for (name, font) in &self.fonts {
            let face = loader.font(&font.source).and_then(|data| tile_render::gpu_text::Face::parse(&data, font.index).map_err(|e| e.to_string()));
            match face {
                Ok(face) => fonts.push((name.clone(), std::sync::Arc::new(face))),
                Err(e) => log::warn!("font {name:?} ({}): {e}", font.source),
            }
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
                    if let Some(sheet) = prefix.strip_suffix('/').filter(|s| sheet_len.contains_key(*s)) {
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
            atlas,
            sets,
            layers,
            values,
            set_ids,
            set_blend,
            fonts,
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
    /// The font faces that loaded, by name, in the order of
    /// `TileWorld::fonts` (see [`Compiled::font`]).
    pub fonts: Vec<(String, std::sync::Arc<tile_render::gpu_text::Face>)>,
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
            fonts: self.fonts.clone(),
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

    /// Font face `name`: its index (`OutlineGlyph::font`) and the face.
    pub fn font(&self, name: &str) -> Option<(u32, &tile_render::gpu_text::Face)> {
        self.fonts.iter().position(|(n, _)| n == name).map(|i| (i as u32, &*self.fonts[i].1))
    }

    pub fn layer_index(&self, layer: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == layer)
    }

    /// An empty world of `cols × rows` cells with all layers.
    pub fn world(&self, cols: u32, rows: u32, wrap_x: bool, wrap_y: bool) -> tile_render::TileWorld {
        let grid = Grid { cols, rows, layout: self.layout, wrap_x, wrap_y, step: self.step };
        let mut world = tile_render::TileWorld::new(grid, self.sets.clone());
        world.color = self.color;
        world.fonts = self.fonts.iter().map(|(_, f)| f.clone()).collect();
        for l in &self.layers {
            world.add_layer(l.clone());
        }
        world
    }
}


/// Adds a sprite: with its pixels to `atlas`, else (a compile without them)
/// its size and anchor to `placed`, checked against the page size; with
/// how views showing the world smaller draw it.
#[allow(clippy::too_many_arguments)]
fn add_sprite(
    atlas: &mut AtlasBuilder,
    placed: &mut Vec<tile_render_shared::SpriteDesc>,
    page_size: u32,
    image: Option<&RgbaImage>,
    (w, h): (u32, u32),
    anchor: (i32, i32),
    scaling: u32,
    [box_x, box_y, box_w, box_h]: [u32; 4],
) -> Result<SpriteId, DescError> {
    let (id, desc) = match image {
        Some(image) => {
            let id = atlas.add(image, anchor)?;
            (id, atlas.sprite_mut(id))
        }
        None => {
            if w > page_size || h > page_size {
                return Err(tile_render::atlas::AtlasError::TooLarge(w, h, page_size).into());
            }
            placed.push(tile_render_shared::SpriteDesc { width: w, height: h, anchor_x: anchor.0, anchor_y: anchor.1, ..Default::default() });
            (SpriteId(placed.len() as u32 - 1), placed.last_mut().expect("just pushed"))
        }
    };
    (desc.scaling, desc.box_x, desc.box_y, desc.box_w, desc.box_h) = (scaling, box_x, box_y, box_w, box_h);
    Ok(id)
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

/// Where a view showing the world `n` times smaller samples a sheet's
/// sprites (`sample_from = "..."`): every `n`-th pixel of every `n`-th row
/// from there (`tile_render_shared::placed`).
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SampleFrom {
    /// From the cell's top-left, all of it.
    #[default]
    Cell,
    /// From the top-left of the sprite's *box*, the smallest rectangle
    /// holding its non-blank pixels (see [`Description::blank_loader`]), as
    /// a game does that keeps sprites cropped to it: the sampling phase is
    /// then the box origin modulo the factor, so it varies from sprite to
    /// sprite, and only `box size / n` pixels (rounded down) are drawn, from
    /// the box's origin divided by `n` (rounded down) on. The boxes are found
    /// once, at compile time.
    Box,
}

/// How a view showing the world smaller draws a sheet's sprites
/// (`scaling = "..."`).
#[derive(Copy, Clone, Debug, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SheetScaling {
    /// Shrunk, anchor divided (rounded toward zero).
    #[default]
    Scale,
    /// Shrunk, anchor kept.
    ScaleKeepAnchor,
    /// As it is (e.g. screen-space art).
    Keep,
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

    /// Fonts from the game's folder, where there is a Civ III installation.
    struct Fonts(std::path::PathBuf);
    impl ImageLoader for Fonts {
        fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Checker.load(source, options)
        }
        fn font(&self, source: &str) -> Result<Vec<u8>, String> {
            std::fs::read(self.0.join(source)).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn fonts_that_load_are_kept_by_name() {
        let d = Description::parse(
            r#"
            grid = { layout = "orthogonal", step = [64, 32] }
            [fonts."Lucida Sans"]
            source = "LSANS.TTF"
            [fonts.missing]
            source = "none.ttf"
            "#,
        )
        .unwrap();
        // A loader without fonts: none, and the rest compiles.
        let c = d.compile(&Checker, 256).unwrap();
        assert!(c.fonts.is_empty() && c.font("Lucida Sans").is_none());
        let install = std::path::PathBuf::from(std::env::var("CIV3_INSTALL").unwrap_or_else(|_| "/mnt/datasets/winvm/civ3-src/Civilization III Complete".into()));
        if !install.join("LSANS.TTF").is_file() {
            return eprintln!("no Civ III installation, skipped");
        }
        for c in [d.compile(&Fonts(install.clone()), 256).unwrap(), d.compile_layout(&Fonts(install), 256).unwrap().without_pixels()] {
            let (index, face) = c.font("Lucida Sans").expect("loaded");
            assert_eq!(index, 0);
            assert!(face.glyph('R').is_some());
            assert!(c.font("missing").is_none());
            assert_eq!(c.world(2, 2, false, false).fonts.len(), 1);
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

    /// A description with most kinds of sheet: grids fitted and given,
    /// groups, `only`, outlines, shrunk from the box, generated images.
    const MIXED: &str = r#"
        grid = { layout = "orthogonal", step = [8, 4] }
        [images.a]
        source = "a"
        [images.b]
        source = "b"
        [images.bar]
        source = "solid:1,2,3,255:5x2"
        [sheets.s]
        image = "a"
        cell = [10, 10]
        grid = [4, 2]
        anchor = [0, -4]
        only = [0, 2, 5]
        [sheets.ring]
        image = "b"
        cell = [8, 8]
        group = "Ring"
        first_index = 3
        outline = { width = 2, color = [9, 9, 9, 255] }
        [sheets.small]
        image = "b"
        cell = [8, 6]
        grid = [2, 1]
        scaled_origin = [0, 8]
        sample_from = "box"
        anchor = [-3, 1]
        [sheets.bar]
        image = "bar"
        cell = [5, 2]
        [sets.anim]
        frames = ["s/0", "Ring/3", "small/1"]
        [[layers]]
        name = "l"
        sets = ["s/*", "anim", "bar/0"]
        "#;

    /// Counts its loads.
    struct Counting(std::cell::Cell<usize>);
    impl ImageLoader for Counting {
        fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            self.0.set(self.0.get() + 1);
            Checker.load(source, options)
        }
        fn sizes(&self, _: &str, _: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
            Ok(vec![(40, 20)])
        }
    }

    fn tables(c: &Compiled) -> (Vec<String>, Vec<(u32, u32, i32, i32)>, Vec<Option<u32>>) {
        let sprites = c.atlas.sprites.iter().map(|s| (s.width, s.height, s.anchor_x, s.anchor_y)).collect();
        let values = ["s/0", "s/5", "anim", "bar/0", "Ring/3"].iter().map(|n| c.value("l", n)).collect();
        (c.sprite_names.clone(), sprites, values)
    }

    #[test]
    fn the_layout_alone_is_the_compiled_layout_without_loading_pixels() {
        let d = Description::parse(MIXED).unwrap();
        let full = d.compile(&Checker, 256).unwrap();
        let counting = Counting(std::cell::Cell::new(0));
        let layout = d.compile_layout(&counting, 256).unwrap();
        assert_eq!(tables(&layout), tables(&full));
        assert_eq!(layout.set("anim").map(|s| s.0), full.set("anim").map(|s| s.0));
        assert!(layout.atlas.pages.is_empty());
        assert_eq!(counting.0.get(), 0, "sizes only");
        // A sheet too big for a page fails as compiling it would.
        assert!(d.compile_layout(&Checker, 8).is_err());
    }

    #[test]
    fn cached_compiles_load_only_new_sheets_and_agree_with_plain_ones() {
        let d = Description::parse(MIXED).unwrap();
        let counting = Counting(std::cell::Cell::new(0));
        let mut cache = SheetCache::default();
        d.compile_cached(&counting, 256, &mut cache).unwrap();
        let first = counting.0.get();
        assert!(first > 0);
        // Grown by a sheet of a new image: only that one is loaded.
        let grown = format!("{MIXED}\n[images.c]\nsource = \"ccc\"\n[sheets.t]\nimage = \"c\"\ncell = [4, 4]\ngrid = [2, 1]\n");
        let d2 = Description::parse(&grown).unwrap();
        let cached = d2.compile_cached(&counting, 256, &mut cache).unwrap();
        assert_eq!(counting.0.get(), first + 1);
        let plain = d2.compile(&Checker, 256).unwrap();
        assert_eq!(tables(&cached), tables(&plain));
        let pixels = |c: &Compiled| -> Vec<Vec<[u8; 4]>> {
            c.atlas.sprites.iter().map(|s| (0..s.height).flat_map(|y| (0..s.width).map(move |x| (x, y))).map(|(x, y)| c.atlas.pages[s.page as usize].get(s.atlas_x + x, s.atlas_y + y)).collect()).collect()
        };
        assert_eq!(pixels(&cached), pixels(&plain));
        // A changed sheet is made again; the cache keeps only what was used.
        let changed = grown.replace("anchor = [0, -4]", "anchor = [1, -4]");
        let n = cache.len();
        let c = Description::parse(&changed).unwrap().compile_cached(&counting, 256, &mut cache).unwrap();
        assert_eq!(counting.0.get(), first + 2);
        assert_eq!(c.atlas.sprites[c.sprite_names.iter().position(|s| s == "s/0").unwrap()].anchor_x, 1);
        assert_eq!(cache.len(), n);
        Description::parse(MIXED).unwrap().compile_cached(&counting, 256, &mut cache).unwrap();
        assert_eq!(cache.len(), n - 1);
        // Renamed (as art added earlier in the order renames what follows):
        // the same art, nothing loaded.
        let loads = counting.0.get();
        let renamed = MIXED.replace("[images.b]", "[images.b2]").replace("image = \"b\"", "image = \"b2\"").replace("[sheets.s]", "[sheets.s0]").replace("s/", "s0/");
        let c = Description::parse(&renamed).unwrap().compile_cached(&counting, 256, &mut cache).unwrap();
        assert_eq!(counting.0.get(), loads);
        assert!(c.sprite_names.contains(&"s0/5".to_owned()));
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

    /// Sprite `name` of `c` as a view showing the world `n` times smaller
    /// draws it (the shaders' reference renderer), anchored at the corner
    /// of a picture of the cell's size divided by `n`: what shrinking the
    /// art itself made before.
    fn render_small(c: &Compiled, name: &str, n: u32) -> RgbaImage {
        let id = c.sprite_names.iter().position(|s| s == name).unwrap();
        let s = c.atlas.sprites[id];
        let mut world = c.world(1, 1, false, false);
        let set = c.set(name).unwrap().0;
        let p = tile_render_shared::placed(&s, n);
        // (World positions are divided by `n` too.)
        let n_i = n.max(1) as i32;
        world.free.push(tile_render::PlacedSprite::new(set, -p.corner_x * n_i, -p.corner_y * n_i));
        let size = if p.step == 1 { (s.width, s.height) } else { (s.width / n, s.height / n) };
        let camera = tile_render::Camera { downscale: n, ..tile_render::Camera::new(size) };
        tile_render::reference::render(&world, &c.atlas, &camera, 0, [0; 4])
    }

    #[test]
    fn views_shown_smaller_keep_even_pixels_and_halve_offsets() {
        let d = Description::parse(
            "[grid]\nlayout = \"staggered\"\nstep = [64, 32]\n\
             [images.a]\nsource = \"a\"\n\
             [sheets.s]\nimage = \"a\"\ncell = [5, 3]\ngrid = [1, 1]\nanchor = [-19, -15]\n\
             [sheets.f]\nimage = \"a\"\ncell = [4, 4]\ngrid = [1, 1]\nanchor = [0, -12]\nscaling = \"scale_keep_anchor\"\n\
             [sheets.u]\nimage = \"a\"\ncell = [4, 4]\ngrid = [1, 1]\nanchor = [3, 3]\nscaling = \"keep\"\n\
             [[layers]]\nname = \"l\"\nsets = [\"s/*\", \"f/*\", \"u/*\"]\ncull_offset = [0, -32]\n",
        )
        .unwrap();
        let c = d.compile(&Checker, 64).unwrap();
        // One copy of the art: the sprites at full size.
        assert_eq!(c.step, (64, 32));
        let sprite = |name: &str| c.atlas.sprites[c.sprite_names.iter().position(|n| n == name).unwrap()];
        let small = |name: &str| {
            let p = tile_render_shared::placed(&sprite(name), 2);
            (p.inset_x + p.width, p.inset_y + p.height, p.corner_x, p.corner_y)
        };
        assert_eq!((sprite("s/0").width, sprite("s/0").height), (5, 3));
        assert_eq!(small("s/0"), (2, 1, -9, -7));
        assert_eq!(small("f/0"), (2, 2, 0, -12));
        assert_eq!(small("u/0"), (4, 4, 3, 3));
        // Pixel (1, 0) of the shrunk sprite is (2, 0) of the cell.
        assert_eq!(render_small(&c, "s/0", 2).get(1, 0)[0], 2);
        assert_eq!(render_small(&c, "u/0", 2).get(3, 2)[..2], [3, 2]);
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
        let c = Description::parse(text).unwrap().compile(&Columns, 64).unwrap();
        let img = render_small(&c, "s/0", n);
        (0..img.height).map(|y| (0..img.width).map(|x| Some(img.get(x, y)).filter(|p| p[3] != 0).map(|p| p[0])).collect()).collect()
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
        let c = Description::parse(text).unwrap().compile(&Rows, 64).unwrap();
        let s = c.atlas.sprites[0];
        assert_eq!((s.box_x, s.box_y, s.box_w, s.box_h), (0, 1, 2, 3));
        let img = render_small(&c, "s/0", 2);
        assert_eq!((img.width, img.height), (1, 2));
        // Pixel row 0 holds cell row 1 (green 1), row 1 is past the box's 3 / 2 rows.
        assert_eq!(img.get(0, 0), [0, 1, 0, 255]);
        assert_eq!(img.get(0, 1)[3], 0);
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
    fn art_for_views_shown_smaller_can_come_from_another_cell() {
        // A game that keeps a second sprite for its zoomed-out view: it is in
        // the atlas too, after the named sprites, drawn instead there.
        let text = "[grid]\nlayout = \"orthogonal\"\nstep = [8, 4]\n[images.a]\nsource = \"a\"\n\
                    [sheets.s]\nimage = \"a\"\ncell = [4, 4]\ngrid = [1, 1]\norigin = [0, 0]\nscaled_origin = [8, 4]\n";
        let c = Description::parse(text).unwrap().compile(&Checker, 64).unwrap();
        assert_eq!((c.sprite_names.len(), c.atlas.sprites.len()), (1, 2));
        assert_eq!(c.atlas.sprites[0].downscaled, 2);
        let full = render_small(&c, "s/0", 1);
        assert_eq!(full.get(0, 0)[..2], [0, 0]);
        // The cell at (8, 4), every second pixel.
        let small = render_small(&c, "s/0", 2);
        assert_eq!([small.get(0, 0), small.get(1, 0)].map(|p| [p[0], p[1]]), [[8, 4], [10, 4]]);
        // A compile without the pixels numbers them the same.
        let layout = Description::parse(text).unwrap().compile_layout(&Counting(std::cell::Cell::new(0)), 64).unwrap();
        assert_eq!((layout.atlas.sprites.len(), layout.atlas.sprites[0].downscaled), (2, 2));
    }

    #[test]
    fn boxes_change_nothing_at_full_size() {
        let text = format!("{BOXED}sample_from = \"box\"\n[blank_loader]\ntransparent = [255]\n");
        let c = Description::parse(&text).unwrap().compile(&Columns, 64).unwrap();
        let s = c.atlas.sprites[0];
        assert_eq!((s.width, s.height, s.anchor_x, s.anchor_y), (8, 4, 0, 0));
        let page = &c.atlas.pages[s.page as usize];
        assert_eq!(page.get(s.atlas_x + 3, s.atlas_y + 2), [3, 2, 7, 255]);
        assert_eq!(render_small(&c, "s/0", 1).get(3, 2), [3, 2, 7, 255]);
    }

    #[test]
    fn boxes_are_found_and_empty_ones_draw_nothing() {
        let img = RgbaImage::from_fn(6, 5, |x, y| if (2..=4).contains(&x) && (1..=2).contains(&y) { [1, 1, 1, 9] } else { [7, 7, 7, 0] });
        assert_eq!(opaque_box(&img), Some([2, 1, 3, 2]));
        assert_eq!(opaque_box(&RgbaImage::new(3, 3)), None);
        // A cell that is all blank: an empty box, nothing drawn shrunk
        // (columns 1 and 2 of `Columns`, made blank; drawn they are not).
        let text = format!("{BOXED}cell = [2, 4]\norigin = [1, 0]\nsample_from = \"box\"\n[blank_loader]\ntransparent = [1, 2]\n").replace("cell = [8, 4]\n", "");
        let c = Description::parse(&text).unwrap().compile(&Columns, 64).unwrap();
        let s = c.atlas.sprites[0];
        assert_eq!((s.box_w, s.box_h), (0, 0));
        assert!(render_small(&c, "s/0", 2).pixels.iter().all(|p| p[3] == 0));
        assert_eq!(render_small(&c, "s/0", 1).get(1, 0), [2, 0, 7, 255]);
    }

    // What compiling a description smaller made before views shown smaller
    // sampled the full-size art themselves: the art shrunk on the CPU. Kept
    // here only to show that the renderer draws exactly the same.

    /// Every `n`-th pixel of every `n`-th row, from the top-left.
    fn downscale(img: &RgbaImage, n: u32) -> RgbaImage {
        RgbaImage::from_fn(img.width / n, img.height / n, |x, y| img.get(x * n, y * n))
    }

    /// `img` shrunk by `n` the way a game does that keeps its sprites
    /// cropped to `bounds` (their box): it samples the box at every `n`-th
    /// pixel from the box's top-left, covering `box size / n` pixels
    /// (rounded down), drawn at the box's origin divided by `n` (rounded
    /// down); the cell's own size, rounded down, is the size of the result.
    fn downscale_from_box(img: &RgbaImage, n: u32, bounds: Option<[u32; 4]>) -> RgbaImage {
        let (w, h) = (img.width / n, img.height / n);
        let Some([bx, by, bw, bh]) = bounds else { return RgbaImage::new(w, h) };
        let (x0, y0) = (bx / n, by / n);
        RgbaImage::from_fn(w, h, |x, y| match (x.checked_sub(x0), y.checked_sub(y0)) {
            (Some(i), Some(j)) if i < bw / n && j < bh / n => img.get(bx + n * i, by + n * j),
            _ => [0; 4],
        })
    }

    #[test]
    fn the_old_cpu_shrink_samples_the_box() {
        let img = RgbaImage::from_fn(6, 5, |x, y| if (2..=4).contains(&x) && (1..=2).contains(&y) { [1, 1, 1, 9] } else { [7, 7, 7, 0] });
        assert!(downscale_from_box(&img, 2, None).pixels.iter().all(|p| p[3] == 0));
        let small = downscale_from_box(&img, 2, opaque_box(&img));
        assert_eq!((small.width, small.height), (3, 2));
        assert_eq!(small.pixels.iter().filter(|p| p[3] != 0).count(), 1);
        assert_eq!(small.get(1, 0), [1, 1, 1, 9]);
    }

    /// A 64 × 48 picture of palette indices: a blob of index 9 and 10 in
    /// every 16 × 12 cell, off centre by the cell's position (so boxes start
    /// at odd and even columns and rows), the rest index 255 or (cell
    /// corners) 254; drawn with 254 and 255 clear unless the options' list
    /// `transparent` says otherwise.
    struct Blobs;
    impl ImageLoader for Blobs {
        fn load(&self, _: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            let clear: Vec<i64> = match options.get("transparent").and_then(|t| t.as_array()) {
                Some(list) => list.iter().filter_map(|v| v.as_integer()).collect(),
                None => vec![254, 255],
            };
            Ok(vec![RgbaImage::from_fn(64, 48, |x, y| {
                let (c, r, cx, cy) = (x / 16, y / 12, x % 16, y % 12);
                let index = if (cx < 2 && cy < 2) || (cx > 13 && cy > 9 && c % 2 == 1) {
                    254
                } else if (1 + c + r..10 + c).contains(&cx) && (r + c % 2..7 + r).contains(&cy) {
                    9 + (cx + cy) % 2
                } else {
                    255
                };
                if clear.contains(&(index as i64)) { [0; 4] } else { [x as u8 * 3, y as u8 * 5, index as u8, 255] }
            })])
        }
    }

    #[test]
    fn views_shown_smaller_draw_what_the_cpu_shrink_did() {
        // A sheet of 4 × 4 cells drawn on a grid both ways: compiled once and
        // drawn by a view at downscale 2, and (as before) each sprite shrunk
        // on the CPU from the same cells (from its box: the box of what the
        // blank load leaves) and drawn 1:1 on a grid of half the steps, with
        // its layers' offsets and anchors halved (or kept) as each sheet
        // says. Sheets sampling from the cell and from the box, each scaling.
        let text = "[grid]\nlayout = \"staggered\"\nstep = [8, 6]\n\
            [blank_loader]\ntransparent = [255]\n\
            [images.a]\nsource = \"a\"\n\
            [sheets.cells]\nimage = \"a\"\ncell = [16, 12]\nanchor = [-3, -7]\n\
            [sheets.boxes]\nimage = \"a\"\ncell = [16, 12]\nanchor = [-5, 9]\nsample_from = \"box\"\n\
            [sheets.kept]\nimage = \"a\"\ncell = [16, 12]\nanchor = [-11, 1]\nsample_from = \"box\"\nscaling = \"scale_keep_anchor\"\n\
            [sheets.screen]\nimage = \"a\"\ncell = [16, 12]\ngrid = [1, 1]\nanchor = [1, -1]\nscaling = \"keep\"\n\
            [[layers]]\nname = \"a\"\nsets = [\"cells/*\"]\noffset = [-1, 3]\n\
            [[layers]]\nname = \"b\"\nsets = [\"boxes/*\"]\noffset = [5, -7]\n\
            [[layers]]\nname = \"c\"\nsets = [\"kept/*\", \"screen/*\"]\noffset = [-3, 0]\n";
        let desc = Description::parse(text).unwrap();
        let c = desc.compile(&Blobs, 256).unwrap();
        let (cols, rows) = (9, 14);
        let fill = |world: &mut tile_render::TileWorld, c: &Compiled| {
            for row in 0..rows {
                for col in 0..cols {
                    let k = (col * 7 + row * 3) % 16;
                    for (l, set) in [("a", format!("cells/{k}")), ("b", format!("boxes/{}", (k + 5) % 16)), ("c", format!("kept/{}", (k + 9) % 16))] {
                        world.set(tile_render::LayerId(c.layer_index(l).unwrap()), col, row, c.value(l, &set).unwrap());
                    }
                    if (col + row) % 5 == 0 {
                        world.set(tile_render::LayerId(2), col, row, c.value("c", "screen/0").unwrap());
                    }
                }
            }
        };
        let mut world = c.world(cols, rows, true, false);
        fill(&mut world, &c);
        // The old way: an atlas of the shrunk sprites, in the same order.
        let blank = Blobs.load("a", &desc.blank_loader).unwrap().remove(0);
        let full = Blobs.load("a", &toml::Table::new()).unwrap().remove(0);
        let mut atlas = AtlasBuilder::new(256);
        for name in &c.sprite_names {
            let (sheet, k) = name.split_once('/').unwrap();
            let k: u32 = k.parse().unwrap();
            let s = &desc.sheets[sheet];
            let (x, y) = (k % 4 * 16, k / 4 * 12);
            let cell = full.crop(x, y, 16, 12);
            let (image, anchor) = match (sheet, s.sample_from) {
                ("screen", _) => (cell, s.anchor),
                (_, SampleFrom::Box) => (downscale_from_box(&cell, 2, opaque_box(&blank.crop(x, y, 16, 12))), s.anchor),
                (_, SampleFrom::Cell) => (downscale(&cell, 2), s.anchor),
            };
            let anchor = if s.scaling == SheetScaling::Scale { anchor.map(|a| a / 2) } else { anchor };
            atlas.add(&image, (anchor[0], anchor[1])).unwrap();
        }
        let atlas = atlas.finish();
        let mut old = c.world(cols, rows, true, false);
        old.grid.step = (4, 3);
        for (l, spec) in c.layers.iter().enumerate() {
            old.layer_mut(tile_render::LayerId(l)).offset = (spec.offset.0 / 2, spec.offset.1 / 2);
        }
        fill(&mut old, &c);
        let gpu = tile_render::Headless::new().expect("GPU");
        for (x, y) in [(0, 0), (-7, 3), (11, -5), (29, 17)] {
            let camera = tile_render::Camera { x, y, overhang: (8, 8, 8, 8), ..tile_render::Camera::new((40, 36)) };
            let want = tile_render::reference::render(&old, &atlas, &camera, 0, [0, 0, 0, 255]);
            let got = tile_render::reference::render(&world, &c.atlas, &tile_render::Camera { downscale: 2, ..camera }, 0, [0, 0, 0, 255]);
            assert_eq!(got, want, "camera at ({x}, {y})");
            // The shaders draw the same.
            let mut renderer = tile_render::TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
            renderer.set_atlas(&gpu.device, &gpu.queue, &c.atlas);
            renderer.set_world(&gpu.device, &world);
            assert_eq!(gpu.render(&mut renderer, &tile_render::Camera { downscale: 2, ..camera }, 0, [0, 0, 0, 255]), want, "GPU, camera at ({x}, {y})");
            assert!(want.pixels.iter().filter(|p| p[..3] != [0, 0, 0]).count() > 400);
        }
    }
}
