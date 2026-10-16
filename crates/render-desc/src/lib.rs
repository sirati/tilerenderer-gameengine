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
use tile_render::{AtlasBuilder, Grid, LayerSpec, Layout, RgbaImage, SpriteId, SpriteSetId, SpriteSets};

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
}

/// One frame of a set: a sprite name, or `{ sheet, index }`-style shorthand
/// `"sheet/index"`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDesc {
    pub frames: Vec<String>,
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
}

fn one() -> u32 {
    1
}

impl Description {
    pub fn parse(text: &str) -> Result<Self, DescError> {
        Ok(toml::from_str(text)?)
    }

    /// Loads every image, slices the sheets and builds the tile renderer's
    /// tables. `page_size` is the atlas page size (e.g. 4096).
    pub fn compile(&self, loader: &dyn ImageLoader, page_size: u32) -> Result<Compiled, DescError> {
        let mut atlas = AtlasBuilder::new(page_size);
        let mut images: HashMap<&str, Vec<RgbaImage>> = HashMap::new();
        for (name, img) in &self.images {
            let mut options = self.loader.clone();
            options.extend(img.options.clone());
            let frames = loader
                .load(&img.source, &options)
                .map_err(|message| DescError::Image { name: name.clone(), message })?;
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
        for (name, &sprite) in &sprites {
            set_ids.insert(name.clone(), sets.single(sprite));
        }
        for (name, set) in &self.sets {
            let frames = set
                .frames
                .iter()
                .map(|f| sprites.get(f).copied().ok_or(DescError::Unknown { kind: "sprite", name: f.clone() }))
                .collect::<Result<Vec<_>, _>>()?;
            set_ids.insert(name.clone(), sets.add(&frames));
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
        })
    }
}

/// A description turned into renderer tables.
pub struct Compiled {
    pub layout: Layout,
    pub step: (u32, u32),
    pub atlas: tile_render::Atlas,
    pub sets: SpriteSets,
    pub layers: Vec<LayerSpec>,
    /// Name of every sprite, by `SpriteId`.
    pub sprite_names: Vec<String>,
    values: HashMap<(String, String), u32>,
}

impl Compiled {
    /// The cell value that draws set `set` on layer `layer` (0 = nothing).
    pub fn value(&self, layer: &str, set: &str) -> Option<u32> {
        self.values.get(&(layer.to_owned(), set.to_owned())).copied()
    }

    pub fn layer_index(&self, layer: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == layer)
    }

    /// An empty world of `cols × rows` cells with all layers.
    pub fn world(&self, cols: u32, rows: u32, wrap_x: bool, wrap_y: bool) -> tile_render::TileWorld {
        let grid = Grid { cols, rows, layout: self.layout, wrap_x, wrap_y, step: self.step };
        let mut world = tile_render::TileWorld::new(grid, self.sets.clone());
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
    fn rejects_unknown_names() {
        let d = Description::parse("grid = { layout = \"orthogonal\", step = [1, 1] }\n[[layers]]\nname = \"x\"\nsets = [\"nope\"]").unwrap();
        assert!(matches!(d.compile(&Checker, 64), Err(DescError::Unknown { kind: "set", .. })));
    }
}
