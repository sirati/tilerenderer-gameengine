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

mod fallback_font;

/// Embedded GPU outline face for viewer warnings and confirmations.
pub const FALLBACK_FONT: &str = "Viewer@fallback";

mod cancel;

pub use cancel::{CANCELLED, StopWhen};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Deserialize;
use tile_render::{AtlasBuilder, AtlasGrowth, Blend, Blit, BlitPixels, ColorModel, Grid, IndexImage, LayerSpec, Layout, RgbaImage, Shelves, SpriteId, SpriteSetId, SpriteSets};

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

    /// What `source` (loaded with `options`) stands for right now, e.g. the
    /// files it and the files its options name resolve to among search
    /// folders that change: sheets (and indexed sprites' indices) are kept
    /// from one compile for the next ([`SheetCache`]) only while it is the
    /// same. By default the source itself.
    fn identity(&self, source: &str, _options: &toml::Table) -> String {
        source.to_owned()
    }

    /// The frames of `source` as palette indices, each with the colour every
    /// index is drawn in (see [`IndexedFrame`]), where the loader can tell
    /// the image so with these options: its pixels are a palette's indices
    /// and the options change only which colour an index stands for. `None`:
    /// not so (e.g. colours computed per pixel); [`ImageLoader::load`] then
    /// makes the image. Indexed sprites take a quarter of the memory, and
    /// sprites of the same pixels in other colours share theirs.
    fn indexed(&self, _source: &str, _options: &toml::Table) -> Result<Option<Vec<IndexedFrame>>, String> {
        Ok(None)
    }

    /// The colours of palette `source` (see [`Description::palettes`]): up to
    /// 256 RGBA8 entries, as the loader would draw an image's pixels of
    /// those colours (the same colour reduction).
    fn palette(&self, source: &str, _options: &toml::Table) -> Result<Vec<[u8; 4]>, String> {
        Err(format!("{source}: this loader has no palettes"))
    }
}

/// One frame of an indexed image (see [`ImageLoader::indexed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedFrame {
    /// Palette indices; they must not depend on the load's options.
    pub pixels: IndexImage,
    /// The colour each index is drawn in, as [`ImageLoader::load`] would
    /// draw it with the same options (transparent and translucent ones too).
    pub palette: Vec<[u8; 4]>,
}

/// An image's frames as loaded: colours, or palette indices with each
/// frame's own palette row (frames of one palette sharing it).
#[derive(Clone)]
enum Frames {
    Rgba(std::rc::Rc<Vec<RgbaImage>>),
    Indexed { frames: std::rc::Rc<Vec<IndexedFrame>>, rows: Vec<std::sync::Arc<Vec<u32>>> },
}

impl Frames {
    fn len(&self) -> usize {
        match self {
            Frames::Rgba(f) => f.len(),
            Frames::Indexed { frames, .. } => frames.len(),
        }
    }

    fn size(&self, frame: usize) -> (u32, u32) {
        match self {
            Frames::Rgba(f) => (f[frame].width, f[frame].height),
            Frames::Indexed { frames, .. } => (frames[frame].pixels.width, frames[frame].pixels.height),
        }
    }

    /// The colour of pixel (`x`, `y`) of `frame`.
    fn pixel(&self, frame: usize, x: u32, y: u32) -> [u8; 4] {
        match self {
            Frames::Rgba(f) => f[frame].get(x, y),
            Frames::Indexed { frames, .. } => {
                let f = &frames[frame];
                f.palette.get(f.pixels.get(x, y) as usize).copied().unwrap_or_default()
            }
        }
    }
}

/// The sprites of one sheet as compiled (named by the compile: the same
/// art may be another sheet's next time).
struct Sliced {
    sprites: Vec<SlicedSprite>,
    /// How many cells the sheet has (`only` may leave some out).
    cells: u32,
}

/// Where an indexed sprite's indices come from: image source, frame, and
/// the cell (x, y, width, height). Sprites of the same pixels share them in
/// the atlas, whatever their colours (e.g. a unit's body and its shadow).
type TexelKey = (String, usize, [u32; 4], Option<AlphaTexels>);

/// A source-alpha edit changes the stored indices even when the source
/// crop stays the same. Its replacement index also depends on the palette.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AlphaTexels {
    keep: bool,
    rects: Vec<[u32; 4]>,
    transparent: u8,
}

#[derive(Clone, Copy)]
struct SourceAlphaMask<'a> {
    keep: bool,
    rects: &'a [[u32; 4]],
}

impl SourceAlphaMask<'_> {
    fn retains(self, x: u32, y: u32) -> bool {
        let inside = self.rects.iter().any(|&[rx, ry, w, h]| x >= rx && y >= ry && x - rx < w && y - ry < h);
        inside == self.keep
    }

    fn rgba(self, mut image: RgbaImage) -> RgbaImage {
        for y in 0..image.height {
            for x in 0..image.width {
                if !self.retains(x, y) {
                    image.pixels[(y * image.width + x) as usize][3] = 0;
                }
            }
        }
        image
    }

    fn indexed(self, image: std::sync::Arc<IndexImage>, row: std::sync::Arc<Vec<u32>>, mut key: TexelKey, remap: u32, shared: &mut SharedTexels) -> Result<Cut, String> {
        // An index inside the remapped range can become opaque through a
        // named palette, including when its own palette entry is clear.
        let remapped = |i: usize| i as u32 >= (remap & 0xFFFF) && (i as u32) < (remap >> 16);
        let mut used = [false; 256];
        let mut drops_pixels = false;
        for y in 0..image.height {
            for x in 0..image.width {
                if self.retains(x, y) {
                    used[image.get(x, y) as usize] = true;
                } else {
                    drops_pixels = true;
                }
            }
        }
        if !drops_pixels {
            return Ok(Cut::Indexed { image, row, key });
        }
        let transparent = (0..256).find(|&i| !remapped(i) && row[i] >> 24 == 0)
            .or_else(|| (0..256).find(|&i| !remapped(i) && !used[i]));
        let Some(transparent) = transparent else {
            if (remap & 0xFFFF) < (remap >> 16) {
                return Err("source alpha mask has no safe transparent index outside its remapped palette range (indexed = false)".into());
            }
            // All indices are still used. Converting this cell preserves
            // every retained colour and alpha without changing its palette.
            return Ok(Cut::Rgba(self.rgba(RgbaImage::from_fn(image.width, image.height, |x, y| row[image.get(x, y) as usize].to_le_bytes()))));
        };
        key.3 = Some(AlphaTexels { keep: self.keep, rects: self.rects.to_vec(), transparent: transparent as u8 });
        let masked = shared.entry(key.clone()).or_insert_with(|| {
            std::sync::Arc::new(IndexImage::from_fn(image.width, image.height, |x, y| if self.retains(x, y) { image.get(x, y) } else { transparent as u8 }))
        }).clone();
        let row = if row[transparent] >> 24 == 0 {
            row
        } else {
            let mut masked_row = (*row).clone();
            masked_row[transparent] &= 0x00FF_FFFF;
            std::sync::Arc::new(masked_row)
        };
        Ok(Cut::Indexed { image: masked, row, key })
    }
}

/// The indices of every indexed sprite sliced, by where they come from:
/// sprites of the same pixels (a unit's body, its shadow, an army's banner
/// art) hold one copy, across compiles too (see `SheetCache`).
type SharedTexels = HashMap<TexelKey, std::sync::Arc<IndexImage>>;

/// A sprite's pixels as sliced.
enum Cut {
    Rgba(RgbaImage),
    /// Palette indices, the colours of its own palette row (entry `i` of
    /// `row` for index `i`) and where the indices come from.
    Indexed { image: std::sync::Arc<IndexImage>, row: std::sync::Arc<Vec<u32>>, key: TexelKey },
}

struct SlicedSprite {
    /// The cell's index in the sheet.
    index: u32,
    /// The pixels, if compiled with them.
    image: Option<Cut>,
    size: (u32, u32),
    anchor: (i32, i32),
    /// What a view showing the world smaller samples (x, y, width, height;
    /// see `tile_render_shared::SpriteDesc::box_x`): the whole sprite, or
    /// its box (`sample_from = "box"`, known with the pixels only).
    sample_box: [u32; 4],
    /// Source-image coordinates used to anchor background-only dithering.
    sample_origin: [u32; 2],
    /// The cell drawn instead there (`scaled_origin`): its pixels (with
    /// the pixels) and sample box.
    scaled: Option<(Option<Cut>, [u32; 4])>,
}

/// The sheets of the last [`Description::compile_cached`], by what makes
/// them, so that the next compile of a description that has them too (e.g.
/// one that grew) only loads and slices the new ones. Holds the sprites'
/// pixels (indexed ones one byte a pixel): about the atlas's size.
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

/// Places a compile's sprites: with their pixels into an atlas (a whole
/// compile) or as blits of a growth (indexed sprites of the same pixels
/// sharing them, own palette rows of the same colours shared), or (without
/// pixels) their sizes only.
struct Placer {
    pixels: bool,
    page_size: u32,
    target: Target,
    /// The id of the first sprite placed (a growth: after those before).
    first: u32,
    /// The first sprite of each indexed sprite's pixels (also of earlier
    /// compiles, for a growth).
    texels: HashMap<TexelKey, SpriteId>,
    /// Palette rows: those of the table before (a growth), the rows this
    /// compile adds (a whole compile: row 0, then one per distinct
    /// colouring), and each colouring's row.
    row_base: u32,
    own: Vec<u32>,
    rows: HashMap<std::sync::Arc<Vec<u32>>, u32>,
    /// Sprite ids whose source art could not be placed, even though their
    /// transparent placeholders retain the normal numeric ids.
    unavailable: BTreeSet<u32>,
}

/// Where a [`Placer`] puts sprites.
enum Target {
    /// A whole atlas with its pixels.
    Atlas(AtlasBuilder),
    /// A growth with pixels: the next places, the new pixels and sprites.
    Grow { rgba: Shelves, index: Shelves, blits: Vec<Blit>, sprites: Vec<tile_render_shared::SpriteDesc> },
    /// Without pixels: the sprites' sizes and anchors.
    Sizes(Vec<tile_render_shared::SpriteDesc>),
}

impl Placer {
    fn new(page_size: u32, pixels: bool) -> Self {
        Self {
            pixels,
            page_size,
            target: if pixels { Target::Atlas(AtlasBuilder::new(page_size)) } else { Target::Sizes(Vec::new()) },
            first: 0,
            texels: HashMap::new(),
            row_base: 0,
            own: tile_render::atlas::no_palettes(),
            rows: HashMap::new(),
            unavailable: BTreeSet::new(),
        }
    }

    /// A placer for a growth of `growing`'s art, which has `first` sprites
    /// and a palette table of `rows` rows.
    fn growing(growing: &Growing, first: u32, rows: u32) -> Self {
        Self {
            pixels: growing.pixels,
            page_size: growing.page_size,
            target: if growing.pixels {
                Target::Grow { rgba: growing.shelves.0.clone(), index: growing.shelves.1.clone(), blits: Vec::new(), sprites: Vec::new() }
            } else {
                Target::Sizes(Vec::new())
            },
            first,
            texels: growing.texels.clone(),
            row_base: rows,
            own: Vec::new(),
            rows: growing.rows.clone(),
            unavailable: BTreeSet::new(),
        }
    }

    /// Whether this sprite's art was replaced by a placeholder, including
    /// sprites from the earlier compilation that a growth refers to.
    fn unavailable(&self, id: SpriteId, known: Option<&Compiled>) -> bool {
        self.unavailable.contains(&id.0) || known.is_some_and(|compiled| compiled.unavailable.contains(&id.0))
    }

    /// The sprites placed (of a growth or a compile without pixels).
    fn placed(&self) -> &[tile_render_shared::SpriteDesc] {
        match &self.target {
            Target::Atlas(_) => &[],
            Target::Grow { sprites, .. } | Target::Sizes(sprites) => sprites,
        }
    }

    /// Sprite `id`, placed by this placer or (`known`) before.
    fn sprite(&self, id: SpriteId, known: Option<&Compiled>) -> Option<tile_render_shared::SpriteDesc> {
        if id.0 < self.first {
            return known.and_then(|k| k.atlas.sprites.get(id.0 as usize)).copied();
        }
        match &self.target {
            Target::Atlas(a) => Some(*a.sprite(id)),
            Target::Grow { sprites, .. } | Target::Sizes(sprites) => sprites.get((id.0 - self.first) as usize).copied(),
        }
    }

    fn sprite_mut(&mut self, id: SpriteId) -> &mut tile_render_shared::SpriteDesc {
        match &mut self.target {
            Target::Atlas(a) => a.sprite_mut(id),
            Target::Grow { sprites, .. } | Target::Sizes(sprites) => &mut sprites[(id.0 - self.first) as usize],
        }
    }

    fn push(&mut self, desc: tile_render_shared::SpriteDesc) -> SpriteId {
        match &mut self.target {
            Target::Atlas(_) => unreachable!("a whole atlas places sprites itself"),
            Target::Grow { sprites, .. } | Target::Sizes(sprites) => {
                sprites.push(desc);
                SpriteId(self.first + sprites.len() as u32 - 1)
            }
        }
    }

    /// A sprite of pixels placed before (part of another sprite's, `desc`
    /// says where); a whole atlas: `of`'s pixels.
    fn alias(&mut self, desc: tile_render_shared::SpriteDesc) -> SpriteId {
        match &mut self.target {
            Target::Atlas(a) => a.add_desc(desc),
            _ => self.push(desc),
        }
    }

    /// Adds sprite `name` (`cut`: its pixels, `None` without pixels),
    /// drawn with `scaling` from `sample_box` by views showing the world
    /// smaller; an indexed one with its indices in `remap` remapped.
    /// `known`: the art a growth is appended to. A sprite larger than the
    /// pages is left out (logged): it keeps its id but is a clear 1×1
    /// pixel, the same with or without the pixels, so that one such sprite
    /// does not cost all the others.
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, name: &str, cut: Option<&Cut>, (w, h): (u32, u32), anchor: (i32, i32), scaling: u32, sample_box: [u32; 4], remap: u32, known: Option<&Compiled>) -> Result<SpriteId, DescError> {
        if w > self.page_size || h > self.page_size {
            log::warn!("sprite {name} of {w}×{h} does not fit an atlas page of {0}×{0}: left out", self.page_size);
            let blank = cut.map(|_| Cut::Rgba(RgbaImage::new(1, 1)));
            let id = self.add(name, blank.as_ref(), (1, 1), anchor, scaling, [0, 0, 1, 1], 0, known)?;
            self.unavailable.insert(id.0);
            return Ok(id);
        }
        let id = match cut {
            None => {
                if w > self.page_size || h > self.page_size {
                    return Err(tile_render::atlas::AtlasError::TooLarge(w, h, self.page_size).into());
                }
                self.push(tile_render_shared::SpriteDesc { width: w, height: h, anchor_x: anchor.0, anchor_y: anchor.1, ..Default::default() })
            }
            Some(Cut::Rgba(image)) => match &mut self.target {
                Target::Atlas(a) => a.add(image, anchor)?,
                Target::Grow { rgba, blits, .. } => {
                    let at = rgba.place(image.width, image.height)?;
                    blits.push(Blit { page: at.0, x: at.1, y: at.2, pixels: BlitPixels::Rgba(image.clone()) });
                    self.push(tile_render::atlas::sprite_at(at, (image.width, image.height), anchor))
                }
                Target::Sizes(_) => unreachable!("pixels without pixels"),
            },
            Some(Cut::Indexed { image, row, key }) => {
                let own = match self.rows.get(row) {
                    Some(&r) => r,
                    None => {
                        let r = self.row_base + (self.own.len() / tile_render_shared::PALETTE_SIZE as usize) as u32;
                        self.own.extend_from_slice(row);
                        self.rows.insert(row.clone(), r);
                        r
                    }
                };
                let shared = self.texels.get(key).and_then(|&first| self.sprite(first, known));
                let id = match (shared, &mut self.target) {
                    (Some(s), _) => {
                        let desc = tile_render::atlas::sprite_at((s.page, s.atlas_x, s.atlas_y), (s.width, s.height), anchor);
                        self.alias(desc)
                    }
                    (None, Target::Atlas(a)) => {
                        let id = a.add_indexed(image, anchor, own, remap)?;
                        self.texels.insert(key.clone(), id);
                        id
                    }
                    (None, Target::Grow { index, blits, .. }) => {
                        let at = index.place(image.width, image.height)?;
                        blits.push(Blit { page: at.0, x: at.1, y: at.2, pixels: BlitPixels::Index((**image).clone()) });
                        let id = self.push(tile_render::atlas::sprite_at(at, (image.width, image.height), anchor));
                        self.texels.insert(key.clone(), id);
                        id
                    }
                    (None, Target::Sizes(_)) => unreachable!("pixels without pixels"),
                };
                let s = self.sprite_mut(id);
                (s.palette, s.remap) = (own, remap);
                id
            }
        };
        let desc = self.sprite_mut(id);
        (desc.scaling, desc.box_x, desc.box_y, desc.box_w, desc.box_h) = (scaling, sample_box[0], sample_box[1], sample_box[2], sample_box[3]);
        Ok(id)
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
    /// The largest sprite art grown onto this description may bring (e.g.
    /// a picture of a whole screen), if larger than its own sheets': its
    /// atlas pages are made for it from the start ([`Description::largest_sprite`]),
    /// as pages do not grow.
    #[serde(default)]
    pub largest: u32,
    /// Named palettes (see [`PaletteDesc`]): rows an indexed image's
    /// remapped indices (`remap = [lo, hi]`) are drawn through, picked per
    /// draw: by a layer with `palettes = true` ([`Compiled::value_in`]) or a
    /// free sprite ([`Compiled::palette`]). One copy of the art for every
    /// colouring (e.g. a unit in each nation's colours). Images are indexed
    /// (one byte a pixel, their own palette a row) wherever the loader can
    /// load them so ([`ImageLoader::indexed`]; `indexed = false` asks for
    /// colours).
    #[serde(default)]
    pub palettes: BTreeMap<String, PaletteDesc>,
}

/// A palette row: the colours of `source` (loaded by
/// [`ImageLoader::palette`], with the image options of `loader`), or those of
/// palette `base`; then `copy` entries `[palette, from, to, count]`: `count`
/// entries of another palette from `from` on, written from `to` on.
/// `variants` as for images.
#[derive(Clone, Debug, Deserialize)]
pub struct PaletteDesc {
    pub source: Option<String>,
    pub base: Option<String>,
    #[serde(default)]
    pub copy: Vec<(String, u32, u32, u32)>,
    /// Everything else, for the loader.
    #[serde(flatten)]
    pub options: toml::Table,
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
    /// The image the sprites are cut from…
    #[serde(default)]
    pub image: String,
    /// …or a sprite compiled before (of a sheet of an image, or of an
    /// earlier description this one is appended to, see [`Growing`]): the
    /// sprites are then parts of its pixels in the atlas, which they share
    /// (no pixels of their own), e.g. regions of a whole picture.
    pub sprite: Option<String>,
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
    /// Each sprite's anchor marked in the image next to its cell, as games
    /// mark a mouse cursor's hot spot: a pixel of this colour (RGB, each
    /// channel within 8) in the row just above the cell is the hot spot's
    /// x, one in the column just left of it its y; the sprite's anchor is
    /// minus that offset (`anchor` where a cell has no marks).
    pub anchor_marks: Option<[u8; 3]>,
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
    /// Explicit pure UI background role. Ordinary art, controls and text
    /// retain nearest sampling. Does not apply to world-space draws.
    #[serde(default)]
    pub sampling: Sampling,
    /// Preserve source alpha only inside the union of these `[x, y, width,
    /// height]` rectangles, relative to each original, unresized cell.
    /// Elsewhere alpha becomes zero. Applied before outline and resize.
    /// Mutually exclusive with `alpha_clear`; requires cell sampling and an
    /// image source, rather than a sheet aliasing an existing sprite.
    pub alpha_keep: Option<Vec<[u32; 4]>>,
    /// Clear source alpha inside these cell-relative rectangles, preserving
    /// it elsewhere. Complements `alpha_keep` without changing geometry,
    /// retained pixel alpha, or the original source sampling coordinates.
    pub alpha_clear: Option<Vec<[u32; 4]>>,
    /// Each sprite replaced by a band `width` pixels wide just outside its
    /// art, in `color` (a ring around a ring); the sprite grows by `width` on
    /// every side, its anchor moves accordingly.
    pub outline: Option<OutlineDesc>,
    /// Each cell resampled to this size, nearest pixel (e.g. a game's icon
    /// fitted into a smaller picture); after `outline`. Colour images only
    /// (`indexed = false`).
    pub resize: Option<[u32; 2]>,
    /// Explicit source increments for a legacy16.16 nearest sampler.
    /// Used only with `resize`; each output pixel samples floor(i*step/65536)
    /// from the original cell rather than inferring scale from its final size.
    pub resize_step: Option<[u32; 2]>,
    /// See [`Description::parse`].
    pub variants: Option<Vec<i64>>,
    /// Cut every frame of the image (`frame` aside), one after the other:
    /// sprite `f·n + i` is cell `i` of frame `f` (`n` cells a frame).
    #[serde(default)]
    pub all_frames: bool,
}

impl SheetDesc {
    fn alpha_mask(&self) -> Option<SourceAlphaMask<'_>> {
        self.alpha_keep.as_deref().map(|rects| SourceAlphaMask { keep: true, rects })
            .or_else(|| self.alpha_clear.as_deref().map(|rects| SourceAlphaMask { keep: false, rects }))
    }

    fn validate_alpha_mask(&self) -> Result<(), String> {
        if self.alpha_keep.is_some() && self.alpha_clear.is_some() {
            return Err("alpha_keep and alpha_clear are mutually exclusive".into());
        }
        let Some(mask) = self.alpha_mask() else { return Ok(()) };
        if self.sample_from == SampleFrom::Box {
            // Complementary masks must share an unchanged sampling grid.
            // Finding separate opaque boxes would change their phase.
            return Err("source alpha masks require sample_from = 'cell'".into());
        }
        if self.sprite.is_some() {
            return Err("source alpha masks require an image source, not sprite parts".into());
        }
        let [cw, ch] = self.cell;
        for &[x, y, w, h] in mask.rects {
            if w == 0 || h == 0 || u64::from(x) + u64::from(w) > u64::from(cw) || u64::from(y) + u64::from(h) > u64::from(ch) {
                return Err(format!("source alpha rectangle [{x}, {y}, {w}, {h}] is empty or outside its {cw}×{ch} cell"));
            }
        }
        Ok(())
    }
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
    /// Clock ticks per animation frame; 0: a cell shows the frame its phase
    /// says, whatever the clock.
    #[serde(default = "one")]
    pub ticks_per_frame: u32,
    #[serde(default)]
    pub per_cell_phase: bool,
    /// Cells carry a pixel offset each (`TileWorld::set_offset`).
    #[serde(default)]
    pub per_cell_offset: bool,
    /// The pass sweeps rows right to left (all layers of a pass must agree).
    #[serde(default)]
    pub descending: bool,
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
    /// Values also pick the named palette the layer's indexed sprites'
    /// remapped indices are drawn through (see [`Compiled::value_in`]).
    #[serde(default)]
    pub palettes: bool,
    /// Visibility category bit for per-camera clean-map filtering. Zero is
    /// always visible. The value is a bit mask so a layer may belong to more
    /// than one category.
    #[serde(default)]
    pub category: u32,
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

/// Sources made without the loader (full colour).
const GENERATED: [&str; 8] = ["solid:", "line:", "arrow:", "crosshair:", "panes:", "pointer:", "rects:", "glyphs:"];

/// The characters of a [`glyphs`] image, one cell each, in this order.
pub const GLYPHS: &str = " 0123456789.%fpsmkingartCPUGbuld/~";

/// The rows of each of [`GLYPHS`] in 5 × 7 pixels (bit 4 the left column).
const GLYPH_ROWS: [[u8; 7]; 34] = [
    [0; 7],
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
    [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
    [0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011],
    [0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000],
    [0b00000, 0b00000, 0b11110, 0b10001, 0b11110, 0b10000, 0b10000],
    [0b00000, 0b00000, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110],
    [0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10001, 0b10001],
    [0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010],
    [0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001],
    [0b00000, 0b00000, 0b01111, 0b10001, 0b01111, 0b00001, 0b01110],
    [0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111],
    [0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000],
    [0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110],
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110], // C
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000], // P
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110], // U
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110], // G
    [0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b11110], // b
    [0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10011, 0b01101], // u
    [0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // l
    [0b00001, 0b00001, 0b01101, 0b10011, 0b10001, 0b10001, 0b01111], // d
    [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000], // /
    [0b00000, 0b00000, 0b01001, 0b10110, 0b00000, 0b00000, 0b00000], // ~

];

/// The cell of each glyph of a [`glyphs`] image at scale `n`: a 5 × 7 glyph
/// with a column to its right and a row above and below it, every pixel
/// `n × n`.
pub fn glyph_cell(n: u32) -> (u32, u32) {
    (6 * n, 9 * n)
}

/// `source = "glyphs:R,G,B,A:R,G,B,A:N"`: the characters of [`GLYPHS`] side
/// by side, each in a cell of [`glyph_cell`]`(N)` (1..=8), drawn in the
/// first colour on the second (cells side by side make a box behind a
/// line of text): small text such as a counter, sharp at any size.
fn glyphs(spec: &str) -> Result<RgbaImage, String> {
    let err = || format!("glyphs image {spec:?}: expected R,G,B,A:R,G,B,A:N (N 1..=8)");
    let parts: Vec<&str> = spec.split(':').collect();
    let [fg, bg, n] = parts[..] else { return Err(err()) };
    let color = |s: &str| -> Result<[u8; 4], String> {
        let c: Vec<u8> = s.split(',').map(|v| v.trim().parse().map_err(|_| err())).collect::<Result<_, _>>()?;
        <[u8; 4]>::try_from(c).map_err(|_| err())
    };
    let (fg, bg) = (color(fg)?, color(bg)?);
    let n: u32 = n.trim().parse().ok().filter(|n| (1..=8).contains(n)).ok_or_else(err)?;
    let (cw, ch) = glyph_cell(n);
    Ok(RgbaImage::from_fn(cw * GLYPH_ROWS.len() as u32, ch, |x, y| {
        let (glyph, gx, gy) = ((x / cw) as usize, (x % cw) / n, (y / n) as i32 - 1);
        let on = gx < 5 && (0..7).contains(&gy) && GLYPH_ROWS[glyph][gy as usize] >> (4 - gx) & 1 == 1;
        if on { fg } else { bg }
    }))
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

/// How a [`panes`] picture divides its screen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PanesSplit {
    /// One pane.
    Standard,
    /// Side by side, equal widths.
    Vertical1To1,
    /// Side by side, the left twice as wide.
    Vertical2To1,
    /// One over the other, equal heights.
    Horizontal1To1,
}

impl PanesSplit {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "standard" => Self::Standard,
            "vertical11" => Self::Vertical1To1,
            "vertical21" => Self::Vertical2To1,
            "horizontal11" => Self::Horizontal1To1,
            _ => return None,
        })
    }
}

/// The divider of a `w × h` [`panes`] picture: `Some((true, x))` the column
/// of a vertical one, `Some((false, y))` the row of a horizontal one.
fn panes_divider(split: PanesSplit, w: i32, h: i32) -> Option<(bool, i32)> {
    let (iw, ih) = (w - 2, h - 2);
    match split {
        PanesSplit::Standard => None,
        PanesSplit::Vertical1To1 => Some((true, 1 + iw / 2)),
        PanesSplit::Vertical2To1 => Some((true, 1 + iw * 2 / 3)),
        PanesSplit::Horizontal1To1 => Some((false, 1 + ih / 2)),
    }
}

/// The panes of a `w × h` [`panes`] picture (inside its frame and apart from
/// its divider), first pane first: x, y, width, height.
pub fn panes_rects(split: PanesSplit, (w, h): (u32, u32)) -> Vec<[i32; 4]> {
    let (w, h) = (w as i32, h as i32);
    let (iw, ih) = (w - 2, h - 2);
    match panes_divider(split, w, h) {
        None => vec![[1, 1, iw, ih]],
        Some((true, x)) => vec![[1, 1, x - 1, ih], [x + 1, 1, w - 2 - x, ih]],
        Some((false, y)) => vec![[1, 1, iw, y - 1], [1, y + 1, iw, h - 2 - y]],
    }
}

/// The middle of each pane of a [`panes`] picture (where a label goes).
pub fn panes_centres(split: PanesSplit, size: (u32, u32)) -> Vec<(i32, i32)> {
    panes_rects(split, size).iter().map(|[x, y, w, h]| (x + w / 2, y + h / 2)).collect()
}

/// `source = "panes:R,G,B,A:WxH:SPLIT"` or `"…:SPLIT:free"`: a screen
/// divided into panes, as a small picture for choosing among arrangements:
/// a one-pixel frame and divider in the colour, clear inside. `SPLIT`:
/// `standard` (one pane), `vertical11`, `vertical21` (the left twice as
/// wide), `horizontal11`. `free`: panes that move on their own: the middle
/// third of the divider left out and small arrows pointing away from each
/// pane's middle in the four directions ([`panes_centres`]; a label goes
/// there).
fn panes(spec: &str) -> Result<RgbaImage, String> {
    let err = || format!("panes image {spec:?}: expected R,G,B,A:WxH:SPLIT[:free] (SPLIT standard, vertical11, vertical21 or horizontal11; at least 8x8)");
    let mut parts = spec.splitn(3, ':');
    let (Some(rgba), Some(size), Some(rest)) = (parts.next(), parts.next(), parts.next()) else { return Err(err()) };
    let (color, (w, h)) = shape_spec(&format!("{rgba}:{size}"), "panes").map_err(|_| err())?;
    let (split, free) = match rest.split_once(':') {
        Some((s, "free")) => (s, true),
        Some(_) => return Err(err()),
        None => (rest, false),
    };
    let split = PanesSplit::parse(split).ok_or_else(err)?;
    if w < 8 || h < 8 {
        return Err(err());
    }
    let (wi, hi) = (w as i32, h as i32);
    let mut img = RgbaImage::from_fn(w, h, |x, y| if x == 0 || y == 0 || x == w - 1 || y == h - 1 { color } else { [0; 4] });
    let mut put = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && x < wi && y < hi {
            img.put(x as u32, y as u32, color);
        }
    };
    match panes_divider(split, wi, hi) {
        Some((true, x)) => {
            let (from, to) = (1 + (hi - 2) / 3, 1 + 2 * (hi - 2) / 3);
            (1..hi - 1).filter(|&y| !free || y < from || y >= to).for_each(|y| put(x, y));
        }
        Some((false, y)) => {
            let (from, to) = (1 + (wi - 2) / 3, 1 + 2 * (wi - 2) / 3);
            (1..wi - 1).filter(|&x| !free || x < from || x >= to).for_each(|x| put(x, y));
        }
        None => {}
    }
    if free {
        // Arrowheads 4 pixels long, their base 8 pixels from the middle,
        // where the pane has room for them.
        const BASE: i32 = 8;
        const LONG: i32 = 4;
        for [px, py, pw, ph] in panes_rects(split, (w, h)) {
            let (cx, cy) = (px + pw / 2, py + ph / 2);
            for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                let room = if dx != 0 { pw / 2 } else { ph / 2 };
                if room < BASE + LONG + 1 {
                    continue;
                }
                for k in 0..LONG {
                    // Row k from the base: half as wide as the head is long, less k.
                    let along = BASE + k;
                    for across in -(LONG - 1 - k)..=(LONG - 1 - k) {
                        let (x, y) = if dx != 0 { (cx + dx * along, cy + across) } else { (cx + across, cy + dy * along) };
                        put(x, y);
                    }
                }
            }
        }
    }
    Ok(img)
}

/// `source = "pointer:R,G,B,A:WxH"`: a mouse pointer (an arrow head
/// pointing up and left, a tail below it), outlined like `arrow:`, the
/// outline as translucent as the colour; its tip at pixel (1, 1).
fn pointer(spec: &str) -> Result<RgbaImage, String> {
    let (color, (w, h)) = shape_spec(spec, "pointer")?;
    let (iw, ih) = (w as f64 - 2.0, h as f64 - 2.0);
    // The head's left edge is upright, its right edge comes down at 45°
    // (for a pointer as wide as 2/3 of its height), its foot rises to the
    // right; the tail leaves the foot down and to the right.
    let head = (ih * 0.72).round();
    let inside = |x: i32, y: i32| {
        let (x, y) = (x as f64 - 1.0, y as f64 - 1.0);
        if x < 0.0 || y < 0.0 || x >= iw || y >= ih {
            return false;
        }
        let in_head = x <= y * iw / head && y <= head - x * head / (4.0 * iw);
        let tail_from = head * 0.6;
        let middle = iw * 0.3 + (y - tail_from) * 0.45;
        let in_tail = y >= tail_from && (x - middle).abs() <= (iw * 0.14).max(1.0);
        in_head || in_tail
    };
    let outline = [0, 0, 0, color[3]];
    Ok(RgbaImage::from_fn(w, h, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if inside(x, y) {
            color
        } else if [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| inside(x + dx, y + dy)) {
            outline
        } else {
            [0; 4]
        }
    }))
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

/// `image` resampled to `w × h`, nearest pixel (each pixel the source
/// pixel under its middle).
fn resize_nearest(image: &RgbaImage, (w, h): (u32, u32)) -> RgbaImage {
    let (sw, sh) = (image.width as u64, image.height as u64);
    RgbaImage::from_fn(w, h, |x, y| image.get(((2 * x as u64 + 1) * sw / (2 * w as u64)) as u32, ((2 * y as u64 + 1) * sh / (2 * h as u64)) as u32))
}

/// The native UI scaler advances exact16.16 source coordinates. Keeping
/// those increments avoids changing its phase after fractional dimension
/// truncation or clipping.
fn resize_fixed(image: &RgbaImage, (w, h): (u32, u32), [sx, sy]: [u32; 2]) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| image.get(
        ((u64::from(x) * u64::from(sx)) >> 16) as u32,
        ((u64::from(y) * u64::from(sy)) >> 16) as u32))
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

/// `source = "rects:WxH;R,G,B,A:X,Y,W,H;..."`: a clear `W × H` image with
/// rectangles filled in order (later ones over earlier ones), each given as
/// its colour and its top-left and size; `R,G,B,A:X,Y,W,H*N,DX,DY` fills it
/// `N` times, each copy moved by `(DX, DY)` from the one before (a column
/// of bars, say). Parts outside the image are left out. For pictures made
/// of a few rectangles (bars, frames, lines) that a driver describes.
fn rects(spec: &str) -> Result<RgbaImage, String> {
    let err = |what: &str| format!("rects image: {what} (expected WxH;R,G,B,A:X,Y,W,H[*N,DX,DY];...)");
    let mut items = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let size = items.next().ok_or_else(|| err("no size"))?;
    let (w, h) = size.split_once('x').ok_or_else(|| err(size))?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| err(size))?, h.parse().map_err(|_| err(size))?);
    let mut img = RgbaImage::from_fn(w, h, |_, _| [0; 4]);
    let numbers = |s: &str, n: usize| -> Result<Vec<i32>, String> {
        let v: Vec<i32> = s.split(',').map(|v| v.trim().parse().map_err(|_| err(s))).collect::<Result<_, _>>()?;
        if v.len() == n { Ok(v) } else { Err(err(s)) }
    };
    for item in items {
        let (color, rest) = item.split_once(':').ok_or_else(|| err(item))?;
        let c = numbers(color, 4)?;
        let color = [0, 1, 2, 3].map(|i| c[i].clamp(0, 255) as u8);
        let (rect, repeat) = match rest.split_once('*') {
            Some((rect, repeat)) => (rect, numbers(repeat, 3)?),
            None => (rest, vec![1, 0, 0]),
        };
        let r = numbers(rect, 4)?;
        for k in 0..repeat[0].max(0) {
            let (x0, y0) = (r[0] + k * repeat[1], r[1] + k * repeat[2]);
            for y in y0.max(0)..(y0 + r[3]).min(h as i32) {
                for x in x0.max(0)..(x0 + r[2]).min(w as i32) {
                    img.put(x as u32, y as u32, color);
                }
            }
        }
    }
    Ok(img)
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
        desc.expand_variants()?;
        Ok(desc)
    }

    /// Expands the variants of images, sheets and palettes (see
    /// [`Description::parse`]).
    fn expand_variants(&mut self) -> Result<(), DescError> {
        let desc = self;
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
        let mut palettes = BTreeMap::new();
        for (name, mut p) in std::mem::take(&mut desc.palettes) {
            match p.options.remove("variants") {
                None => {
                    palettes.insert(name, p);
                }
                Some(values) => {
                    let values: Vec<i64> = values.try_into().map_err(|e: toml::de::Error| DescError::Toml(e))?;
                    for v in values {
                        let copy = p.copy.iter().map(|(n, a, b, c)| (substitute(n, v), *a, *b, *c)).collect();
                        let options = p.options.iter().map(|(k, o)| (k.clone(), substitute_value(o, v))).collect();
                        let (source, base) = (p.source.as_deref().map(|s| substitute(s, v)), p.base.as_deref().map(|s| substitute(s, v)));
                        palettes.insert(format!("{name}@{v}"), PaletteDesc { source, base, copy, options });
                    }
                }
            }
        }
        desc.palettes = palettes;
        Ok(())
    }

    /// The frames of image `name`; with `blank`, as the load that finds
    /// boxes makes them (`blank_loader`, then the image's own `blank`).
    fn load_frames(&self, loader: &dyn ImageLoader, name: &str, img: &ImageDesc, blank: bool) -> Result<Vec<RgbaImage>, DescError> {
        let options = self.image_options(img, blank);
        let generators: [(&str, fn(&str) -> Result<RgbaImage, String>); 8] = [
            ("solid:", solid),
            ("line:", line),
            ("arrow:", arrow),
            ("crosshair:", crosshair),
            ("panes:", panes),
            ("pointer:", pointer),
            ("rects:", rects),
            ("glyphs:", glyphs),
        ];
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
        let generated = GENERATED.iter().any(|p| img.source.starts_with(p));
        if generated {
            return Ok(self.load_frames(loader, name, img, false)?.iter().map(|f| (f.width, f.height)).collect());
        }
        let mut options = self.loader.clone();
        options.extend(img.options.clone());
        options.remove("blank");
        loader.sizes(&img.source, &options).map_err(|message| DescError::Image { name: name.to_owned(), message })
    }

    /// Loads the images, slices the sheets and builds the tile renderer's
    /// tables. `max_page` is the largest atlas page the renderer can take
    /// (its device's `max_texture_dimension_2d`); the pages are as big as
    /// [`page_size`] makes them for the description's largest sprite.
    pub fn compile(&self, loader: &dyn ImageLoader, max_page: u32) -> Result<Compiled, DescError> {
        self.compile_with(loader, max_page, true, None)
    }

    /// The side of the largest sprite the sheets cut (outlines included),
    /// from the description alone.
    pub fn largest_sprite(&self) -> u32 {
        let grow = |s: &SheetDesc| 2 * s.outline.as_ref().map_or(0, |o| o.width);
        self.sheets.values().map(|s| (s.cell[0] + grow(s)).max(s.cell[1] + grow(s))).max().unwrap_or(0).max(self.largest)
    }

    /// Like [`Description::compile`], reusing the sprites of every sheet
    /// `cache` has from an earlier compile (the same sheet of the same image,
    /// loaded the same way): only new sheets load their images. The cache
    /// keeps this compile's sheets afterwards, no others.
    pub fn compile_cached(&self, loader: &dyn ImageLoader, max_page: u32, cache: &mut SheetCache) -> Result<Compiled, DescError> {
        self.compile_with(loader, max_page, true, Some(cache))
    }

    /// The tables [`Description::compile`] makes without the pixels: sprite
    /// names, sizes and anchors, sets and layers, as
    /// [`Compiled::without_pixels`] keeps them (the sprites' places in the
    /// atlas are not decided). Only the images' sizes are needed
    /// ([`ImageLoader::sizes`]), not their pixels: for a side that draws
    /// nothing itself.
    pub fn compile_layout(&self, loader: &dyn ImageLoader, max_page: u32) -> Result<Compiled, DescError> {
        self.compile_with(loader, max_page, false, None)
    }

    /// What `sheet`'s sprites are made from, beyond `self.loader` and
    /// `self.blank_loader` (`globals`, the same for all sheets): its image
    /// and how it is cut, not what the sheet, its image or its sprites are
    /// called (names are given in the order things were added, and move as
    /// art is added). For [`SheetCache`]. `identity`: what the image stands for
    /// now (see [`ImageLoader::identity`]).
    fn sheet_key(&self, sheet: &SheetDesc, globals: &str, identity: &str) -> String {
        let shape = SheetDesc { image: String::new(), group: None, first_index: 0, index_stride: 1, ..sheet.clone() };
        format!("{shape:?}|{:?}|{identity}|{globals}", self.images.get(&sheet.image))
    }

    /// The sprites of sheet `name` (`pixels`: with their pixels, else their
    /// sizes only), from the frames of its image (`frames` loads an image's
    /// frames, `blank` them as the load that finds boxes makes them, `sizes`
    /// gives their sizes).
    #[allow(clippy::too_many_arguments)]
    fn slice(
        &self,
        name: &str,
        sheet: &SheetDesc,
        pixels: bool,
        frames: &mut dyn FnMut(&str, bool) -> Result<Frames, DescError>,
        sizes: &mut dyn FnMut(&str) -> Result<std::rc::Rc<Vec<(u32, u32)>>, DescError>,
        shared: &mut SharedTexels,
        source_id: &str,
    ) -> Result<Sliced, DescError> {
        // (The image is read for its anchor marks even without pixels: the
        // anchors are the same either way.)
        let all = if pixels || sheet.anchor_marks.is_some() { Some(frames(&sheet.image, false)?) } else { None };
        // The images again as the sprite format sees them, for the sheets
        // that sample from their boxes (the boxes are found once, here).
        let boxed = sheet.sample_from == SampleFrom::Box;
        let all_blank = if boxed && pixels { Some(frames(&sheet.image, true)?) } else { None };
        // (`all_frames`: every frame of the image, one after the other.)
        let count = if !sheet.all_frames { 1 } else if let Some(all) = &all { all.len() } else { sizes(&sheet.image)?.len() };
        let mut out = Sliced { sprites: Vec::new(), cells: 0 };
        for k in 0..count {
            let frame = if sheet.all_frames { k } else { sheet.frame };
            let one = self.slice_frame(name, sheet, frame, (all.as_ref(), pixels), all_blank.as_ref(), sizes, shared, source_id)?;
            out.sprites.extend(one.sprites.into_iter().map(|s| SlicedSprite { index: s.index + out.cells, ..s }));
            out.cells += one.cells;
        }
        Ok(out)
    }

    /// [`Description::slice`] for frame `frame` of the image (`all`,
    /// `all_blank`: its frames, if made).
    #[allow(clippy::too_many_arguments)]
    fn slice_frame(
        &self,
        name: &str,
        sheet: &SheetDesc,
        frame: usize,
        (all, pixels): (Option<&Frames>, bool),
        all_blank: Option<&Frames>,
        sizes: &mut dyn FnMut(&str) -> Result<std::rc::Rc<Vec<(u32, u32)>>, DescError>,
        shared: &mut SharedTexels,
        source: &str,
    ) -> Result<Sliced, DescError> {
        let err = |message: String| DescError::Sheet { sheet: name.to_owned(), message };
        let no_frame = || err(format!("image has no frame {frame}"));
        if let Some(f) = all.filter(|f| frame >= f.len()) {
            let _ = f;
            return Err(no_frame());
        }
        let (iw, ih) = match all {
            Some(f) => f.size(frame),
            None => *sizes(&sheet.image)?.get(frame).ok_or_else(no_frame)?,
        };
        if let Some(blank) = all_blank {
            if frame >= blank.len() {
                return Err(no_frame());
            }
            let (bw, bh) = blank.size(frame);
            if (bw, bh) != (iw, ih) {
                return Err(err(format!("the image loads as {bw}×{bh} for its box but as {iw}×{ih} to draw")));
            }
        }
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
        if matches!(all, Some(Frames::Indexed { .. })) && sheet.outline.is_some() {
            return Err(err("an indexed image cannot be outlined".into()));
        }
        if matches!(all, Some(Frames::Indexed { .. })) && sheet.resize.is_some() {
            return Err(err("an indexed image cannot be resized (indexed = false)".into()));
        }
        if sheet.resize.is_some_and(|[w, h]| w == 0 || h == 0) {
            return Err(err("resized to nothing".into()));
        }
        let grow = sheet.outline.as_ref().map_or(0, |o| o.width);
        if let Some([sx, sy]) = sheet.resize_step {
            let Some([w, h]) = sheet.resize else { return Err(err("resize_step requires resize".into())) };
            if sx == 0 || sy == 0 || ((u64::from(w - 1) * u64::from(sx)) >> 16) >= u64::from(cw + 2 * grow)
                || ((u64::from(h - 1) * u64::from(sy)) >> 16) >= u64::from(ch + 2 * grow) {
                return Err(err("resize_step samples outside its source cell".into()));
            }
        }
        let (gw, gh) = match sheet.resize {
            Some([w, h]) => (w, h),
            None => (cw + 2 * grow, ch + 2 * grow),
        };
        let alpha_mask = sheet.alpha_mask();
        let shape = |cell: RgbaImage| {
            let cell = match alpha_mask {
                Some(mask) => mask.rgba(cell),
                None => cell,
            };
            let cell = match &sheet.outline {
                Some(o) => outline(&cell, o.width, o.color),
                None => cell,
            };
            match sheet.resize {
                Some([w, h]) => match sheet.resize_step {
                    Some(step) => resize_fixed(&cell, (w, h), step),
                    None => resize_nearest(&cell, (w, h)),
                },
                None => cell,
            }
        };
        // A cell's pixels and what a view showing the world smaller samples
        // of it: its box (empty if it is all blank) or all of it.
        let mut cut = |x: u32, y: u32| -> Result<(Option<Cut>, [u32; 4]), DescError> {
            let image = all.filter(|_| pixels).map(|f| match f {
                Frames::Rgba(frames) => Ok(Cut::Rgba(shape(frames[frame].crop(x, y, cw, ch)))),
                Frames::Indexed { frames, rows } => {
                    let key = (source.to_owned(), frame, [x, y, cw, ch], None);
                    let image = shared.entry(key.clone()).or_insert_with(|| std::sync::Arc::new(frames[frame].pixels.crop(x, y, cw, ch))).clone();
                    let row = rows[frame].clone();
                    match alpha_mask {
                        Some(mask) => mask.indexed(image, row, key, self.remap(&sheet.image), shared).map_err(&err),
                        None => Ok(Cut::Indexed { image, row, key }),
                    }
                }
            }).transpose()?;
            let sample_box = match all_blank {
                Some(Frames::Rgba(blank)) => opaque_box(&shape(blank[frame].crop(x, y, cw, ch))).unwrap_or_default(),
                Some(Frames::Indexed { frames, rows }) => {
                    let f = &frames[frame];
                    let row = &rows[frame];
                    let cell = RgbaImage::from_fn(cw, ch, |i, j| [0, 0, 0, (row[f.pixels.get(x + i, y + j) as usize] >> 24) as u8]);
                    opaque_box(&cell).unwrap_or_default()
                }
                None => [0, 0, gw, gh],
            };
            Ok((image, sample_box))
        };
        let mut sprites = Vec::new();
        for i in 0..cols * rows {
            if sheet.only.as_ref().is_some_and(|o| !o.contains(&i)) {
                continue;
            }
            let (c, r) = (i % cols, i / cols);
            let at = |[ox, oy]: [u32; 2]| (ox + c * (cw + sx), oy + r * (ch + sy));
            let (x, y) = at(sheet.origin);
            let (image, sample_box) = cut(x, y)?;
            let scaled = sheet.scaled_origin.map(|o| {
                let (x, y) = at(o);
                cut(x, y)
            }).transpose()?;
            let marked = match (sheet.anchor_marks, all) {
                (Some(mark), Some(f)) => anchor_marked(f, frame, (iw, ih), (x, y), (cw, ch), mark),
                _ => None,
            };
            let [ax, ay] = marked.unwrap_or(sheet.anchor);
            let anchor = (ax - grow as i32, ay - grow as i32);
            sprites.push(SlicedSprite { index: i, image, size: (gw, gh), anchor, sample_box, sample_origin: [x, y], scaled });
        }
        Ok(Sliced { sprites, cells: cols * rows })
    }

    /// The remapped range of image `name`'s indexed sprites (`remap = [lo,
    /// hi]`, default none; see `tile_render_shared::SpriteDesc::remap`).
    fn remap(&self, name: &str) -> u32 {
        let r = self.images.get(name).and_then(|i| i.options.get("remap")).and_then(toml::Value::as_array);
        let n = |i: usize| r.and_then(|r| r.get(i)).and_then(toml::Value::as_integer).unwrap_or(0).clamp(0, 256) as u32;
        tile_render_shared::remap_range(n(0), n(1))
    }

    /// The frames of image `name` (`blank`: as the load that finds boxes
    /// makes them): indexed where the loader can make them so (see
    /// [`ImageLoader::indexed`]) and the image does not say `indexed =
    /// false`, else colours.
    fn load_any(&self, loader: &dyn ImageLoader, name: &str, img: &ImageDesc, blank: bool) -> Result<Frames, DescError> {
        let generated = GENERATED.iter().any(|p| img.source.starts_with(p));
        let wanted = img.options.get("indexed").and_then(toml::Value::as_bool).unwrap_or(true);
        if !generated && wanted {
            let options = self.image_options(img, blank);
            let image_error = |message: String| DescError::Image { name: name.to_owned(), message };
            if let Some(frames) = loader.indexed(&img.source, &options).map_err(image_error)? {
                let mut rows: Vec<std::sync::Arc<Vec<u32>>> = Vec::with_capacity(frames.len());
                for f in &frames {
                    let mut row: Vec<u32> = f.palette.iter().map(|&c| tile_render::atlas::palette_entry(c)).collect();
                    row.resize(tile_render_shared::PALETTE_SIZE as usize, 0);
                    // (Frames of one palette share its row.)
                    match rows.last().filter(|r| ***r == row) {
                        Some(r) => rows.push(r.clone()),
                        None => rows.push(std::sync::Arc::new(row)),
                    }
                }
                return Ok(Frames::Indexed { frames: std::rc::Rc::new(frames), rows });
            }
        }
        Ok(Frames::Rgba(std::rc::Rc::new(self.load_frames(loader, name, img, blank)?)))
    }

    /// The loader options of `img` (`blank`: for the load that finds boxes).
    fn image_options(&self, img: &ImageDesc, blank: bool) -> toml::Table {
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
        options
    }

    fn compile_with(&self, loader: &dyn ImageLoader, max_page: u32, pixels: bool, cache: Option<&mut SheetCache>) -> Result<Compiled, DescError> {
        self.compile_full(loader, page_size(self.largest_sprite(), max_page), pixels, cache, None)
    }

    /// A whole compile ([`Description::compile_with`]); with `growing`, the
    /// state to grow it from is kept there (where the next sprites go, the
    /// sheets compiled, shared pixels and palette rows).
    fn compile_full(
        &self,
        loader: &dyn ImageLoader,
        page_size: u32,
        pixels: bool,
        cache: Option<&mut SheetCache>,
        growing: Option<&mut Growing>,
    ) -> Result<Compiled, DescError> {
        let mut placer = Placer::new(page_size, pixels);
        let mut sprites: BTreeMap<String, SpriteId> = BTreeMap::new();
        let mut sheet_len: HashMap<String, u32> = HashMap::new();
        let mut seen = growing.as_ref().map(|_| HashSet::new());
        self.add_sheets(loader, &mut placer, None, &mut sprites, &mut sheet_len, cache, seen.as_mut())?;
        // The palette table: row 0, the sprites' own rows, the named ones.
        let (named, palette_rows) = self.palette_rows(loader)?;
        let Placer { target, own, texels, rows, unavailable, .. } = placer;
        let shelves = match &target {
            Target::Atlas(a) => (a.shelves().0.clone(), a.shelves().1.clone()),
            _ => (Shelves::new(page_size), Shelves::new(page_size)),
        };
        let mut atlas = match target {
            Target::Atlas(a) => a.finish(),
            Target::Grow { .. } => unreachable!("a whole compile does not grow"),
            Target::Sizes(placed) => tile_render::Atlas { page_size, pages: Vec::new(), index_pages: Vec::new(), sprites: placed, palettes: Vec::new(), named_palettes: 1 },
        };
        atlas.named_palettes = (own.len() / tile_render_shared::PALETTE_SIZE as usize) as u32;
        atlas.palettes = own;
        atlas.palettes.extend_from_slice(&named[tile_render_shared::PALETTE_SIZE as usize..]);
        // What of each sprite draws nothing: left out of its drawn rectangle.
        atlas.trim_blank();
        if let Some(g) = growing {
            g.shelves = shelves;
            g.texels = texels;
            g.rows = rows;
            g.seen = seen.unwrap_or_default();
            g.sheet_len = sheet_len.clone();
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

        let fonts = self.load_fonts(loader, |_| false);

        // Layers and their value tables.
        let mut layers = Vec::new();
        let mut values = HashMap::new();
        for layer in &self.layers {
            let mut names = Vec::new();
            for pattern in &layer.sets {
                if pattern.ends_with('*') {
                    names.extend(matching(pattern, set_ids.keys(), &sheet_len));
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
            spec.ticks_per_frame = layer.ticks_per_frame;
            spec.per_cell_phase = layer.per_cell_phase;
            spec.per_cell_offset = layer.per_cell_offset;
            spec.descending = layer.descending;
            spec.hidden = layer.hidden;
            spec.pass = layer.pass;
            spec.cull_offset = (layer.cull_offset[0], layer.cull_offset[1]);
            spec.blend = blend_of(layer.blend)
                .ok_or_else(|| DescError::Unknown { kind: "blend (scale denominator 1..=32) of layer", name: layer.name.clone() })?;
            // (Enough bits for every row, and row 0.)
            spec.category = layer.category;
            spec.palette_bits = if layer.palettes { (u32::BITS - (palette_rows.len() as u32).leading_zeros()).max(1) } else { 0 };
            if layers.last().is_some_and(|l: &LayerSpec| l.pass > spec.pass) {
                return Err(DescError::Unknown { kind: "pass order for layer", name: layer.name.clone() });
            }
            if layers.last().is_some_and(|l: &LayerSpec| l.pass == spec.pass && l.descending != spec.descending) {
                return Err(DescError::Unknown { kind: "column order (all layers of a pass must agree) for layer", name: layer.name.clone() });
            }
            layers.push(spec);
        }

        let compiled = Compiled {
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
            palette_rows,
            sprite_ids: sprites.into_iter().collect(),
            unavailable,
        };
        Ok(compiled)
    }

    /// The font faces that load (logged if not), but those `known` says
    /// are there already.
    fn load_fonts(&self, loader: &dyn ImageLoader, known: impl Fn(&str) -> bool) -> Vec<(String, std::sync::Arc<tile_render::gpu_text::Face>)> {
        let mut fonts = Vec::new();
        for (name, font) in self.fonts.iter().filter(|(n, _)| !known(n)) {
            let face = loader.font(&font.source).and_then(|data| tile_render::gpu_text::Face::parse(&data, font.index).map_err(|e| e.to_string()));
            match face {
                Ok(face) => fonts.push((name.clone(), std::sync::Arc::new(face))),
                Err(e) => log::warn!("font {name:?} ({}): {e}", font.source),
            }
        }
        if !known(FALLBACK_FONT) {
            fonts.push((FALLBACK_FONT.into(), std::sync::Arc::new(fallback_font::face())));
        }
        fonts
    }

    /// Slices the sheets and places their sprites with `placer`, naming
    /// them in `sprites` (the cells drawn instead in views showing the
    /// world smaller after all of them, unnamed). `known`: the art this
    /// compile is appended to (its sprites are kept: a sheet whose sprites
    /// it has adds nothing, a sheet may cut parts of its sprites); `seen`:
    /// the keys of the sheets compiled before (skipped), this compile's
    /// added; `cache`: see [`SheetCache`].
    #[allow(clippy::too_many_arguments)]
    fn add_sheets(
        &self,
        loader: &dyn ImageLoader,
        placer: &mut Placer,
        known: Option<&Compiled>,
        sprites: &mut BTreeMap<String, SpriteId>,
        sheet_len: &mut HashMap<String, u32>,
        mut cache: Option<&mut SheetCache>,
        mut seen: Option<&mut HashSet<u64>>,
    ) -> Result<(), DescError> {
        use std::rc::Rc;
        let pixels = placer.pixels;
        // Images are loaded when a sheet needs them, once per compile.
        let mut loaded: HashMap<(String, bool), Frames> = HashMap::new();
        let mut frames = |name: &str, blank: bool| -> Result<Frames, DescError> {
            if let Some(f) = loaded.get(&(name.to_owned(), blank)) {
                return Ok(f.clone());
            }
            let img = self.images.get(name).ok_or(DescError::Unknown { kind: "image", name: name.to_owned() })?;
            let f = self.load_any(loader, name, img, blank)?;
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
        // What each image stands for now: with its options (the sheet's
        // key), and its source alone (its indices).
        let mut identities: HashMap<(String, bool), String> = HashMap::new();
        let mut identity = |name: &str, with_options: bool| -> String {
            identities
                .entry((name.to_owned(), with_options))
                .or_insert_with(|| match self.images.get(name) {
                    Some(img) if !GENERATED.iter().any(|p| img.source.starts_with(p)) => {
                        let options = if with_options { self.image_options(img, false) } else { toml::Table::new() };
                        loader.identity(&img.source, &options)
                    }
                    Some(img) => img.source.clone(),
                    None => String::new(),
                })
                .clone()
        };
        let globals = format!("{:?}|{:?}", self.loader, self.blank_loader);
        let mut kept: HashMap<String, std::sync::Arc<Sliced>> = HashMap::new();
        // The indices of the sheets kept from earlier compiles, for new sheets
        // of the same pixels to share.
        let mut shared: SharedTexels = HashMap::new();
        for sliced in cache.as_ref().and_then(|c| c.sheets.as_ref()).into_iter().flat_map(|s| s.values()) {
            for s in &sliced.sprites {
                for cut in [s.image.as_ref(), s.scaled.as_ref().and_then(|c| c.0.as_ref())].into_iter().flatten() {
                    if let Cut::Indexed { image, key, .. } = cut {
                        shared.entry(key.clone()).or_insert_with(|| image.clone());
                    }
                }
            }
        }
        // Sprites with a twin for views showing the world smaller
        // (`scaled_origin`): which, and where their twin's cell is.
        let mut twins: Vec<(SpriteId, std::sync::Arc<Sliced>, usize, u32, u32)> = Vec::new();
        let existing = |name: &str| known.and_then(|k| k.sprite(name));
        // Sheets of images first, then those of parts of sprites (which may
        // be those of the first).
        let ordered = self.sheets.iter().filter(|(_, s)| s.sprite.is_none()).chain(self.sheets.iter().filter(|(_, s)| s.sprite.is_some()));
        for (sheet_name, sheet) in ordered {
            sheet.validate_alpha_mask().map_err(|message| DescError::Sheet { sheet: sheet_name.clone(), message })?;
            if sheet.sampling == Sampling::UiBackground && (sheet.resize.is_some() || sheet.outline.is_some()) {
                return Err(DescError::Sheet { sheet: sheet_name.clone(), message: "UI background sampling requires unresized source art without an outline".into() });
            }
            let sprite_name = |index: u32| match &sheet.group {
                Some(g) => format!("{g}/{}", sheet.first_index + index * sheet.index_stride),
                None => format!("{sheet_name}/{index}"),
            };
            let group = sheet.group.clone().unwrap_or_else(|| sheet_name.clone());
            let key = self.sheet_key(sheet, &globals, &identity(&sheet.image, true));
            // (What makes the sheet and what its sprites are called.)
            let seen_key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&key, &sheet.sprite, &sheet.group, sheet.first_index, sheet.index_stride).hash(&mut h);
                if sheet.group.is_none() {
                    sheet_name.hash(&mut h);
                }
                h.finish()
            };
            if seen.as_deref().is_some_and(|s| s.contains(&seen_key)) {
                continue;
            }
            if let Some(s) = seen.as_deref_mut() {
                s.insert(seen_key);
            }
            let scaling = match sheet.scaling {
                SheetScaling::Scale => tile_render_shared::SHRINK,
                SheetScaling::ScaleKeepAnchor => tile_render_shared::SHRINK_KEEP_ANCHOR,
                SheetScaling::Keep => tile_render_shared::KEEP_SIZE,
            };
            // Parts of a sprite compiled before.
            if let Some(of) = &sheet.sprite {
                let err = |message: String| DescError::Sheet { sheet: sheet_name.clone(), message };
                let base_id = sprites.get(of).copied().or_else(|| existing(of)).ok_or_else(|| DescError::Unknown { kind: "sprite", name: of.clone() })?;
                let base = placer.sprite(base_id, known).ok_or_else(|| DescError::Unknown { kind: "sprite", name: of.clone() })?;
                if sheet.scaled_origin.is_some() || sheet.outline.is_some() || sheet.sample_from == SampleFrom::Box || !sheet.image.is_empty() || sheet.all_frames {
                    return Err(err("a sheet of a sprite's parts has no image, frames, scaled origin, outline or box sampling".into()));
                }
                let [cw, ch] = sheet.cell;
                let [ox, oy] = sheet.origin;
                let [sx, sy] = sheet.spacing;
                let fit = |total: u32, o: u32, c: u32, s: u32| if total < o + c { 0 } else { (total - o + s) / (c + s) };
                let [cols, rows] = sheet.grid.unwrap_or_else(|| [fit(base.width, ox, cw, sx), fit(base.height, oy, ch, sy)]);
                if cw == 0 || ch == 0 || cols == 0 || rows == 0 || ox + cols * (cw + sx) - sx > base.width || oy + rows * (ch + sy) - sy > base.height {
                    return Err(err(format!("{cols}×{rows} cells of {cw}×{ch} from ({ox}, {oy}) do not fit sprite {of} ({}×{})", base.width, base.height)));
                }
                for i in (0..cols * rows).filter(|i| sheet.only.as_ref().is_none_or(|o| o.contains(i))) {
                    let name = sprite_name(i);
                    if existing(&name).is_some() {
                        continue;
                    }
                    let (c, r) = (i % cols, i / cols);
                    let (x, y) = (ox + c * (cw + sx), oy + r * (ch + sy));
                    let mut desc = tile_render::atlas::sprite_at((base.page, base.atlas_x + x, base.atlas_y + y), (cw, ch), (sheet.anchor[0], sheet.anchor[1]));
                    (desc.scaling, desc.palette, desc.remap) = (scaling, base.palette, base.remap);
                    desc.sampling = sheet.sampling.flag();
                    desc.sampling_origin_x = base.sampling_origin_x + x;
                    desc.sampling_origin_y = base.sampling_origin_y + y;
                    let id = placer.alias(desc);
                    if placer.unavailable(base_id, known) { placer.unavailable.insert(id.0); }
                    if sprites.insert(name.clone(), id).is_some() {
                        return Err(err(format!("sprite {name} defined twice")));
                    }
                }
                let len = sheet_len.entry(group).or_insert(0);
                *len = (*len).max(sheet.first_index + cols * rows * sheet.index_stride);
                continue;
            }
            let sliced = match cache.as_deref_mut() {
                Some(cache) => {
                    let sliced = match cache.sheets.as_ref().and_then(|c| c.get(&key)) {
                        Some(s) => s.clone(),
                        None => std::sync::Arc::new(self.slice(sheet_name, sheet, pixels, &mut frames, &mut sizes, &mut shared, &identity(&sheet.image, false))?),
                    };
                    kept.insert(key, sliced.clone());
                    sliced
                }
                None => std::sync::Arc::new(self.slice(sheet_name, sheet, pixels, &mut frames, &mut sizes, &mut shared, &identity(&sheet.image, false))?),
            };
            let remap = self.remap(&sheet.image);
            for (k, s) in sliced.sprites.iter().enumerate() {
                let name = sprite_name(s.index);
                // (Appended art: what is there already stays as it is.)
                if existing(&name).is_some() {
                    continue;
                }
                let id = placer.add(&name, s.image.as_ref(), s.size, s.anchor, scaling, s.sample_box, remap, known)?;
                let desc = placer.sprite_mut(id);
                desc.sampling = sheet.sampling.flag();
                desc.sampling_origin_x = s.sample_origin[0];
                desc.sampling_origin_y = s.sample_origin[1];
                if s.scaled.is_some() {
                    twins.push((id, sliced.clone(), k, scaling, remap));
                }
                if sprites.insert(name.clone(), id).is_some() {
                    return Err(DescError::Sheet { sheet: sheet_name.clone(), message: format!("sprite {name} defined twice") });
                }
            }
            let len = sheet_len.entry(group).or_insert(0);
            *len = (*len).max(sheet.first_index + sliced.cells * sheet.index_stride);
        }
        // The cells drawn instead in views showing the world smaller, after
        // all named sprites (so that a compile without the pixels numbers
        // those the same).
        for (id, sliced, k, scaling, remap) in twins {
            let s = &sliced.sprites[k];
            let Some((image, sample_box)) = &s.scaled else { continue };
            let twin = placer.add("(a zoomed-out twin)", image.as_ref(), s.size, s.anchor, scaling, *sample_box, remap, known)?;
            placer.sprite_mut(id).downscaled = twin.0 + 1;
        }
        if let Some(cache) = cache {
            cache.sheets = Some(kept);
        }
        Ok(())
    }

    /// The palette table (row 0 clear, then the palettes in name order, see
    /// `tile_render::Atlas::palettes`) and each palette's row.
    fn palette_rows(&self, loader: &dyn ImageLoader) -> Result<(Vec<u32>, HashMap<String, u32>), DescError> {
        let size = tile_render_shared::PALETTE_SIZE as usize;
        let mut made: HashMap<String, Vec<[u8; 4]>> = HashMap::new();
        let mut table = tile_render::atlas::no_palettes();
        let mut rows = HashMap::new();
        for (i, name) in self.palettes.keys().enumerate() {
            let colors = self.palette_colors(loader, name, &mut made, 0, None)?;
            table.extend(colors.iter().take(size).map(|&c| tile_render::atlas::palette_entry(c)));
            rows.insert(name.clone(), i as u32 + 1);
        }
        Ok((table, rows))
    }

    /// The colours of palette `name` (one of this description's, or of
    /// `known`'s, the art an addition is appended to).
    fn palette_colors(&self, loader: &dyn ImageLoader, name: &str, made: &mut HashMap<String, Vec<[u8; 4]>>, depth: u32, known: Option<&Compiled>) -> Result<Vec<[u8; 4]>, DescError> {
        if let Some(p) = made.get(name) {
            return Ok(p.clone());
        }
        let err = |message: String| DescError::Image { name: format!("palette {name}"), message };
        let Some(p) = self.palettes.get(name) else {
            // A palette of the art appended to: its row's colours.
            let size = tile_render_shared::PALETTE_SIZE as usize;
            return match known.and_then(|k| k.palette(name).map(|r| (k, r))) {
                Some((k, r)) => {
                    let at = (k.atlas.named_palettes + r - 1) as usize * size;
                    Ok(k.atlas.palettes.get(at..at + size).ok_or_else(|| err("not in the table".into()))?.iter().map(|c| c.to_le_bytes()).collect())
                }
                None => Err(DescError::Unknown { kind: "palette", name: name.to_owned() }),
            };
        };
        if depth > 8 {
            return Err(err("palettes based on each other".into()));
        }
        let mut colors = match (&p.source, &p.base) {
            (Some(source), None) => {
                let mut options = self.loader.clone();
                options.extend(p.options.clone());
                loader.palette(source, &options).map_err(err)?
            }
            (None, Some(base)) => self.palette_colors(loader, base, made, depth + 1, known)?,
            _ => return Err(err("needs a source or a base (one of them)".into())),
        };
        colors.resize(tile_render_shared::PALETTE_SIZE as usize, [0; 4]);
        for (other, from, to, count) in &p.copy {
            let other = self.palette_colors(loader, other, made, depth + 1, known)?;
            for k in 0..*count as usize {
                if let (Some(&c), Some(slot)) = (other.get(*from as usize + k), colors.get_mut(*to as usize + k)) {
                    *slot = c;
                }
            }
        }
        made.insert(name.to_owned(), colors.clone());
        Ok(colors)
    }
}

/// The names among `names` that `pattern` (ending in `*`) matches, in a
/// layer's order: by sprite index for `"sheet/*"` (`sheet_len`: the sheets
/// and groups), else by name.
fn matching<'n>(pattern: &str, names: impl Iterator<Item = &'n String>, sheet_len: &HashMap<String, u32>) -> Vec<String> {
    let prefix = pattern.strip_suffix('*').unwrap_or(pattern);
    let mut matched: Vec<&String> = names.filter(|n| n.starts_with(prefix)).collect();
    matched.sort();
    // "sheet/*" in sprite-index order, not lexicographic.
    if let Some(sheet) = prefix.strip_suffix('/').filter(|s| sheet_len.contains_key(*s)) {
        matched.sort_by_key(|n| n[sheet.len() + 1..].parse::<u32>().unwrap_or(u32::MAX));
    }
    matched.into_iter().cloned().collect()
}

/// A compiled description that more art is appended to as it is needed:
/// what was compiled keeps its sprite ids, set ids, layer values, palette
/// rows and atlas places, the new art takes the next ones (see [`Growth`]).
/// Made by [`Description::compile_growing`]; [`Growing::grow`] compiles an
/// *addition*, a description of more images, sheets, sets, fonts and named
/// palettes (no grid, loader options or layers: those of the description
/// it is appended to; its palettes may be based on that one's). An addition's images and sheets are its
/// own (their names may be those of an earlier one's); sprites, sets and
/// fonts are shared: a sprite, set or font whose name is there already is
/// the one there (names are expected to say what art they are), a sheet
/// made the same way as an earlier one adds nothing, indexed sprites of the
/// same pixels as earlier ones share them, and new sets join the layers
/// whose patterns match them, at the end of their tables.
pub struct Growing {
    pixels: bool,
    page_size: u32,
    /// Where the next colour and indexed sprites go (with pixels).
    shelves: (Shelves, Shelves),
    /// The first sprite of each indexed sprite's pixels, and each
    /// colouring's own palette row.
    texels: HashMap<TexelKey, SpriteId>,
    rows: HashMap<std::sync::Arc<Vec<u32>>, u32>,
    grid: GridDesc,
    loader: toml::Table,
    blank_loader: toml::Table,
    /// Each layer's set patterns (those ending in `*`).
    patterns: Vec<Vec<String>>,
    /// The keys of the sheets compiled so far.
    seen: HashSet<u64>,
    /// The sheets and groups (`"sheet/*"` patterns).
    sheet_len: HashMap<String, u32>,
}

/// What appending an addition adds to a compiled description (see
/// [`Growing`]): sprites (and their pixels, compiled with them), palette
/// rows, sets, layer values, fonts. [`Compiled::apply`] adds it to the
/// description it was made for, or to a copy of it (e.g. one without the
/// pixels).
#[derive(Clone, Default)]
pub struct Growth {
    pub atlas: AtlasGrowth,
    /// The new sprites' names, in id order (`""`: a cell drawn instead in
    /// views showing the world smaller).
    pub sprite_names: Vec<String>,
    /// Explicit sets: name, frames, blend.
    sets: Vec<(String, Vec<SpriteId>, Blend)>,
    /// Per layer (index), the sets appended to its value table.
    layer_values: Vec<(usize, Vec<String>)>,
    fonts: Vec<(String, std::sync::Arc<tile_render::gpu_text::Face>)>,
    /// New named palettes and their rows.
    named: Vec<(String, u32)>,
    /// Global sprite ids whose art was replaced by transparent placeholders.
    unavailable: BTreeSet<u32>,
}

impl Growth {
    pub fn is_empty(&self) -> bool {
        self.atlas.sprites.is_empty() && self.sets.is_empty() && self.fonts.is_empty() && self.named.is_empty() && self.unavailable.is_empty()
    }
}

impl Description {
    /// [`Description::compile`] (`pixels`: with the atlas pixels, else
    /// [`Description::compile_layout`]), to be appended to (see [`Growing`]):
    /// its pages as [`page_size`] makes them for the largest sprite
    /// ([`Description::largest_sprite`], the art it may grow by too), at most
    /// `max_page`; a sprite grown later that is larger is left out.
    pub fn compile_growing(&self, loader: &dyn ImageLoader, max_page: u32, pixels: bool) -> Result<(Compiled, Growing), DescError> {
        let page_size = page_size(self.largest_sprite(), max_page);
        let mut growing = Growing {
            pixels,
            page_size,
            shelves: (Shelves::new(page_size), Shelves::new(page_size)),
            texels: HashMap::new(),
            rows: HashMap::new(),
            grid: self.grid.clone(),
            loader: self.loader.clone(),
            blank_loader: self.blank_loader.clone(),
            patterns: self.layers.iter().map(|l| l.sets.iter().filter(|p| p.ends_with('*')).cloned().collect()).collect(),
            seen: HashSet::new(),
            sheet_len: HashMap::new(),
        };
        let mut compiled = self.compile_full(loader, page_size, pixels, None, Some(&mut growing))?;
        // Values that pick a palette keep their meaning as the tables grow,
        // with room for palettes added later.
        for (layer, patterns) in compiled.layers.iter_mut().zip(&growing.patterns) {
            if layer.palette_bits > 0 && !patterns.is_empty() {
                layer.index_bits = GROWING_INDEX_BITS;
                layer.palette_bits = layer.palette_bits.max(GROWING_PALETTE_BITS);
            }
        }
        Ok((compiled, growing))
    }
}

/// Bits a growing layer's values that pick a palette index its table with
/// (see `tile_render::LayerSpec::index_bits`): room for this many sets.
const GROWING_INDEX_BITS: u32 = 16;
/// Bits a growing layer's values pick a palette row with: room for palettes
/// (and own rows, which the named ones count past) added later.
const GROWING_PALETTE_BITS: u32 = 10;

/// The parts of a description an addition has (see [`Growing`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Addition {
    #[serde(default)]
    images: BTreeMap<String, ImageDesc>,
    #[serde(default)]
    sheets: BTreeMap<String, SheetDesc>,
    #[serde(default)]
    sets: BTreeMap<String, SetDesc>,
    #[serde(default)]
    fonts: BTreeMap<String, FontDesc>,
    #[serde(default)]
    palettes: BTreeMap<String, PaletteDesc>,
}

impl Growing {
    /// Whether the art comes with its pixels.
    pub fn pixels(&self) -> bool {
        self.pixels
    }

    /// Pages the atlas uses (with pixels): colour pages, index pages.
    pub fn pages(&self) -> (u32, u32) {
        (self.shelves.0.pages, self.shelves.1.pages)
    }

    /// Compiles `addition` (TOML) as appended to `compiled` (this one's
    /// description, as grown so far: apply each growth to it before the
    /// next). Nothing is added on error.
    pub fn grow(&mut self, compiled: &Compiled, addition: &str, loader: &dyn ImageLoader) -> Result<Growth, DescError> {
        let a: Addition = toml::from_str(addition)?;
        // As a description of its own, with the grid and loader options of
        // the one it is appended to (variants expanded the same way).
        let mut desc = Description {
            grid: self.grid.clone(),
            loader: self.loader.clone(),
            blank_loader: self.blank_loader.clone(),
            images: a.images,
            sheets: a.sheets,
            sets: a.sets,
            layers: Vec::new(),
            fonts: a.fonts,
            palettes: a.palettes,
            largest: 0,
        };
        desc.expand_variants()?;
        let first = compiled.atlas.sprites.len() as u32;
        let rows = (compiled.atlas.palettes.len() / tile_render_shared::PALETTE_SIZE as usize).max(1) as u32;
        let mut placer = Placer::growing(self, first, rows);
        let mut sprites: BTreeMap<String, SpriteId> = BTreeMap::new();
        let mut sheet_len = self.sheet_len.clone();
        let mut seen = self.seen.clone();
        desc.add_sheets(loader, &mut placer, Some(compiled), &mut sprites, &mut sheet_len, None, Some(&mut seen))?;
        let mut sprite_names = vec![String::new(); placer.placed().len()];
        for (name, id) in &sprites {
            sprite_names[(id.0 - first) as usize] = name.clone();
        }
        let sprite = |name: &str| sprites.get(name).copied().or_else(|| compiled.sprite(name));
        let mut sets = Vec::new();
        for (name, set) in &desc.sets {
            let blend = blend_of(set.blend).ok_or_else(|| DescError::Unknown { kind: "blend of set", name: name.clone() })?;
            let frames = set.frames.iter().map(|f| sprite(f).ok_or(DescError::Unknown { kind: "sprite", name: f.clone() })).collect::<Result<Vec<_>, _>>()?;
            // A set there already stays as it is, unless it has other frames
            // now (more of an animation made): then it is replaced.
            if let Some(&id) = compiled.set_ids.get(name) {
                let had: Vec<SpriteId> = (0..).map_while(|i| compiled.sets.frame(id, i)).collect();
                if had == frames {
                    continue;
                }
            }
            sets.push((name.clone(), frames, blend));
        }
        // The new sets on the layers whose patterns match them.
        let new_sets: Vec<String> = sprites
            .keys()
            .chain(sets.iter().map(|(n, _, _)| n))
            .filter(|n| !compiled.set_ids.contains_key(*n))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut layer_values = Vec::new();
        for (i, patterns) in self.patterns.iter().enumerate() {
            let Some(layer) = compiled.layers.get(i) else { continue };
            let mut names: Vec<String> = Vec::new();
            for p in patterns {
                for n in matching(p, new_sets.iter(), &sheet_len) {
                    if !names.contains(&n) && !compiled.values.contains_key(&(layer.name.clone(), n.clone())) {
                        names.push(n);
                    }
                }
            }
            if !names.is_empty() {
                layer_values.push((i, names));
            }
        }
        let fonts = desc.load_fonts(loader, |n| compiled.font(n).is_some());
        let Placer { target, mut own, texels, rows: colourings, unavailable, .. } = placer;
        // New named palettes after the new own rows (a named palette's row
        // counts from the first, wherever it is in the table).
        let size = tile_render_shared::PALETTE_SIZE as usize;
        let mut named = Vec::new();
        let mut made = HashMap::new();
        for name in desc.palettes.keys().filter(|n| compiled.palette(n).is_none()) {
            let colors = desc.palette_colors(loader, name, &mut made, 0, Some(compiled))?;
            let row = rows + (own.len() / size) as u32;
            own.extend(colors.iter().take(size).map(|&c| tile_render::atlas::palette_entry(c)));
            named.push((name.clone(), row + 1 - compiled.atlas.named_palettes.max(1)));
        }
        let (shelves, sprites, blits) = match target {
            Target::Grow { rgba, index, blits, sprites } => ((rgba, index), sprites, blits),
            Target::Sizes(sprites) => (self.shelves.clone(), sprites, Vec::new()),
            Target::Atlas(_) => unreachable!("a growth places its sprites itself"),
        };
        let mut atlas = AtlasGrowth { first, pages: shelves.0.pages, index_pages: shelves.1.pages, sprites, blits, palettes: own };
        // What of each new sprite draws nothing (where its pixels are new).
        atlas.trim_blank(&compiled.atlas.palettes, compiled.atlas.named_palettes);
        let growth = Growth {
            atlas,
            sprite_names,
            sets,
            layer_values,
            fonts,
            named,
            unavailable,
        };
        // (Only now: nothing is added on error.)
        self.shelves = shelves;
        self.texels = texels;
        self.rows = colourings;
        self.seen = seen;
        self.sheet_len = sheet_len;
        Ok(growth)
    }
}

impl Compiled {
    /// Applies growth and reports whether frames resolved before it retain
    /// their numeric sprite/set/font IDs and sprite/set name resolution.
    /// The frame owns its old layer lookup tables, so replacing a layer's
    /// named set does not itself invalidate those tables.
    ///
    /// This checks only names the growth writes. Sets and fonts append and
    /// old palettes remain a prefix by construction in `apply`. Appended
    /// palette rows can change old sprite transparency bounds, so only that
    /// case retains and compares the old sprite descriptors.
    pub fn apply_with_prefix_check(&mut self, growth: &Growth) -> bool {
        let old_empty_blend = self.set_blend.get(&SpriteSetId::EMPTY).copied().unwrap_or_default();
        let empty_blend_changed = growth.sets.iter().rev()
            .find(|(_, frames, _)| frames.is_empty())
            .is_some_and(|(_, _, blend)| *blend != old_empty_blend
                && self.set_ids.values().any(|id| *id == SpriteSetId::EMPTY));
        let final_sets: HashMap<&str, bool> = growth.sets.iter()
            .map(|(name, frames, _)| (name.as_str(), frames.is_empty())).collect();
        let set_name_preserved = |name: &String| {
            self.set_ids.get(name).is_none_or(|old| {
                // A later empty set can restore an old empty-set alias that
                // an earlier entry in this growth temporarily replaced.
                *old == SpriteSetId::EMPTY && final_sets.get(name.as_str()) == Some(&true)
            })
        };
        let names_preserved = !empty_blend_changed
            && self.sprite_names.len() <= self.atlas.sprites.len()
            && growth.sprite_names.iter().filter(|name| !name.is_empty())
                .all(|name| !self.sprite_ids.contains_key(name) && set_name_preserved(name))
            && growth.sets.iter().all(|(name, _, _)| set_name_preserved(name));
        let old_descriptors = (!growth.atlas.palettes.is_empty()).then(|| self.atlas.sprites.clone());
        self.apply(growth);
        names_preserved && old_descriptors.as_ref()
            .is_none_or(|old| self.atlas.sprites.starts_with(old))
    }

    /// Adds `growth` (made for this description as it is now, or for
    /// another copy of it, see [`Growing`]): the same sprite ids, set ids,
    /// palette rows and layer values on every copy. A copy with the atlas
    /// pixels gets the new pixels too.
    pub fn apply(&mut self, growth: &Growth) {
        let first = self.atlas.sprites.len();
        self.atlas.grow(&growth.atlas);
        self.sprite_names.resize(first, String::new());
        for (i, name) in growth.sprite_names.iter().enumerate() {
            let id = SpriteId((first + i) as u32);
            self.sprite_names.push(name.clone());
            if !name.is_empty() {
                self.sprite_ids.insert(name.clone(), id);
                let set = self.sets.single(id);
                self.set_ids.insert(name.clone(), set);
            }
        }
        for (name, frames, blend) in &growth.sets {
            let id = self.sets.add(frames);
            // (A set replaced: the layer values that drew it draw the new one.)
            if let Some(old) = self.set_ids.insert(name.clone(), id) {
                for layer in &mut self.layers {
                    if let Some(&v) = self.values.get(&(layer.name.clone(), name.clone())) {
                        if layer.lut.get(v as usize) == Some(&old) {
                            layer.lut[v as usize] = id;
                        }
                    }
                }
            }
            self.set_blend.insert(id, *blend);
        }
        for (i, names) in &growth.layer_values {
            let Some(layer) = self.layers.get_mut(*i) else { continue };
            if layer.palette_bits > 0 && layer.index_bits > 0 && layer.lut.len() + names.len() > 1 << layer.index_bits {
                log::error!("layer {}: more than {} sets, values that pick a palette change", layer.name, 1u32 << layer.index_bits);
            }
            for n in names {
                let Some(&set) = self.set_ids.get(n) else { continue };
                self.values.insert((layer.name.clone(), n.clone()), layer.lut.len() as u32);
                layer.lut.push(set);
            }
        }
        self.fonts.extend(growth.fonts.iter().cloned());
        self.palette_rows.extend(growth.named.iter().cloned());
        self.unavailable.extend(growth.unavailable.iter().copied());
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
    /// The row of every palette (see [`Description::palettes`]).
    palette_rows: HashMap<String, u32>,
    /// Every sprite by name.
    sprite_ids: HashMap<String, SpriteId>,
    /// Source sprites that could not fit in the atlas. Their transparent
    /// descriptors preserve ids but must not satisfy visible-art readiness.
    unavailable: BTreeSet<u32>,
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
            atlas: tile_render::Atlas {
                page_size: self.atlas.page_size,
                pages: Vec::new(),
                index_pages: Vec::new(),
                sprites: self.atlas.sprites.clone(),
                palettes: self.atlas.palettes.clone(),
                named_palettes: self.atlas.named_palettes,
            },
            sets: self.sets.clone(),
            layers: self.layers.clone(),
            sprite_names: self.sprite_names.clone(),
            values: self.values.clone(),
            set_ids: self.set_ids.clone(),
            set_blend: self.set_blend.clone(),
            fonts: self.fonts.clone(),
            palette_rows: self.palette_rows.clone(),
            sprite_ids: self.sprite_ids.clone(),
            unavailable: self.unavailable.clone(),
        }
    }

    /// The row of palette `name` (for a free sprite, `FreeSprite::palette`).
    pub fn palette(&self, name: &str) -> Option<u32> {
        self.palette_rows.get(name).copied()
    }

    /// Sprite `name`.
    pub fn sprite(&self, name: &str) -> Option<SpriteId> {
        self.sprite_ids.get(name).copied()
    }

    /// Whether this id has compiled art, including any zoomed-out twin.
    /// Transparent source art is available; a transparent placeholder made
    /// because a sprite did not fit the atlas is unavailable.
    pub fn sprite_available(&self, id: SpriteId) -> bool {
        let Some(sprite) = self.atlas.sprites.get(id.0 as usize) else { return false };
        !self.unavailable.contains(&id.0) && (sprite.downscaled == 0 || !self.unavailable.contains(&(sprite.downscaled - 1)))
    }

    /// Whether a named set has at least one frame and every frame's art
    /// was compiled. This does not mistake omitted art for a valid clear
    /// sprite just because its numeric placeholder exists.
    pub fn set_available(&self, name: &str) -> bool {
        let Some((set, _)) = self.set(name) else { return false };
        let mut frames = (0..).map_while(|i| self.sets.frame(set, i)).peekable();
        frames.peek().is_some() && frames.all(|id| self.sprite_available(id))
    }

    /// The cell value that draws set `set` on layer `layer` with palette row
    /// `palette` (0: none; else the layer must have `palettes = true`).
    pub fn value_in(&self, layer: &str, set: &str, palette: u32) -> Option<u32> {
        let index = self.value(layer, set)?;
        self.layers.iter().find(|l| l.name == layer)?.value_in(index, palette)
    }

    /// The cell value that draws set `set` on layer `layer` (0 = nothing).
    pub fn value(&self, layer: &str, set: &str) -> Option<u32> {
        self.values.get(&(layer.to_owned(), set.to_owned())).copied()
    }

    /// The set (sprite, or named animation) for a free sprite, and how it
    /// blends.
    /// The sets whose names start with `prefix`, by name.
    pub fn sets_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, SpriteSetId)> + 'a {
        self.set_ids.range::<str, _>((std::ops::Bound::Included(prefix), std::ops::Bound::Unbounded)).take_while(move |(n, _)| n.starts_with(prefix)).map(|(n, id)| (n.as_str(), *id))
    }

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


/// The anchor marked for the `size` cell at `at` of `frame` (an image of
/// `image` size; see `SheetDesc::anchor_marks`): minus the offsets of the
/// `mark` pixels in the row above and the column left of it, if both are
/// there.
fn anchor_marked(f: &Frames, frame: usize, image: (u32, u32), (x, y): (u32, u32), (w, h): (u32, u32), mark: [u8; 3]) -> Option<[i32; 2]> {
    let is_mark = |px: [u8; 4]| px[3] != 0 && (0..3).all(|c| px[c].abs_diff(mark[c]) <= 8);
    let (x0, y0) = (x.checked_sub(1)?, y.checked_sub(1)?);
    let hx = (0..w).find(|&i| x + i < image.0 && is_mark(f.pixel(frame, x + i, y0)))?;
    let hy = (0..h).find(|&j| y + j < image.1 && is_mark(f.pixel(frame, x0, y + j)))?;
    Some([-(hx as i32), -(hy as i32)])
}

/// The atlas page size of most descriptions: small enough for 32-bit
/// processes and software renderers (the atlas texture holds as many pages
/// as are in use, all of this size).
pub const DEFAULT_PAGE: u32 = 2048;

/// The largest page every device takes (wgpu's default limits): for
/// compiles not tied to one, or when its limit is unknown.
pub const ANY_DEVICE_PAGE: u32 = 8192;

/// The side of the (square) atlas pages for a description whose largest
/// sprite has side `largest`, on a renderer taking pages of at most
/// `max_page` (its device's `max_texture_dimension_2d`): [`DEFAULT_PAGE`],
/// larger only as far as that sprite needs (in steps of 256), never past
/// `max_page`. Every side compiling a description (one that draws, one that
/// keeps only the tables) gets the same size from the same two numbers.
pub fn page_size(largest: u32, max_page: u32) -> u32 {
    largest.next_multiple_of(256).max(DEFAULT_PAGE).min(max_page.max(1))
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

/// Sampling policy for a declared sprite sheet. Only pure background
/// textures opt in; controls, text and mixed art use exact nearest pixels.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Sampling {
    #[default]
    Nearest,
    UiBackground,
}

impl Sampling {
    fn flag(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::UiBackground => tile_render_shared::SAMPLING_UI_BACKGROUND,
        }
    }
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

    #[test]
    fn panes_divide_the_frame_and_free_ones_have_arrows_and_a_gap() {
        let c = [10, 20, 30, 255];
        let plain = panes("10,20,30,255:100x60:vertical21").unwrap();
        assert_eq!((plain.width, plain.height), (100, 60));
        // The frame, the divider at 1 + 98·2/3 = 66, clear panes.
        assert_eq!([plain.get(0, 0), plain.get(99, 59), plain.get(66, 30), plain.get(66, 1)], [c; 4]);
        assert_eq!(plain.get(33, 30), [0; 4]);
        assert_eq!(panes_centres(PanesSplit::Vertical2To1, (100, 60)), [(33, 30), (83, 30)]);
        assert_eq!(panes_centres(PanesSplit::Vertical1To1, (100, 60)), [(25, 30), (75, 30)]);
        assert_eq!(panes_centres(PanesSplit::Horizontal1To1, (100, 60)), [(50, 15), (50, 45)]);
        assert_eq!(panes_centres(PanesSplit::Standard, (100, 60)), [(50, 30)]);
        let free = panes("10,20,30,255:100x60:horizontal11:free").unwrap();
        // The divider (row 30) without its middle third (x 33..66).
        assert_eq!([free.get(10, 30), free.get(80, 30)], [c; 2]);
        assert_eq!(free.get(50, 30), [0; 4]);
        // Arrows around the top pane's middle (50, 15), their bases 8 and
        // their tips 11 pixels out.
        assert_eq!([free.get(42, 15), free.get(39, 15), free.get(58, 15), free.get(61, 15), free.get(50, 7), free.get(50, 4), free.get(50, 26)], [c; 7]);
        assert_eq!([free.get(38, 15), free.get(62, 15), free.get(50, 3), free.get(50, 27), free.get(50, 15)], [[0; 4]; 5]);
        // The head narrows: 7 pixels across at its base, 1 at its tip.
        assert_eq!((12..=18).filter(|&y| free.get(42, y) == c).count(), 7);
        assert_eq!((12..=18).filter(|&y| free.get(39, y) == c).count(), 1);
        let standard = panes("1,1,1,255:40x40:standard:free").unwrap();
        assert_eq!([standard.get(20, 12), standard.get(20, 28)], [[1, 1, 1, 255]; 2]);
        assert!(panes("1,1,1,255:40x40:diagonal").is_err());
        assert!(panes("1,1,1,255:40x40:standard:loose").is_err());
        assert!(panes("1,1,1,255:4x4:standard").is_err());
    }

    #[test]
    fn pointers_have_their_tip_at_one_one_and_a_translucent_outline() {
        let p = pointer("255,255,255,128:14x22").unwrap();
        let white = [255, 255, 255, 128];
        let edge = [0, 0, 0, 128];
        // The tip, outlined above and to the left, nothing beyond.
        assert_eq!(p.get(1, 1), white);
        assert_eq!((p.get(0, 1), p.get(1, 0)), (edge, edge));
        assert_eq!((p.get(0, 0), p.get(3, 1)), ([0; 4], [0; 4]));
        // The head widens downwards, the tail reaches the bottom right of it.
        let wide = |y: u32| (0..14).filter(|&x| p.get(x, y) == white).count();
        assert!(wide(12) > wide(4) && wide(4) > wide(1));
        let last = (0..14).filter(|&x| p.get(x, 20) == white).collect::<Vec<_>>();
        assert!(!last.is_empty() && last[0] > 4, "{last:?}");
        assert!(pointer("1,2,3,4:3x9").is_err());
    }

    #[test]
    fn glyphs_are_cells_of_text_on_their_background() {
        let (fg, bg) = ([255, 255, 255, 255], [0, 0, 0, 160]);
        let img = glyphs("255,255,255,255:0,0,0,160:2").unwrap();
        assert_eq!(glyph_cell(2), (12, 18));
        assert_eq!((img.width, img.height), (12 * GLYPHS.chars().count() as u32, 18));
        // The space is all background; "1" has its stem in the middle column
        // (glyph column 2, pixels 4..6), a row clear above it.
        let one = 12 * GLYPHS.find('1').unwrap() as u32;
        assert!((0..12).all(|x| (0..18).all(|y| img.get(x, y) == bg)));
        assert_eq!((img.get(one + 4, 2), img.get(one + 5, 15), img.get(one + 4, 1)), (fg, fg, bg));
        // The column right of the glyph stays clear.
        assert!((0..18).all(|y| img.get(one + 10, y) == bg && img.get(one + 11, y) == bg));
        assert!(glyphs("1,2,3,4:5,6,7,8:0").is_err());
        assert!(glyphs("1,2,3,4:5,6,7,8").is_err());
        assert_eq!(GLYPHS.chars().count(), GLYPH_ROWS.len());
        assert_eq!(&GLYPHS[..24], " 0123456789.%fpsmkingart", "existing glyph indices must stay stable");
    }

    #[test]
    fn rects_fill_in_order_and_repeat() {
        let img = rects("4x6; 0,0,0,255:1,0,2,6; 0,255,0,255:1,4,2,1*3,0,-2; 9,9,9,255:3,5,5,5").unwrap();
        assert_eq!((img.width, img.height), (4, 6));
        let col: Vec<[u8; 4]> = (0..6).map(|y| img.get(1, y)).collect();
        let (k, g) = ([0, 0, 0, 255], [0, 255, 0, 255]);
        assert_eq!(col, vec![g, k, g, k, g, k]);
        assert_eq!((img.get(0, 3), img.get(3, 5), img.get(3, 4)), ([0; 4], [9, 9, 9, 255], [0; 4]));
        assert!(rects("4x6; 1,2,3:0,0,1,1").is_err());
        assert!(rects("").is_err());
    }

    #[test]
    fn layers_may_carry_offsets_follow_phases_and_sweep_right_to_left() {
        let d = Description::parse(
            r#"
            grid = { layout = "staggered", step = [64, 32] }
            [images.a]
            source = "a.pcx"
            [sheets.s]
            image = "a"
            cell = [10, 10]
            [[layers]]
            name = "units"
            sets = ["s/*"]
            pass = 2
            per_cell_offset = true
            per_cell_phase = true
            ticks_per_frame = 0
            descending = true
            "#,
        )
        .unwrap();
        let c = d.compile(&Checker, 256).unwrap();
        let l = &c.layers[0];
        assert_eq!((l.per_cell_offset, l.per_cell_phase, l.ticks_per_frame, l.descending), (true, true, 0, true));
        let mut w = c.world(3, 3, true, false);
        w.set_offset(tile_render::LayerId(0), 1, 2, (-3, 4));
        assert!(w.passes()[0].descending);
        let mixed = Description::parse(
            r#"
            grid = { layout = "staggered", step = [64, 32] }
            [[layers]]
            name = "a"
            sets = []
            [[layers]]
            name = "b"
            sets = []
            descending = true
            "#,
        )
        .unwrap();
        assert!(mixed.compile(&Checker, 256).is_err(), "the layers of a pass sweep it one way");
    }

    struct Checker;
    impl ImageLoader for Checker {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            let seed = source.len() as u8;
            Ok(vec![RgbaImage::from_fn(40, 20, |x, y| [x as u8, y as u8, seed, 255])])
        }
    }

    /// Indexed images: frame `f` of "a.flc" is 8×4 indices `x + 10·f` (to
    /// 63), its colours grey ramps, index 0 clear, `shadow = true` making
    /// all but 9 clear; "pal" palettes are red ramps.
    struct Indexed;
    impl ImageLoader for Indexed {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Ok(vec![RgbaImage::from_fn(8, 4, |x, y| [x as u8, y as u8, source.len() as u8, 255])])
        }
        fn indexed(&self, source: &str, options: &toml::Table) -> Result<Option<Vec<IndexedFrame>>, String> {
            if source != "a.flc" {
                return Ok(None);
            }
            let shadow = options.get("shadow").and_then(toml::Value::as_bool).unwrap_or(false);
            let palette: Vec<[u8; 4]> = (0..256u32).map(|i| if i == 0 || (shadow && i != 9) { [0; 4] } else { [i as u8, i as u8, i as u8, 255] }).collect();
            Ok(Some((0..3u32).map(|f| IndexedFrame { pixels: IndexImage::from_fn(8, 4, |x, _| (x + 10 * f) as u8), palette: palette.clone() }).collect()))
        }
        fn palette(&self, source: &str, _: &toml::Table) -> Result<Vec<[u8; 4]>, String> {
            let k = source.len() as u32;
            Ok((0..256u32).map(|i| [(i * k) as u8, 0, 0, 255]).collect())
        }
    }

    #[test]
    fn indexed_images_share_their_pixels_and_pick_named_palettes() {
        let text = r#"
            grid = { layout = "orthogonal", step = [8, 4] }
            [palettes.civ]
            source = "pal{v}"
            variants = [1, 22]
            [palettes.army]
            base = "civ@1"
            copy = [["civ@22", 2, 5, 2]]
            [images.body]
            source = "a.flc"
            remap = [2, 6]
            [images.shadow]
            source = "a.flc"
            shadow = true
            [images.plain]
            source = "b.pcx"
            [images.colours]
            source = "a.flc"
            indexed = false
            [sheets.body]
            image = "body"
            all_frames = true
            cell = [8, 4]
            [sheets.shadow]
            image = "shadow"
            all_frames = true
            cell = [8, 4]
            [sheets.plain]
            image = "plain"
            cell = [8, 4]
            [sheets.colours]
            image = "colours"
            cell = [8, 4]
            [[layers]]
            name = "units"
            sets = ["body/*"]
            palettes = true
        "#;
        let c = Description::parse(text).unwrap().compile(&Indexed, 64).unwrap();
        let sprite = |n: &str| c.atlas.sprites[c.sprite(n).unwrap().0 as usize];
        // Every frame, indexed; the shadow on the body's pixels, its own row.
        let (body, shadow) = (sprite("body/2"), sprite("shadow/2"));
        assert_eq!((body.page, body.atlas_x, body.atlas_y), (shadow.page, shadow.atlas_x, shadow.atlas_y));
        assert!(body.palette != 0 && shadow.palette != 0 && body.palette != shadow.palette);
        assert_eq!((body.remap, shadow.remap), (tile_render_shared::remap_range(2, 6), 0));
        // Frames of one palette share its row; the colour images are not indexed.
        assert_eq!(sprite("body/0").palette, body.palette);
        assert_eq!((sprite("plain/0").palette, sprite("colours/0").palette), (0, 0));
        assert_eq!((c.atlas.index_pages.len(), c.atlas.pages.len()), (1, 1));
        // Named rows after the own ones (row 0 clear, body, shadow).
        assert_eq!(c.atlas.named_palettes, 3);
        let row = |name: &str| c.palette(name).unwrap();
        assert_eq!((row("army"), row("civ@1"), row("civ@22")), (1, 2, 3));
        let entry = |r: u32, i: u32| c.atlas.palettes[((c.atlas.named_palettes + r - 1) * tile_render_shared::PALETTE_SIZE + i) as usize].to_le_bytes();
        // (civ@1: 4 · i, from "pal1"; civ@22: 5 · i, from "pal22"; the army
        // row civ@1's with entries 2, 3 of civ@22 at 5, 6.)
        assert_eq!((entry(2, 3), entry(3, 3), entry(1, 5), entry(1, 7)), ([12, 0, 0, 255], [15, 0, 0, 255], [10, 0, 0, 255], [28, 0, 0, 255]));
        // Drawn: the body's frame 2 (indices 20..27) through civ@22 where remapped (none are: 2..6).
        let tile = tile_render_shared::indexed_color(22, body.palette, body.remap, row("civ@22"), c.atlas.named_palettes, &c.atlas.palettes);
        assert_eq!(tile, [22, 22, 22, 255]);
        let remapped = tile_render_shared::indexed_color(3, body.palette, body.remap, row("civ@22"), c.atlas.named_palettes, &c.atlas.palettes);
        assert_eq!(remapped, [15, 0, 0, 255]);
        // A layer value picks a named palette in its upper bits.
        let v = c.value_in("units", "body/1", row("civ@22")).unwrap();
        assert_eq!(v & 3, c.value("units", "body/1").unwrap());
        assert_eq!(c.layers[0].value_in(c.value("units", "body/1").unwrap(), 3), Some(v));
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
        assert_eq!(c.fonts.len(), 1);
        assert!(c.font(FALLBACK_FONT).is_some() && c.font("Lucida Sans").is_none());
        let install = std::path::PathBuf::from(std::env::var("CIV3_INSTALL").unwrap_or_else(|_| "/mnt/datasets/winvm/civ3-src/Civilization III Complete".into()));
        if !install.join("LSANS.TTF").is_file() {
            return eprintln!("no Civ III installation, skipped");
        }
        for c in [d.compile(&Fonts(install.clone()), 256).unwrap(), d.compile_layout(&Fonts(install), 256).unwrap().without_pixels()] {
            let (index, face) = c.font("Lucida Sans").expect("loaded");
            assert_eq!(index, 0);
            assert!(face.glyph('R').is_some());
            assert!(c.font("missing").is_none());
            assert_eq!(c.world(2, 2, false, false).fonts.len(), 2);
        }
    }

    #[test]
    fn explicit_background_sampling_preserves_source_origins_across_compile_modes_and_growth() {
        let source = r#"
            grid = { layout = "orthogonal", step = [1, 1] }
            [images.a]
            source = "a.pcx"
            [sheets.texture]
            image = "a"
            origin = [4, 2]
            cell = [24, 16]
            grid = [1, 1]
            sampling = "ui-background"
            [sheets.texture_crop]
            sprite = "texture/0"
            origin = [3, 5]
            cell = [8, 8]
            grid = [1, 1]
            sampling = "ui-background"
            [sheets.control_crop]
            sprite = "texture/0"
            cell = [8, 8]
            grid = [1, 1]
        "#;
        let desc = Description::parse(source).unwrap();
        let expected = [("texture/0", 1, (4, 2)), ("texture_crop/0", 1, (7, 7)), ("control_crop/0", 0, (4, 2))];
        let check = |compiled: &Compiled| {
            for (name, sampling, origin) in expected {
                let id = compiled.sprite(name).unwrap();
                let sprite = compiled.atlas.sprites[id.0 as usize];
                assert_eq!(sprite.sampling, sampling, "{name}");
                assert_eq!((sprite.sampling_origin_x, sprite.sampling_origin_y), origin, "{name}");
            }
        };
        check(&desc.compile(&Checker, 128).unwrap());
        check(&desc.compile_layout(&Checker, 128).unwrap());
        let mut cache = SheetCache::new();
        check(&desc.compile_cached(&Checker, 128, &mut cache).unwrap());
        check(&desc.compile_cached(&Checker, 128, &mut cache).unwrap());
        let (mut compiled, mut growing) = Description::parse("grid = { layout = 'orthogonal', step = [1, 1] }").unwrap().compile_growing(&Checker, 128, true).unwrap();
        let addition = source.lines().skip(2).collect::<Vec<_>>().join("\n");
        let growth = growing.grow(&compiled, &addition, &Checker).unwrap();
        compiled.apply(&growth);
        check(&compiled);
    }

    #[test]
    fn background_sampling_does_not_accept_art_with_a_baked_resize_or_outline() {
        for operation in ["resize = [8, 8]", "outline = { width = 1, color = [1,2,3,255] }"] {
            let source = format!("grid = {{ layout = 'orthogonal', step = [1, 1] }}\n[images.a]\nsource = 'a.pcx'\n[sheets.background]\nimage = 'a'\ncell = [16, 16]\ngrid = [1, 1]\nsampling = 'ui-background'\n{operation}\n");
            let desc = Description::parse(&source).unwrap();
            assert!(desc.compile(&Checker, 128).err().unwrap().to_string().contains("requires unresized source art"));
        }
    }

    /// Indexed source for alpha-mask tests. Every used index keeps an
    /// identifiable colour, with optional keyed or partial transparency.
    struct MaskedIndexed;
    impl ImageLoader for MaskedIndexed {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Err("indexed fixture should not load RGBA".into())
        }
        fn sizes(&self, source: &str, _: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
            Ok(vec![if source == "full" { (257, 1) } else { (8, 2) }])
        }
        fn indexed(&self, source: &str, options: &toml::Table) -> Result<Option<Vec<IndexedFrame>>, String> {
            let clear = options.get("clear").and_then(toml::Value::as_integer);
            let palette = (0..256).map(|i| [i as u8, 255 - i as u8, 17, if clear == Some(i) { 0 } else if i == 3 { 99 } else { 255 }]).collect();
            let pixels = if source == "full" { IndexImage::from_fn(257, 1, |x, _| x as u8) } else { IndexImage::from_fn(8, 2, |x, y| ((x + 2 * y) % 8) as u8) };
            Ok(Some(vec![IndexedFrame { pixels, palette }]))
        }
        fn palette(&self, _: &str, _: &toml::Table) -> Result<Vec<[u8; 4]>, String> {
            Ok((0..256).map(|i| [37, i as u8, 99, 255]).collect())
        }
    }

    fn indexed_sprite_pixels(compiled: &Compiled, name: &str, named: u32) -> Vec<[u32; 4]> {
        let s = compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
        (0..s.height).flat_map(|y| (0..s.width).map(move |x| {
            let index = compiled.atlas.index_pages[s.page as usize].get(s.atlas_x + x, s.atlas_y + y) as u32;
            tile_render_shared::indexed_color(index, s.palette, s.remap, named, compiled.atlas.named_palettes, &compiled.atlas.palettes)
        })).collect()
    }

    const MASKED_RGBA_SHEETS: &str = r#"
        [images.art]
        source = "a.pcx"
        [sheets.original]
        image = "art"
        origin = [3, 2]
        cell = [8, 6]
        grid = [1, 1]
        anchor = [-2, 4]
        scaling = "keep"
        [sheets.keep]
        image = "art"
        origin = [3, 2]
        cell = [8, 6]
        grid = [1, 1]
        anchor = [-2, 4]
        scaling = "keep"
        sampling = "ui-background"
        alpha_keep = [[1, 1, 4, 3], [4, 2, 3, 3]]
        [sheets.clear]
        image = "art"
        origin = [3, 2]
        cell = [8, 6]
        grid = [1, 1]
        anchor = [-2, 4]
        scaling = "keep"
        alpha_clear = [[1, 1, 4, 3], [4, 2, 3, 3]]
    "#;

    struct PartialAlpha;
    impl ImageLoader for PartialAlpha {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Ok(vec![RgbaImage::from_fn(12, 10, |x, y| [x as u8, y as u8, 73, [0, 1, 64, 128, 255][((x + y) % 5) as usize]])])
        }
        fn sizes(&self, _: &str, _: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
            Ok(vec![(12, 10)])
        }
    }

    #[test]
    fn source_alpha_masks_complement_keyed_and_partial_rgba_without_geometry_changes() {
        let desc = Description::parse(&format!("grid = {{ layout = 'orthogonal', step = [1, 1] }}\n{MASKED_RGBA_SHEETS}")).unwrap();
        let compiled = desc.compile(&PartialAlpha, 64).unwrap();
        let original = sprite_pixels(&compiled, "original/0");
        let kept = sprite_pixels(&compiled, "keep/0");
        let cleared = sprite_pixels(&compiled, "clear/0");
        let mask = desc.sheets["keep"].alpha_mask().unwrap();
        for y in 0..6 {
            for x in 0..8 {
                let at = (y * 8 + x) as usize;
                let (retained, dropped) = if mask.retains(x, y) { (kept[at], cleared[at]) } else { (cleared[at], kept[at]) };
                assert_eq!(retained, original[at]);
                assert_eq!(dropped, [original[at][0], original[at][1], original[at][2], 0]);
                assert_eq!(u16::from(kept[at][3]) + u16::from(cleared[at][3]), u16::from(original[at][3]));
                // Applying one uniform draw opacity still gives precisely
                // one source contribution, including partial source alpha.
                for opacity in [0u32, 1, 63, 128, 255] {
                    let effective = |a: u8| u32::from(a) * opacity / 255;
                    assert_eq!(effective(kept[at][3]) + effective(cleared[at][3]), effective(original[at][3]));
                }
            }
        }
        for name in ["original/0", "keep/0", "clear/0"] {
            let s = compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
            assert_eq!((s.width, s.height, s.anchor_x, s.anchor_y, s.scaling), (8, 6, -2, 4, tile_render_shared::KEEP_SIZE));
            assert_eq!((s.box_x, s.box_y, s.box_w, s.box_h), (0, 0, 8, 6));
            assert_eq!((s.sampling_origin_x, s.sampling_origin_y), (3, 2));
        }
    }

    #[test]
    fn source_alpha_masks_precede_outline_and_resize() {
        let source = "grid = { layout = 'orthogonal', step = [1, 1] }\n[images.art]\nsource = 'solid:7,8,9,91:4x4'\n[sheets.masked]\nimage = 'art'\ncell = [4, 4]\ngrid = [1, 1]\nalpha_keep = [[1, 1, 2, 2]]\n";
        let resized = Description::parse(&format!("{source}resize = [2, 2]\n")).unwrap().compile(&Checker, 64).unwrap();
        assert_eq!(sprite_pixels(&resized, "masked/0"), vec![[7, 8, 9, 91], [7, 8, 9, 0], [7, 8, 9, 0], [7, 8, 9, 0]]);
        let outlined = Description::parse(&format!("{source}outline = {{ width = 1, color = [1,2,3,255] }}\n")).unwrap().compile(&Checker, 64).unwrap();
        let image = RgbaImage::from_fn(4, 4, |x, y| [7, 8, 9, if (1..3).contains(&x) && (1..3).contains(&y) { 91 } else { 0 }]);
        assert_eq!(sprite_pixels(&outlined, "masked/0"), outline(&image, 1, [1, 2, 3, 255]).pixels);
    }

    #[test]
    fn source_alpha_masks_reject_invalid_rectangles_box_sampling_and_sprite_aliases() {
        let base = "grid = { layout = 'orthogonal', step = [1, 1] }\n[images.art]\nsource = 'a.pcx'\n[sheets.masked]\nimage = 'art'\ncell = [8, 6]\ngrid = [1, 1]\n";
        for extra in [
            "alpha_keep = [[1, 1, 0, 2]]",
            "alpha_clear = [[1, 1, 2, 0]]",
            "alpha_keep = [[7, 0, 2, 1]]",
            "alpha_clear = [[0, 5, 1, 2]]",
            "alpha_keep = [[4294967295, 0, 2, 1]]",
            "alpha_keep = []\nalpha_clear = []",
            "alpha_keep = [[1,1,2,2]]\nsample_from = 'box'",
            "alpha_keep = [[1,1,2,2]]\nsprite = 'anything/0'",
        ] {
            let desc = Description::parse(&format!("{base}{extra}\n")).unwrap();
            assert!(matches!(desc.compile(&Checker, 64), Err(DescError::Sheet { .. })), "{extra}");
            assert!(matches!(desc.compile_layout(&Checker, 64), Err(DescError::Sheet { .. })), "{extra}");
        }
    }

    #[test]
    fn indexed_alpha_masks_preserve_retained_indices_and_named_palette_remapping() {
        let source = r#"
            grid = { layout = "orthogonal", step = [1, 1] }
            [images.art]
            source = "partial"
            remap = [0, 4]
            [palettes.alternate]
            source = "alternate"
            [sheets.original]
            image = "art"
            cell = [8, 2]
            grid = [1, 1]
            [sheets.keep]
            image = "art"
            cell = [8, 2]
            grid = [1, 1]
            alpha_keep = [[0, 0, 4, 2]]
            [sheets.clear]
            image = "art"
            cell = [8, 2]
            grid = [1, 1]
            alpha_clear = [[0, 0, 4, 2]]
        "#;
        let compiled = Description::parse(source).unwrap().compile(&MaskedIndexed, 64).unwrap();
        assert!(compiled.atlas.sprites.iter().all(|s| s.palette != 0), "source masks retain one-byte indexed storage");
        let original = compiled.atlas.sprites[compiled.sprite("original/0").unwrap().0 as usize];
        for (name, keep) in [("keep/0", true), ("clear/0", false)] {
            let s = compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
            assert_eq!(s.remap, original.remap);
            assert_ne!((s.atlas_x, s.atlas_y), (original.atlas_x, original.atlas_y));
            for y in 0..2 {
                for x in 0..8 {
                    let i = compiled.atlas.index_pages[s.page as usize].get(s.atlas_x + x, s.atlas_y + y);
                    if (x < 4) == keep {
                        assert_eq!(i, ((x + 2 * y) % 8) as u8, "retained indices are never rewritten");
                    } else {
                        assert!(!(0..4).contains(&i), "replacement is outside the named remap range");
                        assert_eq!(compiled.atlas.palettes[(s.palette * 256 + u32::from(i)) as usize] >> 24, 0);
                    }
                }
            }
        }
        for named in [0, compiled.palette("alternate").unwrap()] {
            let original = indexed_sprite_pixels(&compiled, "original/0", named);
            let kept = indexed_sprite_pixels(&compiled, "keep/0", named);
            let cleared = indexed_sprite_pixels(&compiled, "clear/0", named);
            for at in 0..16 {
                let (retained, dropped) = if at % 8 < 4 { (kept[at], cleared[at]) } else { (cleared[at], kept[at]) };
                assert_eq!(retained, original[at]);
                assert_eq!(dropped[3], 0);
            }
        }
    }

    #[test]
    fn indexed_alpha_masks_reuse_keyed_indices_without_mutating_the_original_palette() {
        let source = r#"
            grid = { layout = "orthogonal", step = [1, 1] }
            [images.art]
            source = "partial"
            clear = 3
            [sheets.original]
            image = "art"
            cell = [8, 2]
            grid = [1, 1]
            [sheets.keep]
            image = "art"
            cell = [8, 2]
            grid = [1, 1]
            alpha_keep = [[0, 0, 4, 2]]
        "#;
        let compiled = Description::parse(source).unwrap().compile(&MaskedIndexed, 64).unwrap();
        let sprite = |name: &str| compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
        let original = sprite("original/0");
        let keep = sprite("keep/0");
        assert_eq!(original.palette, keep.palette, "existing transparent entry does not need a new palette row");
        for y in 0..2 {
            for x in 0..8 {
                assert_eq!(compiled.atlas.index_pages[keep.page as usize].get(keep.atlas_x + x, keep.atlas_y + y), if x < 4 { ((x + 2 * y) % 8) as u8 } else { 3 });
            }
        }
        assert_eq!(indexed_sprite_pixels(&compiled, "original/0", 0)[7], [7, 248, 17, 255]);
        assert_eq!(indexed_sprite_pixels(&compiled, "original/0", 0)[3][3], 0);
    }

    #[test]
    fn indexed_alpha_masks_fall_back_to_rgba_when_all_indices_remain_used() {
        let source = "grid = { layout = 'orthogonal', step = [1, 1] }\n[images.art]\nsource = 'full'\n[sheets.keep]\nimage = 'art'\ncell = [257, 1]\ngrid = [1, 1]\nalpha_keep = [[0, 0, 256, 1]]\n";
        let compiled = Description::parse(source).unwrap().compile(&MaskedIndexed, 512).unwrap();
        assert_eq!(compiled.atlas.index_pages.len(), 0);
        let pixels = sprite_pixels(&compiled, "keep/0");
        for i in 0..256 {
            assert_eq!(pixels[i], [i as u8, 255 - i as u8, 17, if i == 3 { 99 } else { 255 }]);
        }
        assert_eq!(pixels[256], [0, 255, 17, 0]);
        let remapped = source.replace("source = 'full'", "source = 'full'\nremap = [0, 256]");
        assert!(Description::parse(&remapped).unwrap().compile(&MaskedIndexed, 512).err().unwrap().to_string().contains("no safe transparent index"));
    }

    #[test]
    fn indexed_alpha_masks_with_different_palette_options_keep_distinct_replacement_indices() {
        let source = r#"
            grid = { layout = "orthogonal", step = [1, 1] }
            [images.first]
            source = "partial"
            clear = 3
            [images.second]
            source = "partial"
            clear = 5
            [sheets.first]
            image = "first"
            cell = [8, 2]
            grid = [1, 1]
            alpha_keep = [[0, 0, 4, 2]]
            [sheets.second]
            image = "second"
            cell = [8, 2]
            grid = [1, 1]
            alpha_keep = [[0, 0, 4, 2]]
        "#;
        let desc = Description::parse(source).unwrap();
        let mut cache = SheetCache::new();
        for compiled in [desc.compile_cached(&MaskedIndexed, 64, &mut cache).unwrap(), desc.compile_cached(&MaskedIndexed, 64, &mut cache).unwrap()] {
            let sprite = |name: &str| compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
            let first = sprite("first/0");
            let second = sprite("second/0");
            assert_ne!((first.atlas_x, first.atlas_y), (second.atlas_x, second.atlas_y));
            for (s, transparent) in [(first, 3), (second, 5)] {
                assert_eq!(compiled.atlas.index_pages[s.page as usize].get(s.atlas_x + 7, s.atlas_y), transparent);
            }
            let first_pixels = indexed_sprite_pixels(&compiled, "first/0", 0);
            let second_pixels = indexed_sprite_pixels(&compiled, "second/0", 0);
            assert_eq!((first_pixels[3][3], second_pixels[3][3]), (0, 99));
            assert_eq!((first_pixels[7][3], second_pixels[7][3]), (0, 0));
        }
    }

    #[test]
    fn identity_alpha_masks_keep_full_indexed_palettes_and_dynamic_remapping() {
        let source = "grid = { layout = 'orthogonal', step = [1, 1] }\n[images.art]\nsource = 'full'\nremap = [0, 256]\n[sheets.keep]\nimage = 'art'\ncell = [257, 1]\ngrid = [1, 1]\nalpha_keep = [[0, 0, 257, 1]]\n";
        let compiled = Description::parse(source).unwrap().compile(&MaskedIndexed, 512).unwrap();
        let s = compiled.atlas.sprites[compiled.sprite("keep/0").unwrap().0 as usize];
        assert_ne!(s.palette, 0);
        assert_eq!(s.remap, tile_render_shared::remap_range(0, 256));
        for x in 0..257 { assert_eq!(compiled.atlas.index_pages[s.page as usize].get(s.atlas_x + x, s.atlas_y), x as u8); }
    }

    #[test]
    fn source_alpha_masks_keep_source_origins_across_cache_layout_and_growth() {
        let source = format!("grid = {{ layout = 'orthogonal', step = [1, 1] }}\n{MASKED_RGBA_SHEETS}");
        let desc = Description::parse(&source).unwrap();
        let reference = desc.compile(&PartialAlpha, 64).unwrap();
        let mut cache = SheetCache::new();
        for compiled in [desc.compile_cached(&PartialAlpha, 64, &mut cache).unwrap(), desc.compile_cached(&PartialAlpha, 64, &mut cache).unwrap(), desc.compile_layout(&PartialAlpha, 64).unwrap()] {
            assert_eq!(compiled.sprite_names, reference.sprite_names);
            for name in ["original/0", "keep/0", "clear/0"] {
                let s = compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
                let expected = reference.atlas.sprites[reference.sprite(name).unwrap().0 as usize];
                assert_eq!((s.width, s.height, s.anchor_x, s.anchor_y, s.sampling, s.sampling_origin_x, s.sampling_origin_y), (expected.width, expected.height, expected.anchor_x, expected.anchor_y, expected.sampling, expected.sampling_origin_x, expected.sampling_origin_y));
                if !compiled.atlas.pages.is_empty() { assert_eq!(sprite_pixels(&compiled, name), sprite_pixels(&reference, name)); }
            }
        }
        let base = Description::parse("grid = { layout = 'orthogonal', step = [1, 1] }").unwrap();
        for pixels in [true, false] {
            let (mut compiled, mut growing) = base.compile_growing(&PartialAlpha, 64, pixels).unwrap();
            let growth = growing.grow(&compiled, MASKED_RGBA_SHEETS, &PartialAlpha).unwrap();
            compiled.apply(&growth);
            for name in ["original/0", "keep/0", "clear/0"] {
                let s = compiled.atlas.sprites[compiled.sprite(name).unwrap().0 as usize];
                assert_eq!((s.sampling_origin_x, s.sampling_origin_y), (3, 2));
                if pixels { assert_eq!(sprite_pixels(&compiled, name), sprite_pixels(&reference, name)); }
            }
        }
    }

    #[test]
    fn indexed_alpha_mask_cache_and_growth_do_not_share_different_pixels() {
        let base = "grid = { layout = 'orthogonal', step = [1, 1] }\n";
        let original = "[images.art]\nsource = 'partial'\n[sheets.original]\nimage = 'art'\ncell = [8, 2]\ngrid = [1, 1]\n";
        let masked = "[sheets.keep]\nimage = 'art'\ncell = [8, 2]\ngrid = [1, 1]\nalpha_keep = [[0, 0, 4, 2]]\n[sheets.clear]\nimage = 'art'\ncell = [8, 2]\ngrid = [1, 1]\nalpha_clear = [[0, 0, 4, 2]]\n";
        let source = format!("{base}{original}{masked}");
        let desc = Description::parse(&source).unwrap();
        let plain = desc.compile(&MaskedIndexed, 64).unwrap();
        let mut cache = SheetCache::new();
        Description::parse(&format!("{base}{original}")).unwrap().compile_cached(&MaskedIndexed, 64, &mut cache).unwrap();
        for compiled in [desc.compile_cached(&MaskedIndexed, 64, &mut cache).unwrap(), desc.compile_cached(&MaskedIndexed, 64, &mut cache).unwrap()] {
            for name in ["original/0", "keep/0", "clear/0"] { assert_eq!(indexed_sprite_pixels(&compiled, name, 0), indexed_sprite_pixels(&plain, name, 0)); }
        }
        let changed = Description::parse(&source.replace("alpha_keep = [[0, 0, 4, 2]]", "alpha_keep = [[2, 0, 4, 2]]")).unwrap().compile_cached(&MaskedIndexed, 64, &mut cache).unwrap();
        assert_eq!(indexed_sprite_pixels(&changed, "keep/0", 0)[0][3], 0, "mask changes invalidate cached texels");
        assert_eq!(indexed_sprite_pixels(&changed, "keep/0", 0)[4][3], 255);
        let (mut grown, mut growing) = Description::parse(&format!("{base}{original}")).unwrap().compile_growing(&MaskedIndexed, 64, true).unwrap();
        let addition = format!("[images.art]\nsource = 'partial'\n{masked}");
        let growth = growing.grow(&grown, &addition, &MaskedIndexed).unwrap();
        assert_eq!(growth.atlas.blits.len(), 2, "keep and clear cannot reuse unmasked or each other's texels");
        grown.apply(&growth);
        for name in ["original/0", "keep/0", "clear/0"] { assert_eq!(indexed_sprite_pixels(&grown, name, 0), indexed_sprite_pixels(&plain, name, 0)); }
        let alias = "[images.art]\nsource = 'partial'\n[sheets.keep_again]\nimage = 'art'\ncell = [8, 2]\ngrid = [1, 1]\nalpha_keep = [[0, 0, 4, 2]]\n";
        let growth = growing.grow(&grown, alias, &MaskedIndexed).unwrap();
        assert!(growth.atlas.blits.is_empty(), "identical masked indices can share previous pixels");
        grown.apply(&growth);
        assert_eq!(indexed_sprite_pixels(&grown, "keep_again/0", 0), indexed_sprite_pixels(&grown, "keep/0", 0));
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
    fn pages_are_as_large_as_the_largest_sprite_needs() {
        // 2048² unless a sprite is larger; then in steps of 256 up to the
        // device's limit, never beyond it.
        assert_eq!(page_size(128, 16384), 2048);
        assert_eq!(page_size(3840, 16384), 3840);
        assert_eq!(page_size(3841, 16384), 4096);
        assert_eq!(page_size(3840, 4000), 3840);
        assert_eq!(page_size(5000, 4096), 4096);
        assert_eq!(page_size(10, 256), 256);
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
        // Sprites too big for any page are left out (a clear pixel), the same
        // with or without the pixels: they do not cost the others.
        let (small, small_full) = (d.compile_layout(&Checker, 8).unwrap(), d.compile(&Checker, 8).unwrap());
        assert_eq!(tables(&small), tables(&small_full));
        assert!(small.atlas.sprites.iter().any(|s| (s.width, s.height) == (1, 1)));
        assert!(small_full.atlas.sprites.iter().all(|s| s.width <= 8 && s.height <= 8));
    }

    const UNAVAILABLE_ART: &str = r#"
        [images.clear]
        source = "solid:0,0,0,0:1x1"
        [sheets.clear]
        image = "clear"
        cell = [1, 1]
        grid = [1, 1]
        [images.large]
        source = "solid:255,10,20,255:8x2"
        [sheets.large]
        image = "large"
        cell = [8, 2]
        grid = [1, 1]
        [sheets.part]
        sprite = "large/0"
        cell = [1, 1]
        grid = [1, 1]
        [sets.mixed]
        frames = ["clear/0", "large/0"]
        "#;

    #[test]
    fn oversized_art_is_unavailable_without_confusing_legitimate_transparency() {
        let description = Description::parse(&format!("grid = {{ layout = \"orthogonal\", step = [1, 1] }}\n{UNAVAILABLE_ART}")).unwrap();
        let mut cache = SheetCache::default();
        let small = description.compile_cached(&Checker, 4, &mut cache).unwrap();
        let layout = description.compile_layout(&Checker, 4).unwrap();
        let tables = small.without_pixels();
        for compiled in [&small, &layout, &tables] {
            assert!(compiled.set_available("clear/0"), "legitimate transparent art is complete");
            assert!(compiled.sprite("large/0").is_some() && compiled.set("large/0").is_some(), "omission preserves numeric lookup");
            assert_eq!(compiled.atlas.sprites[compiled.sprite("large/0").unwrap().0 as usize].width, 1);
            assert!(!compiled.sprite_available(compiled.sprite("large/0").unwrap()));
            assert!(!compiled.set_available("large/0"), "an opaque source replaced with a clear pixel is missing art");
            assert!(!compiled.set_available("part/0"), "aliases of missing art remain unavailable");
            assert!(!compiled.set_available("mixed"), "every animation frame needs its source art");
        }
        let recovered = description.compile_cached(&Checker, 16, &mut cache).unwrap();
        assert_eq!(small.sprite_names, recovered.sprite_names);
        assert_eq!(small.sprite("large/0"), recovered.sprite("large/0"));
        assert_eq!(small.set("mixed"), recovered.set("mixed"));
        assert!(recovered.set_available("large/0") && recovered.set_available("part/0") && recovered.set_available("mixed"), "cache retains source art rather than the unavailable placeholder");
        assert_eq!(sprite_pixels(&recovered, "large/0"), vec![[255, 10, 20, 255]; 16]);
    }

    #[test]
    fn unavailable_growth_metadata_survives_apply_cloning_and_later_aliases() {
        let base = Description::parse("grid = { layout = \"orthogonal\", step = [1, 1] }").unwrap();
        let (mut full, mut growing) = base.compile_growing(&Checker, 4, true).unwrap();
        let (mut layout, mut layout_growing) = base.compile_growing(&Checker, 4, false).unwrap();
        let mut tables = full.without_pixels();
        let growth = growing.grow(&full, UNAVAILABLE_ART, &Checker).unwrap();
        let copy = growth.clone();
        let layout_growth = layout_growing.grow(&layout, UNAVAILABLE_ART, &Checker).unwrap();
        full.apply(&growth);
        tables.apply(&copy);
        layout.apply(&layout_growth);
        assert_eq!(full.sprite_names, layout.sprite_names);
        assert_eq!(full.sprite_names, tables.sprite_names);
        let alias = "[sheets.late]\nsprite = \"large/0\"\ncell = [1, 1]\ngrid = [1, 1]\n";
        let next = growing.grow(&full, alias, &Checker).unwrap();
        let layout_next = layout_growing.grow(&layout, alias, &Checker).unwrap();
        full.apply(&next);
        tables.apply(&next);
        layout.apply(&layout_next);
        for compiled in [&full, &tables, &layout, &full.without_pixels()] {
            assert!(compiled.set_available("clear/0"));
            for name in ["large/0", "part/0", "mixed", "late/0"] { assert!(!compiled.set_available(name), "{name}"); }
            assert_eq!(compiled.sprite("late/0"), full.sprite("late/0"));
        }
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

    const BASE: &str = r#"
        grid = { layout = "orthogonal", step = [10, 10] }
        [images.a]
        source = "a"
        [sheets.s]
        image = "a"
        cell = [10, 10]
        grid = [4, 1]
        [sets.anim]
        frames = ["s/0", "s/1"]
        [[layers]]
        name = "l"
        sets = ["s/*", "anim", "U@*"]
        [[layers]]
        name = "m"
        sets = ["Anim@*"]
        "#;

    /// Two additions: units of art (sheets of a new image, an animation of
    /// them), then more, some of it again, and parts of a sprite of the
    /// first.
    const ADD1: &str = r#"
        [images.u]
        source = "uuu"
        [sheets.u]
        image = "u"
        cell = [8, 5]
        grid = [5, 2]
        group = "U@one"
        [sets."Anim@one"]
        frames = ["U@one/0", "U@one/3", "s/2"]
        "#;
    const ADD2: &str = r#"
        [images.u]
        source = "vvvvv"
        [sheets.u]
        image = "u"
        cell = [20, 20]
        grid = [2, 1]
        group = "U@two"
        [images.again]
        source = "uuu"
        [sheets.again]
        image = "again"
        cell = [8, 5]
        grid = [5, 2]
        group = "U@one"
        [sheets.parts]
        sprite = "U@two/1"
        origin = [5, 4]
        cell = [3, 2]
        grid = [2, 1]
        group = "Part"
        [sets."Anim@two"]
        frames = ["U@two/0", "U@one/9"]
        "#;
    /// The same art as one description.
    const WHOLE: &str = r#"
        [images.u1]
        source = "uuu"
        [sheets.u1]
        image = "u1"
        cell = [8, 5]
        grid = [5, 2]
        group = "U@one"
        [sets."Anim@one"]
        frames = ["U@one/0", "U@one/3", "s/2"]
        [images.u2]
        source = "vvvvv"
        [sheets.u2]
        image = "u2"
        cell = [20, 20]
        grid = [2, 1]
        group = "U@two"
        [sets."Anim@two"]
        frames = ["U@two/0", "U@one/9"]
        "#;

    /// Each sprite's pixels by name.
    fn sprite_pixels(c: &Compiled, name: &str) -> Vec<[u8; 4]> {
        let s = c.atlas.sprites[c.sprite(name).unwrap().0 as usize];
        let page = &c.atlas.pages[s.page as usize];
        (0..s.height).flat_map(|y| (0..s.width).map(move |x| page.get(s.atlas_x + x, s.atlas_y + y))).collect()
    }

    fn frame_names(c: &Compiled, set: &str) -> Vec<String> {
        let (id, _) = c.set(set).unwrap();
        (0..).map_while(|i| c.sets.frame(id, i)).map(|s| c.sprite_names[s.0 as usize].clone()).collect()
    }

    #[test]
    fn growing_art_has_pages_for_the_largest_sprite_it_may_grow_by() {
        // The art may grow by a picture of a 3000-pixel screen: pages for it
        // from the start, as far as the device takes them.
        let base = Description::parse(&format!("largest = 3000\n{BASE}")).unwrap();
        assert_eq!(base.largest_sprite(), 3000);
        let (grown, _) = base.compile_growing(&Checker, 16384, true).unwrap();
        assert_eq!(grown.atlas.page_size, 3072);
        let (tables, _) = base.compile_growing(&Checker, 16384, false).unwrap();
        assert_eq!(tables.atlas.page_size, 3072);
        let (small, _) = base.compile_growing(&Checker, 2048, true).unwrap();
        assert_eq!(small.atlas.page_size, 2048);
        let (plain, _) = Description::parse(BASE).unwrap().compile_growing(&Checker, 16384, true).unwrap();
        assert_eq!(plain.atlas.page_size, DEFAULT_PAGE);
    }

    #[test]
    fn grown_art_keeps_what_was_there_and_draws_what_a_whole_compile_does() {
        let base = Description::parse(BASE).unwrap();
        let (mut grown, mut growing) = base.compile_growing(&Checker, 64, true).unwrap();
        let (mut tables, mut tables_growing) = base.compile_growing(&Counting(std::cell::Cell::new(0)), 64, false).unwrap();
        let before = (grown.sprite_names.clone(), grown.value("l", "s/3"), grown.value("l", "anim"), grown.set("anim").unwrap().0);
        for add in [ADD1, ADD2] {
            let g = growing.grow(&grown, add, &Checker).unwrap();
            grown.apply(&g);
            let t = tables_growing.grow(&tables, add, &Counting(std::cell::Cell::new(0))).unwrap();
            assert!(t.atlas.blits.is_empty());
            tables.apply(&t);
        }
        // What was there is as it was.
        assert_eq!(&grown.sprite_names[..before.0.len()], &before.0[..]);
        assert_eq!((grown.value("l", "s/3"), grown.value("l", "anim"), grown.set("anim").unwrap().0), (before.1, before.2, before.3));
        // The new sets at the end of the layers' tables.
        assert_eq!(grown.value("l", "U@one/0"), Some(6));
        assert_eq!(grown.value("m", "Anim@one"), Some(1));
        assert_eq!(grown.value("m", "Anim@two"), Some(2));
        // Art there already is kept, a sheet made again adds nothing.
        assert_eq!(grown.sprite_names.iter().filter(|n| n.starts_with("U@one/")).count(), 10);
        // The same pixels as one compile of everything.
        let whole = Description::parse(&format!("{BASE}{WHOLE}")).unwrap().compile(&Checker, 64).unwrap();
        for name in whole.sprite_names.iter() {
            assert_eq!(sprite_pixels(&grown, name), sprite_pixels(&whole, name), "{name}");
        }
        for set in ["anim", "Anim@one", "Anim@two"] {
            assert_eq!(frame_names(&grown, set), frame_names(&whole, set));
        }
        // Parts of a sprite: its pixels, none of their own.
        let part = sprite_pixels(&grown, "Part/1");
        let two = sprite_pixels(&grown, "U@two/1");
        assert_eq!(part, [two[4 * 20 + 8], two[4 * 20 + 9], two[4 * 20 + 10], two[5 * 20 + 8], two[5 * 20 + 9], two[5 * 20 + 10]]);
        // The tables alone agree.
        assert_eq!(tables.sprite_names, grown.sprite_names);
        let sizes = |c: &Compiled| c.atlas.sprites.iter().map(|s| (s.width, s.height, s.anchor_x, s.anchor_y)).collect::<Vec<_>>();
        assert_eq!(sizes(&tables), sizes(&grown));
        assert_eq!(tables.value("m", "Anim@two"), grown.value("m", "Anim@two"));
        // A set given again with other frames is replaced: its layer value
        // draws the new one.
        let value = grown.value("m", "Anim@one");
        let g = growing.grow(&grown, "[sets.\"Anim@one\"]\nframes = [\"s/0\", \"U@one/1\"]\n", &Checker).unwrap();
        grown.apply(&g);
        assert_eq!(frame_names(&grown, "Anim@one"), ["s/0", "U@one/1"]);
        assert_eq!(grown.value("m", "Anim@one"), value);
        let layer = grown.layers.iter().find(|l| l.name == "m").unwrap();
        assert_eq!(layer.lut[value.unwrap() as usize], grown.set("Anim@one").unwrap().0);
        // The same frames again: nothing.
        assert!(growing.grow(&grown, "[sets.\"Anim@one\"]\nframes = [\"s/0\", \"U@one/1\"]\n", &Checker).unwrap().is_empty());
        // A failed addition adds nothing; the next one numbers on.
        let n = grown.atlas.sprites.len();
        let bad = "[images.x]\nsource = \"x\"\n[sheets.x]\nimage = \"x\"\ncell = [2, 2]\ngroup = \"X\"\n[sets.\"Anim@bad\"]\nframes = [\"nothing\"]\n";
        assert!(growing.grow(&grown, bad, &Checker).is_err());
        let g = growing.grow(&grown, &bad.replace("nothing", "X/0"), &Checker).unwrap();
        assert_eq!(g.atlas.first as usize, n);
        grown.apply(&g);
        assert_eq!(grown.value("m", "Anim@bad"), Some(3));
    }


    #[test]
    fn grown_indexed_art_shares_pixels_and_keeps_the_palette_rows() {
        let base = r#"
            grid = { layout = "orthogonal", step = [8, 4] }
            [palettes.civ]
            source = "pal{v}"
            variants = [1, 22]
            [images.body]
            source = "a.flc"
            remap = [2, 6]
            [sheets.body]
            image = "body"
            all_frames = true
            cell = [8, 4]
            [[layers]]
            name = "units"
            sets = ["body/*", "Shadow@*"]
            palettes = true
        "#;
        let add = r#"
            [images.shadow]
            source = "a.flc"
            shadow = true
            [sheets.shadow]
            image = "shadow"
            all_frames = true
            cell = [8, 4]
            group = "Shadow@a"
        "#;
        let (mut grown, mut growing) = Description::parse(base).unwrap().compile_growing(&Indexed, 64, true).unwrap();
        let before = (grown.palette("civ@22"), grown.atlas.named_palettes, grown.value_in("units", "body/1", 2));
        let g = growing.grow(&grown, add, &Indexed).unwrap();
        // The shadow's indices are the body's: no pixels, a palette row of its own.
        assert!(g.atlas.blits.is_empty());
        assert_eq!(g.atlas.palettes.len(), tile_render_shared::PALETTE_SIZE as usize);
        grown.apply(&g);
        assert_eq!((grown.palette("civ@22"), grown.atlas.named_palettes, grown.value_in("units", "body/1", 2)), before);
        let whole = Description::parse(&format!("{base}{add}")).unwrap().compile(&Indexed, 64).unwrap();
        for (name, index) in [("body/2", 3), ("body/2", 22), ("Shadow@a/2", 22), ("Shadow@a/0", 9)] {
            let color = |c: &Compiled| {
                let s = c.atlas.sprites[c.sprite(name).unwrap().0 as usize];
                let page = &c.atlas.index_pages[s.page as usize];
                let i = page.get(s.atlas_x + index % 8, s.atlas_y) as u32;
                tile_render_shared::indexed_color(i, s.palette, s.remap, c.palette("civ@22").unwrap(), c.atlas.named_palettes, &c.atlas.palettes)
            };
            assert_eq!(color(&grown), color(&whole), "{name} at {index}");
        }
        assert!(grown.value("units", "Shadow@a/1").is_some());
        // A named palette added later, based on one there: its row counts
        // on past the own rows, values pick it.
        let army = "[palettes.army]\nbase = \"civ@1\"\ncopy = [[\"civ@22\", 2, 5, 2]]\n";
        let g = growing.grow(&grown, army, &Indexed).unwrap();
        grown.apply(&g);
        let whole = Description::parse(&format!("{base}{add}{army}")).unwrap().compile(&Indexed, 64).unwrap();
        let body = |c: &Compiled| c.atlas.sprites[c.sprite("body/0").unwrap().0 as usize];
        for index in [3, 5, 6, 7] {
            let color = |c: &Compiled| tile_render_shared::indexed_color(index, body(c).palette, body(c).remap, c.palette("army").unwrap(), c.atlas.named_palettes, &c.atlas.palettes);
            assert_eq!(color(&grown), color(&whole), "index {index}");
        }
        let row = grown.palette("army").unwrap();
        assert_eq!(grown.value_in("units", "body/1", row).map(|v| v >> 16), Some(row));
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
    fn fixed_resize_keeps_native_phase_instead_of_centered_nearest() {
        let image = RgbaImage::from_fn(4, 1, |x, _| [x as u8, 0, 0, 255]);
        let fixed = resize_fixed(&image, (3, 1), [87381, 65536]);
        assert_eq!((0..3).map(|x| fixed.get(x, 0)[0]).collect::<Vec<_>>(), [0, 1, 2]);
        let nearest = resize_nearest(&image, (3, 1));
        assert_eq!((0..3).map(|x| nearest.get(x, 0)[0]).collect::<Vec<_>>(), [0, 2, 3]);
    }

    #[test]
    fn fixed_resize_rejects_missing_output_size_and_out_of_cell_reads() {
        for (options, expected) in [
            ("resize_step = [65536,65536]", "resize_step requires resize"),
            ("resize = [4,4]\nresize_step = [262144,262144]", "resize_step samples outside"),
            ("resize = [4,4]\nresize_step = [0,65536]", "resize_step samples outside"),
        ] {
            let text=format!("grid = {{ layout = \"orthogonal\", step = [1,1] }}\n[images.a]\nsource = \"a\"\n[sheets.icon]\nimage = \"a\"\ncell = [4,4]\ngrid = [1,1]\n{options}\n[[layers]]\nname = \"l\"\nsets = [\"icon/0\"]");
            let error=Description::parse(&text).unwrap().compile(&Checker,256).err().expect("invalid sampler accepted").to_string();
            assert!(error.contains(expected),"{error}");
        }
    }

    #[test]
    fn a_sheet_may_resize_its_cells_nearest_pixel() {
        let text = "grid = { layout = \"orthogonal\", step = [1, 1] }\n\
            [images.a]\nsource = \"a\"\n\
            [sheets.icon]\nimage = \"a\"\norigin = [8, 2]\ncell = [8, 4]\ngrid = [1, 1]\nresize = [4, 3]\n\
            [[layers]]\nname = \"l\"\nsets = [\"icon/0\"]\n";
        let c = Description::parse(text).unwrap().compile(&Checker, 256).unwrap();
        let s = c.atlas.sprites[c.sprite_names.iter().position(|n| n == "icon/0").unwrap()];
        assert_eq!((s.width, s.height), (4, 3));
        // (Checker's pixels hold their x and y.) Each the source pixel under
        // its middle: columns 9, 11, 13, 15, rows 2, 4, 5 of the image.
        let page = &c.atlas.pages[s.page as usize];
        let got: Vec<(u8, u8)> = (0..3).flat_map(|y| (0..4).map(move |x| (x, y))).map(|(x, y)| page.get(s.atlas_x + x, s.atlas_y + y)).map(|p| (p[0], p[1])).collect();
        let want: Vec<(u8, u8)> = [2, 4, 5].iter().flat_map(|&y| [9, 11, 13, 15].map(|x| (x, y))).collect();
        assert_eq!(got, want);
        // Not of indexed art.
        let indexed = text.replace("source = \"a\"", "source = \"a.flc\"").replace("origin = [8, 2]", "origin = [0, 0]");
        assert!(Description::parse(&indexed).unwrap().compile(&Indexed, 256).is_err());
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

    /// Two 4 × 4 cursor cells at (1, 1) and (6, 1), a 1-pixel grid of grey
    /// between them; the first's hot spot (2, 3) marked in orange in the
    /// grid, the second unmarked.
    struct Marked;
    impl ImageLoader for Marked {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Ok(vec![RgbaImage::from_fn(11, 6, |x, y| match (x, y) {
                (3, 0) | (0, 4) => [228, 152, 24, 255],
                (_, 0) | (0, _) | (5, _) | (10, _) | (_, 5) => [70, 70, 70, 255],
                _ => [1, 2, 3, 255],
            })])
        }
    }

    #[test]
    fn anchor_marks_beside_a_cell_give_its_hot_spot() {
        let text = "[grid]\nlayout = \"orthogonal\"\nstep = [4, 4]\n\
            [images.c]\nsource = \"c\"\n\
            [sheets.c]\nimage = \"c\"\ncell = [4, 4]\ngrid = [2, 1]\norigin = [1, 1]\nspacing = [1, 0]\nanchor = [-1, -1]\nanchor_marks = [230, 150, 20]\n";
        let d = Description::parse(text).unwrap();
        for c in [d.compile(&Marked, 64).unwrap(), d.compile_with(&Marked, 64, false, None).unwrap()] {
            let anchor = |name: &str| {
                let s = c.atlas.sprites[c.sets.frame(c.set(name).unwrap().0, 0).unwrap().0 as usize];
                (s.anchor_x, s.anchor_y)
            };
            assert_eq!(anchor("c/0"), (-2, -3));
            // No marks: the sheet's anchor.
            assert_eq!(anchor("c/1"), (-1, -1));
        }
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
