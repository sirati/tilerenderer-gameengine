//! Complete UI command batches can precede optional append-only art pixels.
//! Keep their layout metadata and original scene, but never send future
//! numeric references to shaders bound to an older descriptor prefix.

use std::sync::Arc;
use std::collections::HashMap;
use render_desc::{Compiled, Growth};
use scene::{FreeSprite, Scene};
use tile_render::SpriteSetId;

pub(crate) struct Metadata {
    pub base: u64,
    pub through: u64,
    pub layout: Arc<Compiled>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct MetadataWork {
    pub table_copies: usize,
    pub growths_applied: usize,
}

/// Extend the privately owned future tables instead of recloning the entire
/// uploaded description for every optional-art generation. A pinned reader
/// keeps its old tables; that uncommon case gets an independent copy.
pub(crate) fn refresh_metadata<'a>(current: &mut Option<Metadata>, base: u64, drawable: &Compiled, uploaded: u64, target: u64, growths: impl Iterator<Item = (u64, &'a Growth)>) -> MetadataWork {
    let mut work = MetadataWork::default();
    if target <= uploaded { return work; }
    if current.as_ref().is_some_and(|m| m.base == base && m.through == target) { return work; }
    let extend = current.as_ref().is_some_and(|m| m.base == base && uploaded <= m.through && m.through < target);
    let start = if extend { current.as_ref().expect("extension").through } else { uploaded };
    let mut through = start;
    let suffix: Vec<_> = growths.filter(|(version, _)| {
        if *version <= through { return false; }
        through = *version;
        true
    }).take_while(|(version, _)| *version <= target).collect();
    // An incomplete target must not cause a large throwaway metadata copy
    // on every frame while the remaining producer update is still absent.
    if suffix.last().is_none_or(|(version, _)| *version != target) { return work; }
    if !extend {
        *current = Some(Metadata { base, through: uploaded, layout: Arc::new(drawable.without_pixels()) });
        work.table_copies += 1;
    }
    let metadata = current.as_mut().expect("complete target");
    if Arc::get_mut(&mut metadata.layout).is_none() {
        metadata.layout = Arc::new(metadata.layout.without_pixels());
        work.table_copies += 1;
    }
    let layout = Arc::get_mut(&mut metadata.layout).expect("privately owned tables");
    for (version, growth) in suffix {
        layout.apply(growth);
        metadata.through = version;
        work.growths_applied += 1;
    }
    work
}

#[cfg(test)]
pub(crate) fn metadata_through<'a>(drawable: &Compiled, uploaded: u64, target: u64, growths: impl Iterator<Item = (u64, &'a Growth)>) -> Option<Compiled> {
    let mut metadata = None;
    refresh_metadata(&mut metadata, uploaded, drawable, uploaded, target, growths);
    metadata.map(|m| Arc::try_unwrap(m.layout).ok().expect("unshared test tables"))
}

fn palette_available(compiled: &Compiled, palette: u32) -> bool {
    palette == 0 || compiled.atlas.named_palettes.checked_add(palette)
        .is_some_and(|end| u64::from(end) * 256 <= compiled.atlas.palettes.len() as u64)
}

fn same_palette(drawable: &Compiled, future: &Compiled, palette: u32) -> bool {
    if palette == 0 { return true; }
    if !palette_available(drawable, palette) || !palette_available(future, palette) { return false; }
    let row = |c: &Compiled| (c.atlas.named_palettes + palette - 1) as usize * 256;
    let (old, new) = (row(drawable), row(future));
    drawable.atlas.palettes[old..old + 256] == future.atlas.palettes[new..new + 256]
}

fn set_available(compiled: &Compiled, set: u32) -> bool {
    let set = SpriteSetId::from_raw(set);
    if set == SpriteSetId::EMPTY { return true; }
    let mut frame = 0;
    while let Some(id) = compiled.sets.frame(set, frame) {
        if compiled.atlas.sprites.get(id.0 as usize).is_none() { return false; }
        frame += 1;
    }
    frame != 0
}

pub(crate) fn sprite_available(compiled: &Compiled, sprite: &FreeSprite) -> bool {
    if sprite.solid.is_some() || sprite.output_rect.is_some() { return true; }
    match sprite.glyph {
        Some(glyph) => (glyph.font as usize) < compiled.fonts.len(),
        None => set_available(compiled, sprite.set) && palette_available(compiled, sprite.palette),
    }
}

fn same_set(drawable: &Compiled, future: &Compiled, set: u32) -> bool {
    if !set_available(drawable, set) { return false; }
    let set = SpriteSetId::from_raw(set);
    let mut frame = 0;
    loop {
        match (drawable.sets.frame(set, frame), future.sets.frame(set, frame)) {
            (None, None) => return true,
            (Some(old), Some(new)) if old == new && drawable.atlas.sprites.get(old.0 as usize) == future.atlas.sprites.get(new.0 as usize) => {
                let descriptor = &drawable.atlas.sprites[old.0 as usize];
                if descriptor.palette != 0 {
                    let at = descriptor.palette as usize * 256;
                    if drawable.atlas.palettes.get(at..at + 256) != future.atlas.palettes.get(at..at + 256) { return false; }
                }
                frame += 1;
            },
            _ => return false,
        }
    }
}

fn no_op(sprite: &mut FreeSprite) {
    // Preserve order, positions and cardinality for spans, lifetimes and UI
    // hit rectangles. EMPTY is a known valid descriptor, never a future ID.
    sprite.set = SpriteSetId::EMPTY.raw();
    sprite.glyph = None;
    sprite.solid = None;
    sprite.palette = 0;
    sprite.clip = Some([0; 4]);
}

fn split(layer: &tile_render::LayerSpec, raw: u32) -> (u32, u32) {
    let shift = layer.palette_shift();
    if shift == 0 || shift >= 32 { (raw, 0) } else { (raw & ((1 << shift) - 1), raw >> shift) }
}

fn cell_available(drawable: &Compiled, future: &Compiled, layer: usize, raw: u32) -> bool {
    let (Some(old), Some(new)) = (drawable.layers.get(layer), future.layers.get(layer)) else { return false; };
    let (index, palette) = split(new, raw);
    if split(old, raw) != (index, palette) || !same_palette(drawable, future, palette) { return false; }
    let (Some(old_set), Some(new_set)) = (old.lut.get(index as usize), new.lut.get(index as usize)) else { return false; };
    old_set == new_set && same_set(drawable, future, new_set.raw())
}

#[cfg(test)]
pub(crate) fn drawable_scene(drawable: &Compiled, future: &Compiled, scene: &Scene) -> Scene {
    let mut result = scene.clone();
    let mut sets = HashMap::new();
    let mut palettes = HashMap::new();
    let mut sanitize = |sprite: &mut FreeSprite| {
        if sprite.solid.is_some() { return; }
        let available = if sprite.glyph.is_some() { sprite_available(drawable, sprite) } else {
            *sets.entry(sprite.set).or_insert_with(|| same_set(drawable, future, sprite.set))
                && *palettes.entry(sprite.palette).or_insert_with(|| same_palette(drawable, future, sprite.palette))
        };
        if !available { no_op(sprite); }
    };
    for sprite in &mut result.sprites { sanitize(sprite); }
    for view in &mut result.views { for sprite in &mut view.overlay { sanitize(sprite); } }
    for look in &mut result.looks { for view in &mut look.views { for (_, sprites) in &mut view.states { for sprite in sprites { sanitize(sprite); } } } }
    let mut cells_available = HashMap::new();
    for (layer, cells) in result.layers.iter_mut().enumerate() {
        for raw in &mut cells.values { if !*cells_available.entry((layer, *raw)).or_insert_with(|| cell_available(drawable, future, layer, *raw)) { *raw = 0; } }
    }
    for slot in &mut result.slots {
        for raw in &mut slot.cells.values { if !*cells_available.entry((slot.layer as usize, *raw)).or_insert_with(|| cell_available(drawable, future, slot.layer as usize, *raw)) { *raw = 0; } }
    }
    result
}

/// Retained sanitized source commands. These contain no rendered pixels.
pub(crate) struct DrawableScene {
    sequence: u64,
    drawable_version: u64,
    command_version: u64,
    scene: Arc<Scene>,
    cells: HashMap<(usize, u32), bool>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct DrawableWork {
    pub full_cell_copies: usize,
    pub changed_cell_copies: usize,
    pub cell_mapping_checks: usize,
}

pub(crate) fn refresh_drawable_scene(current: &mut Option<DrawableScene>, drawable_version: u64, drawable: &Compiled, future: &Compiled, source: &Scene, previous_sequence: u64, changes: &scene::Changes) -> (Arc<Scene>, DrawableWork) {
    let mut work = DrawableWork::default();
    if current.as_ref().is_some_and(|c| c.sequence == source.seq && c.drawable_version == drawable_version && c.command_version == source.assets) {
        return (current.as_ref().expect("matched").scene.clone(), work);
    }
    let initial = current.is_none();
    if initial {
        *current = Some(DrawableScene { sequence: source.seq, drawable_version, command_version: source.assets, scene: Arc::new(source.clone()), cells: HashMap::new() });
        work.full_cell_copies = source.layers.iter().map(|l| l.values.len()).chain(source.slots.iter().map(|s| s.cells.values.len())).sum();
    }
    let cached = current.as_mut().expect("initialized");
    // A skipped/minimized preparation can adopt scenes without updating
    // this derived snapshot. Its next refresh must then use the full source,
    // not a delta whose predecessor this snapshot never observed.
    let reset = scene::Changes { world: true, sprites: true, views: true, slots: true, ..Default::default() };
    let changes = if cached.sequence != previous_sequence && cached.sequence != source.seq { &reset } else { changes };
    let mapping_changed = cached.drawable_version != drawable_version || cached.command_version != source.assets;
    let full_cells = initial || changes.world || cached.scene.shape != source.shape || cached.scene.layers.len() != source.layers.len();
    let mut changed_layers = std::collections::HashSet::new();
    if full_cells { cached.cells.clear(); }
    else if mapping_changed {
        for (&(layer, raw), available) in &mut cached.cells {
            let now = cell_available(drawable, future, layer, raw);
            work.cell_mapping_checks += 1;
            if now != *available { changed_layers.insert(layer); *available = now; }
        }
    }
    if Arc::strong_count(&cached.scene) != 1 {
        work.full_cell_copies += cached.scene.layers.iter().map(|l| l.values.len()).chain(cached.scene.slots.iter().map(|s| s.cells.values.len())).sum::<usize>();
    }
    let result = Arc::make_mut(&mut cached.scene);
    if !initial {
        let delta = crate::scene_snapshot::update(result, source, changes);
        work.full_cell_copies += delta.full_cell_copies;
        work.changed_cell_copies += delta.changed_cell_copies;
    }
    // Changed descriptor prefixes can make a previously omitted cell valid
    // without a game-cell update. Restore only layers whose used mappings
    // actually changed, then sanitize them against the new prefixes.
    for &layer in &changed_layers {
        if let (Some(to), Some(from)) = (result.layers.get_mut(layer), source.layers.get(layer)) {
            to.values.clone_from(&from.values); work.full_cell_copies += from.values.len();
        }
        for to in result.slots.iter_mut().filter(|s| s.layer as usize == layer) {
            if let Some(from) = source.slots.iter().find(|s| s.id == to.id) {
                to.cells.values.clone_from(&from.cells.values); work.full_cell_copies += from.cells.values.len();
            }
        }
    }
    let mut available = |layer: usize, raw: u32| *cached.cells.entry((layer, raw)).or_insert_with(|| {
        work.cell_mapping_checks += 1;
        cell_available(drawable, future, layer, raw)
    });
    for (layer, cells) in result.layers.iter_mut().enumerate() {
        if full_cells || changed_layers.contains(&layer) {
            for raw in &mut cells.values { if !available(layer, *raw) { *raw = 0; } }
        }
    }
    if !full_cells {
        for &(layer, cell) in &changes.cells {
            if changed_layers.contains(&(layer as usize)) { continue; }
            if let Some(raw) = result.layers.get_mut(layer as usize).and_then(|l| l.values.get_mut(cell as usize)) {
                if !available(layer as usize, *raw) { *raw = 0; }
            }
        }
    }
    for slot in &mut result.slots {
        if full_cells || changes.slots || changed_layers.contains(&(slot.layer as usize)) {
            for raw in &mut slot.cells.values { if !available(slot.layer as usize, *raw) { *raw = 0; } }
        } else {
            for &(_, cell) in changes.slot_cells.iter().filter(|(id, _)| *id == slot.id) {
                if let Some(raw) = slot.cells.values.get_mut(cell as usize) { if !available(slot.layer as usize, *raw) { *raw = 0; } }
            }
        }
    }
    // Art can become available even if these sprite lists did not change.
    // Restore their original IDs before rechecking a new descriptor prefix.
    if mapping_changed {
        result.sprites.clone_from(&source.sprites);
        result.views.clone_from(&source.views);
    }
    let mut sets = HashMap::new();
    let mut palettes = HashMap::new();
    let mut sanitize = |sprite: &mut FreeSprite| {
        if sprite.solid.is_some() { return; }
        let available = if sprite.glyph.is_some() { sprite_available(drawable, sprite) } else {
            *sets.entry(sprite.set).or_insert_with(|| same_set(drawable, future, sprite.set))
                && *palettes.entry(sprite.palette).or_insert_with(|| same_palette(drawable, future, sprite.palette))
        };
        if !available { no_op(sprite); }
    };
    for sprite in &mut result.sprites { sanitize(sprite); }
    for view in &mut result.views { for sprite in &mut view.overlay { sanitize(sprite); } }
    // snapshot::update always refreshes look metadata from the source.
    for look in &mut result.looks { for view in &mut look.views { for (_, sprites) in &mut view.states { for sprite in sprites { sanitize(sprite); } } } }
    cached.sequence = source.seq;
    cached.drawable_version = drawable_version;
    cached.command_version = source.assets;
    (cached.scene.clone(), work)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Images;
    impl render_desc::ImageLoader for Images {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Ok(vec![tile_render::RgbaImage::from_fn(8, 8, |_, _| [255, 0, 0, 255])])
        }
    }
    fn fixture() -> (Compiled, Growth, Compiled) {
        let description = render_desc::Description::parse("grid = {layout='orthogonal',step=[8,8]}\n[images.a]\nsource='a'\n[sheets.old]\nimage='a'\ncell=[8,8]\n[[layers]]\nname='base'\nsets=['old/*']\n").unwrap();
        let (old, mut growing) = description.compile_growing(&Images, 16, true).unwrap();
        let growth = growing.grow(&old, "[images.b]\nsource='b'\n[sheets.new]\nimage='b'\ncell=[8,8]\n", &Images).unwrap();
        let mut new = old.without_pixels();
        new.apply(&growth);
        (old, growth, new)
    }

    fn assert_incremental_proof(old: &Compiled, growth: &Growth) -> bool {
        let mut new = old.without_pixels();
        let incremental = new.apply_with_prefix_check(growth);
        assert_eq!(incremental, preserves_render_prefix(old, &new),
            "incremental proof must match the complete numeric/name/font proof");
        incremental
    }

    #[test]
    fn incremental_growth_proof_matches_full_scan_for_named_sets_and_aliases() {
        for name in ["old/0", "empty", "new"] {
            for frames in ["[]", "['old/0']", "['old/0','old/0']"] {
                let description = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}\n[images.a]\nsource='a'\n[sheets.old]\nimage='a'\ncell=[8,8]\n[sets.empty]\nframes=[]\n").unwrap();
                let (old, mut growing) = description.compile_growing(&Images, 16, true).unwrap();
                let growth = growing.grow(&old, &format!("[sets.'{name}']\nframes={frames}\n"), &Images).unwrap();
                assert_incremental_proof(&old, &growth);
            }
        }
        let (old, growth, _) = fixture();
        assert!(assert_incremental_proof(&old, &growth));
        let mut alias = Growth::default();
        alias.atlas.first = old.atlas.sprites.len() as u32;
        alias.atlas.sprites.push(old.atlas.sprites[0]);
        alias.sprite_names.push(old.sprite_names[0].clone());
        assert!(!assert_incremental_proof(&old, &alias));
    }

    #[test]
    fn incremental_growth_proof_checks_shared_empty_set_blend_changes() {
        let description = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}\n[sets.empty]\nframes=[]\n").unwrap();
        let (old, mut growing) = description.compile_growing(&Images, 16, true).unwrap();
        let growth = growing.grow(&old, "[sets.new]\nframes=[]\nblend={scale=16}\n", &Images).unwrap();
        assert!(!assert_incremental_proof(&old, &growth), "empty set IDs share their blend entry");
    }

    #[test]
    fn incremental_growth_proof_detects_palette_rewrites_of_old_descriptors() {
        let (mut old, _, _) = fixture();
        old.atlas.palettes = vec![0; 256];
        old.atlas.sprites[0].palette = 1;
        let mut growth = Growth::default();
        growth.atlas.first = old.atlas.sprites.len() as u32;
        growth.atlas.palettes = vec![u32::MAX; 256];
        assert!(!assert_incremental_proof(&old, &growth));
        old.atlas.sprites[0].palette = 0;
        assert!(assert_incremental_proof(&old, &growth));
    }

    #[test]
    fn incremental_growth_proof_matches_full_scan_through_long_optional_art_stream() {
        let (mut old, _, _) = fixture();
        for chunk in 0..512 {
            let mut growth = Growth::default();
            growth.atlas.first = old.atlas.sprites.len() as u32;
            growth.atlas.sprites = vec![old.atlas.sprites[0]; 8];
            growth.sprite_names = (0..8).map(|i| format!("stream/{chunk}/{i}")).collect();
            if chunk % 29 == 0 { growth.sprite_names[0] = old.sprite_names[0].clone(); }
            assert_incremental_proof(&old, &growth);
            old.apply(&growth);
        }
    }

    #[test]
    fn compatible_append_preserves_old_frames_even_when_future_layers_pick_new_sets() {
        let (old, _, mut future) = fixture();
        assert!(preserves_render_prefix(&old, &future));
        future.layers[0].lut[1] = future.set("new/0").unwrap().0;
        assert!(preserves_render_prefix(&old, &future), "submitted world owns the old layer lookup table");
    }

    #[test]
    fn changed_descriptor_palette_set_or_patch_name_invalidates_old_frame_prefix() {
        let (old, _, future) = fixture();
        let mut changed = future.without_pixels();
        changed.atlas.sprites[0].anchor_x += 1;
        assert!(!preserves_render_prefix(&old, &changed));
        let mut changed = future.without_pixels();
        changed.atlas.palettes[0] ^= 1;
        assert!(!preserves_render_prefix(&old, &changed));
        let mut changed = future.without_pixels();
        changed.atlas.named_palettes += 1;
        assert!(!preserves_render_prefix(&old, &changed));
        let mut changed = future.without_pixels();
        changed.sets = tile_render::SpriteSets::default();
        assert!(!preserves_render_prefix(&old, &changed));
        let mut changed = old.without_pixels();
        let mut alias = Growth::default();
        alias.atlas.first = old.atlas.sprites.len() as u32;
        alias.atlas.sprites.push(old.atlas.sprites[0]);
        alias.sprite_names.push(old.sprite_names[0].clone());
        changed.apply(&alias);
        assert!(changed.sprite_names.starts_with(&old.sprite_names));
        assert_ne!(old.sprite(&old.sprite_names[0]), changed.sprite(&old.sprite_names[0]));
        assert!(!preserves_render_prefix(&old, &changed), "patch names must still resolve to their old IDs");
    }

    #[test]
    fn future_commands_keep_layout_and_order_but_never_alias_a_prefix_set_or_font() {
        let (old, _, future) = fixture();
        let old_set = old.set("old/0").unwrap().0.raw();
        let future_set = future.set("new/0").unwrap().0.raw();
        let missing = FreeSprite { set: future_set, x: 40, y: 50, ..Default::default() };
        let glyph = FreeSprite { set: old_set, glyph: Some(scene::Glyph { font: u32::MAX, ch: 'x', em64: 640, advance: 10, bold: false, italic: false, key: None, bar: None }), x: 60, ..Default::default() };
        let scene = Scene { sprites: vec![FreeSprite { set: old_set, ..Default::default() }, missing, glyph], ..Default::default() };
        let prefix = drawable_scene(&old, &future, &scene);
        assert_eq!(prefix.sprites.len(), 3);
        assert_eq!(prefix.sprites[0], scene.sprites[0]);
        for (original, no_op) in scene.sprites[1..].iter().zip(&prefix.sprites[1..]) {
            assert_eq!((no_op.x, no_op.y), (original.x, original.y));
            assert_eq!(no_op.set, SpriteSetId::EMPTY.raw());
            assert_eq!(no_op.clip, Some([0; 4]));
            assert_eq!(no_op.glyph, None);
        }
        assert_eq!(scene.sprites[1], missing, "source commands must survive art arrival untouched");
        assert_eq!(drawable_scene(&future, &future, &scene).sprites[1], missing);
    }

    #[test]
    fn future_layer_replacement_is_not_rendered_with_the_old_value_mapping() {
        let (old, _, mut future) = fixture();
        future.layers[0].lut[1] = future.set("new/0").unwrap().0;
        let scene = Scene { layers: vec![scene::LayerCells { values: vec![1], ..Default::default() }], ..Default::default() };
        assert_eq!(drawable_scene(&old, &future, &scene).layers[0].values, [0]);
        assert_eq!(scene.layers[0].values, [1]);
        assert_eq!(drawable_scene(&future, &future, &scene).layers[0].values, [1]);
    }

    #[test]
    fn metadata_requires_the_exact_complete_growth_without_pixel_residency() {
        let (old, growth, future) = fixture();
        let metadata = metadata_through(&old, 7, 9, [(9, &growth)].into_iter()).unwrap();
        assert_eq!(metadata.sprite_names, future.sprite_names);
        assert!(metadata.atlas.pages.is_empty());
        assert!(metadata_through(&old, 7, 10, [(9, &growth)].into_iter()).is_none());
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    struct Images;
    impl render_desc::ImageLoader for Images {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Ok(vec![tile_render::RgbaImage::from_fn(8, 8, |_, _| [255, 0, 0, 255])])
        }
    }
    #[test]
    fn gpu_complete_groups_move_together_while_future_ids_remain_no_ops() {
        let description = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}\n[images.a]\nsource='a'\n[sheets.old]\nimage='a'\ncell=[8,8]\n").unwrap();
        let (old, mut growing) = description.compile_growing(&Images, 16, true).unwrap();
        let growth = growing.grow(&old, "[images.b]\nsource='b'\n[sheets.new]\nimage='b'\ncell=[8,8]\n", &Images).unwrap();
        let mut future = old.without_pixels();
        future.apply(&growth);
        let known = old.set("old/0").unwrap().0.raw();
        let unknown = future.set("new/0").unwrap().0.raw();
        let gpu = tile_render::Headless::new().expect("RADV test device");
        let mut renderer = tile_render::TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        renderer.set_atlas(&gpu.device, &gpu.queue, &old.atlas);
        let camera = tile_render::Camera::new((64, 16));
        let clear = [17, 29, 43, 255];
        for (x, rgba) in [(0, [0, 120, 0, 255]), (12, [0, 0, 140, 255]), (0, [0, 120, 0, 255])] {
            let scene = Scene { sprites: vec![
                FreeSprite { solid: Some(scene::SolidRect { width: 12, height: 12, rgba }), set: u32::MAX, palette: u32::MAX, x, screen: true, ..Default::default() },
                FreeSprite { set: known, x: x + 1, y: 1, screen: true, ..Default::default() },
                FreeSprite { set: unknown, x: 32, screen: true, ..Default::default() },
                FreeSprite { set: known, x: 44, screen: true, glyph: Some(scene::Glyph { font: u32::MAX, ch: 'x', em64: 640, advance: 8, bold: false, italic: false, key: None, bar: None }), ..Default::default() },
            ], ..Default::default() };
            let prefix = drawable_scene(&old, &future, &scene);
            renderer.set_world(&gpu.device, &crate::scene_world(&old, &prefix));
            let pixels = gpu.render(&mut renderer, &camera, 0, clear);
            for y in 0..16 { for px in 0..64 {
                let expected = if px >= x + 1 && px < x + 9 && (1..9).contains(&y) { [255, 0, 0, 255] }
                    else if px >= x && px < x + 12 && y < 12 { rgba } else { clear };
                assert_eq!(pixels.get(px as u32, y), expected, "group x={x}, pixel ({px},{y})");
            } }
        }
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;
    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no fixture images".into()) }
    }
    #[test]
    fn equal_palette_numbers_with_different_row_bases_never_alias_colours() {
        let mut old = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}").unwrap().compile(&NoImages, 16).unwrap();
        old.atlas.named_palettes = 1;
        old.atlas.palettes = vec![0; 3 * 256];
        old.atlas.palettes[256..512].fill(0xff0000ff);
        let mut future = old.without_pixels();
        future.atlas.named_palettes = 2;
        future.atlas.palettes[512..768].fill(0xffff0000);
        assert!(palette_available(&old, 1) && palette_available(&future, 1));
        assert!(!same_palette(&old, &future, 1));
        let scene = Scene { sprites: vec![FreeSprite { palette: 1, ..Default::default() }], ..Default::default() };
        assert_eq!(drawable_scene(&old, &future, &scene).sprites[0].clip, Some([0; 4]));
    }
}

/// Whether a frame resolved against `old` can still use its numeric IDs
/// after committing `new`. Its world owns the old grid, layer lookup tables,
/// sets and font outlines. The renderer still supplies sprite descriptors,
/// atlas palettes, and name resolution for live patches.
pub(crate) fn preserves_render_prefix(old: &Compiled, new: &Compiled) -> bool {
    if old.atlas.page_size != new.atlas.page_size
        || old.atlas.named_palettes != new.atlas.named_palettes
        || !new.atlas.sprites.starts_with(&old.atlas.sprites)
        || !new.atlas.palettes.starts_with(&old.atlas.palettes)
        || !old.sets.is_prefix_of(&new.sets)
        || !new.sprite_names.starts_with(&old.sprite_names)
        || new.fonts.len() < old.fonts.len()
    { return false; }
    for (old, new) in old.fonts.iter().zip(&new.fonts) {
        if old.0 != new.0 || !Arc::ptr_eq(&old.1, &new.1) { return false; }
    }
    for name in old.sprite_names.iter().filter(|name| !name.is_empty()) {
        if old.sprite(name) != new.sprite(name) { return false; }
    }
    for (name, _) in old.sets_with_prefix("") {
        if old.set(name) != new.set(name) { return false; }

    }
    true
}

#[cfg(test)]
mod incremental_metadata_tests {
    use super::*;
    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no fixture images".into()) }
    }
    fn base() -> Compiled {
        render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}").unwrap().compile(&NoImages, 16).unwrap()
    }
    fn growth(compiled: &Compiled, chunk: usize) -> Growth {
        let mut growth = Growth::default();
        growth.atlas.first = compiled.atlas.sprites.len() as u32;
        growth.atlas.sprites = vec![compiled.atlas.sprites.first().copied().unwrap_or_default(); 8];
        growth.sprite_names = (0..8).map(|i| format!("stream/{chunk}/{i}")).collect();
        growth
    }
    #[test]
    fn growing_target_reuses_owned_tables_and_matches_full_reconstruction() {
        let base = base();
        let mut expected = base.without_pixels();
        let mut current = None;
        let mut copies = 0;
        for i in 0..256 {
            let growth = growth(&expected, i);
            expected.apply(&growth);
            let work = refresh_metadata(&mut current, 1, &base, 1, i as u64 + 2, [(i as u64 + 2, &growth)].into_iter());
            copies += work.table_copies;
            assert_eq!(work.growths_applied, 1);
            let actual = &current.as_ref().unwrap().layout;
            assert_eq!(actual.sprite_names, expected.sprite_names);
            assert!(preserves_render_prefix(actual, &expected) && preserves_render_prefix(&expected, actual));
            for name in &expected.sprite_names { assert_eq!(actual.sprite(name), expected.sprite(name)); }
        }
        assert_eq!(copies, 1, "one base copy, not one entire copy per target");
    }
    #[test]
    fn incomplete_or_duplicate_targets_do_not_copy_or_apply_twice() {
        let base = base();
        let growth = growth(&base, 0);
        let mut current = None;
        assert_eq!(refresh_metadata(&mut current, 1, &base, 1, 3, [(2, &growth)].into_iter()), MetadataWork::default());
        assert!(current.is_none());
        let work = refresh_metadata(&mut current, 1, &base, 1, 2, [(2, &growth), (2, &growth)].into_iter());
        assert_eq!(work, MetadataWork { table_copies: 1, growths_applied: 1 });
        assert_eq!(refresh_metadata(&mut current, 1, &base, 1, 2, [(2, &growth)].into_iter()), MetadataWork::default());
        assert_eq!(current.unwrap().layout.atlas.sprites.len(), base.atlas.sprites.len() + 8);
    }
    #[test]
    fn pinned_metadata_reader_and_whole_replacement_keep_independent_id_spaces() {
        let base = base();
        let first = growth(&base, 0);
        let mut current = None;
        refresh_metadata(&mut current, 1, &base, 1, 2, [(2, &first)].into_iter());
        let pinned = current.as_ref().unwrap().layout.clone();
        let second = growth(&pinned, 1);
        let work = refresh_metadata(&mut current, 1, &base, 1, 3, [(3, &second)].into_iter());
        assert_eq!(work.table_copies, 1);
        assert_eq!(pinned.sprite_names.len(), base.sprite_names.len() + 8);
        assert_eq!(current.as_ref().unwrap().layout.sprite_names.len(), base.sprite_names.len() + 16);
        let replacement = growth(&base, 17);
        let work = refresh_metadata(&mut current, 10, &base, 10, 11, [(11, &replacement)].into_iter());
        assert_eq!(work.table_copies, 1);
        assert_eq!(current.as_ref().unwrap().base, 10);
        assert_eq!(current.as_ref().unwrap().layout.sprite_names.len(), base.sprite_names.len() + 8);
        assert!(current.as_ref().unwrap().layout.sprite("stream/0/0").is_none());
        assert!(pinned.sprite("stream/0/0").is_some());
    }
}

#[cfg(test)]
mod incremental_drawable_tests {
    use super::*;
    use scene::{Changes, LayerCells, LayerSlot, WorldShape};
    struct Images;
    impl render_desc::ImageLoader for Images {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Ok(vec![tile_render::RgbaImage::from_fn(8, 8, |_, _| [255, 0, 0, 255])])
        }
    }
    fn fixture() -> (Compiled, Compiled, Scene) {
        let desc = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}\n[images.a]\nsource='a'\n[sheets.old]\nimage='a'\ncell=[8,8]\n[[layers]]\nname='base'\nsets=['old/*']\n").unwrap();
        let (old, mut growing) = desc.compile_growing(&Images, 16, true).unwrap();
        let growth = growing.grow(&old, "[images.b]\nsource='b'\n[sheets.new]\nimage='b'\ncell=[8,8]\n", &Images).unwrap();
        let mut future = old.without_pixels(); future.apply(&growth);
        let known = old.value("base", "old/0").unwrap();
        let unknown = future.layers[0].lut.len() as u32;
        let new_set = future.set("new/0").unwrap().0;
        future.layers[0].lut.push(new_set);
        let cells = LayerCells { values: (0..65536).map(|i| if i % 2 == 0 { known } else { unknown }).collect(), phases: Some(vec![0; 65536]), offsets: Some(vec![(0, 0); 65536]) };
        let source = Scene { seq: 1, assets: 2, shape: WorldShape { cols: 256, rows: 256, ..Default::default() }, layers: vec![cells.clone()], slots: vec![LayerSlot { id: 9, layer: 0, cells }], sprites: vec![FreeSprite { set: future.set("new/0").unwrap().0.raw(), x: 17, ..Default::default() }], ..Default::default() };
        (old, future, source)
    }
    #[test]
    fn delta_sanitization_matches_full_oracle_without_recopied_world_cells() {
        let (old, future, mut source) = fixture();
        let mut cached = None;
        let (first, work) = refresh_drawable_scene(&mut cached, 1, &old, &future, &source, 0, &Changes { world: true, ..Default::default() });
        assert_eq!(*first, drawable_scene(&old, &future, &source));
        assert_eq!(work.full_cell_copies, 131072);
        let pointer = first.layers[0].values.as_ptr(); drop(first);
        for i in 0..128 {
            let previous = source.seq; source.seq += 1; source.tick += 1;
            source.layers[0].values[i] ^= 1;
            source.layers[0].phases.as_mut().unwrap()[i] = i as u8;
            source.layers[0].offsets.as_mut().unwrap()[i] = (-3, i as i16);
            source.slots[0].cells.values[i] ^= 1;
            source.sprites[0].x += 1;
            let changes = Changes { cells: vec![(0, i as u32)], slot_cells: vec![(9, i as u32)], sprites: true, ..Default::default() };
            let (actual, work) = refresh_drawable_scene(&mut cached, 1, &old, &future, &source, previous, &changes);
            assert_eq!(*actual, drawable_scene(&old, &future, &source), "update {i}");
            assert_eq!(actual.layers[0].values.as_ptr(), pointer);
            assert_eq!(work.full_cell_copies, 0);
            assert_eq!(work.changed_cell_copies, 2);
        }
    }
    #[test]
    fn newly_available_prefix_repairs_omitted_cells_and_preserves_pinned_reader() {
        let (old, future, source) = fixture();
        let mut cached = None;
        let (pinned, _) = refresh_drawable_scene(&mut cached, 1, &old, &future, &source, 0, &Changes { world: true, ..Default::default() });
        assert_eq!(pinned.layers[0].values[1], 0);
        let (ready, work) = refresh_drawable_scene(&mut cached, 2, &future, &future, &source, source.seq, &Changes::default());
        assert_eq!(*ready, source);
        assert!(work.cell_mapping_checks <= 2);
        assert_eq!(pinned.layers[0].values[1], 0);
        assert_eq!(pinned.sprites[0].clip, Some([0; 4]));
        assert_ne!(ready.sprites[0].clip, Some([0; 4]));
    }
    #[test]
    fn optional_generation_checks_used_mappings_once_and_skipped_predecessor_rebuilds() {
        let (old, future, mut source) = fixture();
        let mut cached = None;
        drop(refresh_drawable_scene(&mut cached, 1, &old, &future, &source, 0, &Changes { world: true, ..Default::default() }));
        let previous = source.seq; source.seq += 1; source.assets += 1;
        let (actual, work) = refresh_drawable_scene(&mut cached, 1, &old, &future, &source, previous, &Changes::default());
        assert_eq!(*actual, drawable_scene(&old, &future, &source));
        assert_eq!(work.full_cell_copies, 0);
        assert_eq!(work.cell_mapping_checks, 2);
        drop(actual);
        source.seq += 3; source.layers[0].values[39] = 0; source.sprites[0].x += 7;
        let (actual, work) = refresh_drawable_scene(&mut cached, 1, &old, &future, &source, source.seq - 1, &Changes::default());
        assert_eq!(*actual, drawable_scene(&old, &future, &source));
        assert_eq!(work.full_cell_copies, 131072);
    }
}
