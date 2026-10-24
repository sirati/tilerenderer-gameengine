//! Retain independently owned scene data and copy only changed cell records.
//! This is source metadata, not a retained rendered picture.

use scene::{Changes, Scene};

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Work {
    pub full_cell_copies: usize,
    pub changed_cell_copies: usize,
}

pub(crate) fn update(shown: &mut Scene, latest: &Scene, changes: &Changes) -> Work {
    // Keep this exhaustive so a new Scene control field cannot be silently
    // omitted from an adopted immutable snapshot.
    let Scene { seq, tick, shape, layers, slots, bindings, sprites, sprite_lifetimes, motions,
        views, zooms, assets, focus, focus_motion, camera_focus, camera_arrival, map_wheel_step, minimap, patches, settings, settings_preview,
        interface, interactables, cursor, cursor_at, screen_size, looks } = latest;
    let mut work = Work::default();
    if changes.world || shown.shape != *shape || shown.layers.len() != layers.len() {
        shown.layers.clone_from(layers);
        work.full_cell_copies += layers.iter().map(|l| l.values.len()).sum::<usize>();
    } else {
        for &(layer, cell) in &changes.cells {
            let (Some(to), Some(from)) = (shown.layers.get_mut(layer as usize), layers.get(layer as usize)) else { continue };
            let i = cell as usize;
            if let (Some(to), Some(from)) = (to.values.get_mut(i), from.values.get(i)) { *to = *from; }
            if let (Some(to), Some(from)) = (&mut to.phases, &from.phases) {
                if let (Some(to), Some(from)) = (to.get_mut(i), from.get(i)) { *to = *from; }
            }
            if let (Some(to), Some(from)) = (&mut to.offsets, &from.offsets) {
                if let (Some(to), Some(from)) = (to.get_mut(i), from.get(i)) { *to = *from; }
            }
            work.changed_cell_copies += 1;
        }
    }
    if changes.world || changes.slots || shown.seq == 0 {
        shown.slots.clone_from(slots);
        shown.bindings.clone_from(bindings);
        work.full_cell_copies += slots.iter().map(|s| s.cells.values.len()).sum::<usize>();
    } else {
        for &(id, cell) in &changes.slot_cells {
            let (Some(to), Some(from)) = (shown.slots.iter_mut().find(|s| s.id == id), slots.iter().find(|s| s.id == id)) else { continue };
            if let (Some(to), Some(from)) = (to.cells.values.get_mut(cell as usize), from.cells.values.get(cell as usize)) { *to = *from; }
            work.changed_cell_copies += 1;
        }
    }
    if changes.sprites || changes.world || shown.seq == 0 {
        shown.sprites.clone_from(sprites);
        shown.sprite_lifetimes.clone_from(sprite_lifetimes);
    }
    // (Small, and their ranges are only meaningful with the latest sprites.)
    shown.motions.clone_from(motions);
    if changes.views || changes.world || shown.seq == 0 { shown.views.clone_from(views); }
    shown.seq = *seq;
    shown.tick = *tick;
    shown.shape = *shape;
    shown.zooms.clone_from(zooms);
    shown.assets = *assets;
    shown.focus = *focus;
    shown.focus_motion = *focus_motion;
    shown.camera_focus = *camera_focus;
    shown.camera_arrival = *camera_arrival;
    shown.map_wheel_step = *map_wheel_step;
    shown.minimap = *minimap;
    shown.patches.clone_from(patches);
    shown.settings.clone_from(settings);
    shown.settings_preview = *settings_preview;
    shown.interface.clone_from(interface);
    shown.interactables.clone_from(interactables);
    shown.cursor = *cursor;
    shown.cursor_at = *cursor_at;
    shown.screen_size = *screen_size;
    shown.looks.clone_from(looks);
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::{LayerCells, LayerSlot, Update, WorldShape};
    use std::time::Duration;

    #[test]
    fn coalesced_updates_match_full_snapshots_without_copying_unchanged_world_arrays() {
        let (driver, mut display) = scene::connect();
        let mut shown = Scene::default();
        let cells = LayerCells { values: vec![1; 65536], phases: Some(vec![0; 65536]), offsets: Some(vec![(0, 0); 65536]) };
        let batches = vec![
            vec![Update::World { shape: WorldShape { cols: 256, rows: 256, ..Default::default() }, layers: vec![cells.clone()] }, Update::Slot { id: 9, slot: Some((0, cells)) }],
            vec![Update::Tick(13), Update::Assets(7), Update::Focus(Some((19, 23))), Update::Cursor(scene::Cursor::Default),
                Update::Zooms(vec![(3, 2)]), Update::MapWheelStep((24, 16)), Update::CameraFocus(Some(scene::CameraFocus { serial: 7, center: (11, 17) })),
                Update::ScreenSize((800, 600)), Update::Settings(Default::default()),
                Update::Interactables(vec![scene::Interactable { rect: [1, 2, 3, 4], ..Default::default() }]),
                Update::Looks(vec![scene::Looks::default()]),
                Update::SpriteLifetimes(vec![scene::SpriteLifetime { serial: 2, sprites: (0, 1), duration_ms: 17 }])],
            vec![Update::Cells(vec![(0, 71, 5)]), Update::Phases(vec![(0, 71, 33)]), Update::Offsets(vec![(0, 71, (-3, 7))]), Update::SlotCells(vec![(9, 72, 4)])],
            vec![Update::Sprites(vec![scene::FreeSprite { x: 7, ..Default::default() }]), Update::View { id: 5, view: Some(scene::View { camera_x: 42, ..Default::default() }) }, Update::Overlay { view: 5, sprites: vec![scene::FreeSprite { x: 11, ..Default::default() }] }],
            vec![Update::Slot { id: 9, slot: None }, Update::Bindings(vec![])],
        ];
        let mut layer_ptr = std::ptr::null();
        for (i, batch) in batches.into_iter().enumerate() {
            assert!(driver.send(batch));
            assert!(display.receive(Duration::ZERO));
            let changes = display.take_changes().0;
            let work = update(&mut shown, display.scene(), &changes);
            assert_eq!(shown, *display.scene(), "batch {i}");
            if i == 0 { layer_ptr = shown.layers[0].values.as_ptr(); }
            else {
                assert_eq!(shown.layers[0].values.as_ptr(), layer_ptr);
                assert_eq!(work.full_cell_copies, 0);
            }
            if i == 1 { assert_eq!(work, Work::default()); }
            if i == 2 { assert_eq!(work.changed_cell_copies, 4); }
        }
    }

    #[test]
    fn changed_shape_replaces_owned_cells_and_leaves_pinned_snapshot_unchanged() {
        let mut shown = Scene { seq: 1, shape: WorldShape { cols: 2, rows: 1, ..Default::default() }, layers: vec![LayerCells { values: vec![1, 2], ..Default::default() }], slots: vec![LayerSlot::default()], ..Default::default() };
        let pinned = shown.clone();
        let next = Scene { seq: 2, shape: WorldShape { cols: 1, rows: 1, ..Default::default() }, layers: vec![LayerCells { values: vec![4], ..Default::default() }], ..Default::default() };
        update(&mut shown, &next, &Changes { world: true, ..Default::default() });
        assert_eq!(shown, next);
        assert_eq!(pinned.layers[0].values, [1, 2]);
    }
}
