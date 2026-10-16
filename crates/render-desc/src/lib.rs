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
    /// Everything else, for the loader.
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
    /// See [`Description::parse`].
    pub variants: Option<Vec<i64>>,
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

    /// Loads every image, slices the sheets and builds the tile renderer's
    /// tables. `page_size` is the atlas page size (e.g. 4096).
    pub fn compile(&self, loader: &dyn ImageLoader, page_size: u32) -> Result<Compiled, DescError> {
        let mut atlas = AtlasBuilder::new(page_size);
        let mut images: HashMap<&str, Vec<RgbaImage>> = HashMap::new();
        for (name, img) in &self.images {
            let mut options = self.loader.clone();
            options.extend(img.options.clone());
            let generated = if let Some(spec) = img.source.strip_prefix("solid:") {
                Some(solid(spec))
            } else {
                img.source.strip_prefix("line:").map(line)
            };
            let frames = match generated {
                Some(image) => vec![image.map_err(|message| DescError::Image { name: name.clone(), message })?],
                None => loader.load(&img.source, &options).map_err(|message| DescError::Image { name: name.clone(), message })?,
            };
            images.insert(name, frames);
        }

        // Sprites, named "sheet/index".
        let mut sprites: BTreeMap<String, SpriteId> = BTreeMap::new();
        let mut sheet_len: HashMap<&str, u32> = HashMap::new();
        for (sheet_name, sheet) in &self.sheets {
            let err = |message: String| DescError::Sheet { sheet: sheet_name.clone(), message };
            let frames = images.get(sheet.image.as_str()).ok_or(DescError::Unknown { kind: "image", name: sheet.image.clone() })?;
            let img = frames.get(sheet.frame).ok_or_else(|| err(format!("image has no frame {}", sheet.frame)))?;
            let [cw, ch] = sheet.cell;
            let [ox, oy] = sheet.origin;
            let [sx, sy] = sheet.spacing;
            let fit = |total: u32, o: u32, c: u32, s: u32| if total < o + c { 0 } else { (total - o + s) / (c + s) };
            let [cols, rows] = sheet.grid.unwrap_or([fit(img.width, ox, cw, sx), fit(img.height, oy, ch, sy)]);
            if ox + cols * (cw + sx) - sx > img.width || oy + rows * (ch + sy) - sy > img.height {
                return Err(err(format!("{cols}×{rows} cells of {cw}×{ch} exceed the {}×{} image", img.width, img.height)));
            }
            for i in 0..cols * rows {
                if sheet.only.as_ref().is_some_and(|o| !o.contains(&i)) {
                    continue;
                }
                let (c, r) = (i % cols, i / cols);
                let sprite = img.crop(ox + c * (cw + sx), oy + r * (ch + sy), cw, ch);
                let id = atlas.add(&sprite, (sheet.anchor[0], sheet.anchor[1]))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
