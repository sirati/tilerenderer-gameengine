//! Input routing changes only with the immutable frame the worker presented.
use crate::{App, CachedPicture, Scene, input, layout, looks, scaling};
use std::time::Instant;

pub(crate) struct Mapping {
    scene: std::sync::Arc<Scene>,
    layout: layout::Layout,
    looks: std::sync::Arc<looks::LookScene>,
    plan: Option<scaling::Plan>,
    input: input::InputState,
    frame: Option<layout::Frame>,
    reported_size: (u32, u32),
    reported_window: (u32, u32, u32),
    overlay_camera: Option<u32>,
}
impl Mapping {
    pub fn camera_arrival_receipt(&self, frame: &layout::Frame) -> Option<scene::Input> {
        self.layout.camera_arrival_receipt(&self.scene, frame)
    }
    pub fn capture(app: &App) -> Self {
        Self { scene: app.shown_scene.clone(), layout: app.layout.clone(), looks: app.look_scene.clone(), plan: app.plan, input: app.input.clone(), frame: app.drawn_frame.clone(), reported_size: app.reported_size, reported_window: app.reported_window, overlay_camera: app.overlay_camera }
    }
    pub fn restore(self, app: &mut App) {
        app.shown_scene = self.scene;
        app.layout = self.layout;
        app.look_scene = self.looks;
        app.plan = self.plan;
        app.input = self.input;
        app.drawn_frame = self.frame;
        app.reported_size = self.reported_size;
        app.reported_window = self.reported_window;
        app.overlay_camera = self.overlay_camera;
    }
    pub fn activate(self, app: &mut App, unchanged_input: bool) -> Option<scene::Input> {
        app.shown_scene = self.scene;
        if unchanged_input { app.layout = self.layout; }
        else { app.layout.adopt_presented_mapping(self.layout); }
        app.look_scene = self.looks;
        app.plan = self.plan;
        app.reported_size = self.reported_size;
        app.reported_window = self.reported_window;
        app.overlay_camera = self.overlay_camera;
        // Deliver the latest pointer after this frame's versioned camera
        // control batch, never before it or from a stale preparation.
        self.plan.and_then(|plan| app.input.set_plan(plan))
    }
}

pub(crate) struct PendingUi {
    pub id: u64,
    pub config_revision: u64,
    pub input_revision: u64,
    pub mapping: Mapping,
    pub started: Instant,
    pub main_prepare: std::time::Duration,
    pub frame: layout::Frame,
    pub picture: CachedPicture,
    pub inputs: Vec<scene::Input>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Assets, ViewerConfig};
    use std::sync::{Arc, atomic::AtomicBool};

    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no images in this fixture".into()) }
    }
    fn app() -> App {
        let compiled = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}").unwrap().compile(&NoImages, 16).unwrap();
        let (_, display) = scene::connect();
        crate::new_app(ViewerConfig::default(), Assets::new(Arc::new(compiled)), display, Arc::new(AtomicBool::new(false)))
    }
    #[test]
    fn combat_receipt_reads_the_immutable_mapping_not_a_newer_prepared_scene() {
        let mut app = app();
        app.layout.configure("main=focus,pane=pointer,split=vertical").unwrap();
        let mut scene = Scene { shape: scene::WorldShape { cols: 62, rows: 125, ..Default::default() },
            views: vec![scene::ViewState { id: 0, view: scene::View { zoom: (1, 1), ..Default::default() }, overlay: vec![] }],
            focus: Some((3000, 2000)), camera_focus: Some(scene::CameraFocus { serial: 10, center: (3000, 2000) }),
            camera_arrival: Some(scene::CameraArrival { serial: 10, bounds: [2936, 1968, 3064, 2032] }), ..Default::default() };
        app.shown_scene = Arc::new(scene.clone());
        let frame = app.layout.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], None);
        let accepted = Mapping::capture(&app);
        scene.camera_focus = Some(scene::CameraFocus { serial: 11, center: (7000, 3000) });
        scene.camera_arrival = Some(scene::CameraArrival { serial: 11, bounds: [6936, 2968, 7064, 3032] });
        app.shown_scene = Arc::new(scene);
        assert_eq!(accepted.camera_arrival_receipt(&frame), Some(scene::Input::CameraArrived { serial: 10 }));
        assert_eq!(app.camera_arrival_reported, None, "capturing/preparing never sends a receipt");
    }

    #[test]
    fn preparing_a_frame_shares_scene_data_and_cannot_activate_its_input_mapping() {
        let mut app = app();
        app.shown_scene = Arc::new(Scene { seq: 7, ..Default::default() });
        let original = app.shown_scene.clone();
        let before = Mapping::capture(&app);
        assert!(Arc::ptr_eq(&before.scene, &original), "a control snapshot must not duplicate world layers");
        app.shown_scene = Arc::new(Scene { seq: 8, ..Default::default() });
        let candidate = Mapping::capture(&app);
        before.restore(&mut app);
        assert!(Arc::ptr_eq(&app.shown_scene, &original));
        assert_eq!(app.shown_scene.seq, 7, "preparation/acquisition failure keeps active routing");
        let _ = candidate.activate(&mut app, true);
        assert_eq!(app.shown_scene.seq, 8, "only a matching successful presentation activates routing");
    }
    #[test]
    fn pending_routing_shares_immutable_looks_and_keeps_its_exact_metadata() {
        let mut app = app();
        let mut scene = Scene::default();
        scene.interactables.push(scene::Interactable { id: 3, rect: [10, 20, 30, 40], ..Default::default() });
        scene.looks.push(scene::Looks { id: 3, views: vec![scene::ViewLooks {
            view: 0, sprites: (0, 1), states: vec![(scene::LookState::default(), vec![scene::FreeSprite { set: 11, ..Default::default() }; 10_000])],
        }], ..Default::default() });
        assert!(looks::LookScene::refresh_shared(&mut app.look_scene, &scene));
        let active = app.look_scene.clone();
        let before = Mapping::capture(&app);
        let pending = Mapping::capture(&app);
        assert!(Arc::ptr_eq(&before.looks, &active));
        assert!(Arc::ptr_eq(&pending.looks, &active));
        assert!(!looks::LookScene::refresh_shared(&mut app.look_scene, &scene));
        assert!(Arc::ptr_eq(&app.look_scene, &active), "unchanged metadata retains its allocation");
        scene.interactables[0].rect[0] = 99;
        scene.looks[0].views[0].states[0].1[0].set = 22;
        assert!(looks::LookScene::refresh_shared(&mut app.look_scene, &scene));
        assert!(!Arc::ptr_eq(&app.look_scene, &active));
        assert_eq!(pending.looks.interactables[0].rect[0], 10);
        assert_eq!(pending.looks.looks[0].views[0].states[0].1[0].set, 11);
        before.restore(&mut app);
        assert!(Arc::ptr_eq(&app.look_scene, &active));
        let _ = pending.activate(&mut app, false);
        assert!(Arc::ptr_eq(&app.look_scene, &active), "a presented candidate activates precisely its own routing metadata");
    }

    #[test]
    fn only_matching_presented_configuration_releases_a_complete_control_batch() {
        let mut app = app();
        let old = app.shown_scene.clone();
        app.shown_scene = Arc::new(Scene { seq: 99, ..Default::default() });
        let candidate = || PendingUi {
            id: 8, config_revision: 3, input_revision: app.input_revision,
            mapping: Mapping::capture(&app), started: Instant::now(), main_prepare: std::time::Duration::ZERO,
            frame: layout::Frame::default(),
            picture: CachedPicture { plan: scaling::plan((100, 100), 1.0, (100, 100), &app.settings.resolution), copies: vec![], pixel_box: 1 },
            inputs: vec![scene::Input::MainView { x: 20, y: 30, w: 40, h: 50 }],
        };
        let mut pending = Some(candidate());
        assert!(matches!(complete(&mut pending, 7, Some(3)), Completion::Unrelated));
        assert_eq!(pending.as_ref().unwrap().id, 8);
        assert!(matches!(complete(&mut pending, 8, Some(4)), Completion::Obsolete));
        assert!(pending.is_none(), "resize must release the old candidate for retry");
        pending = Some(candidate());
        app.shown_scene = old;
        let Completion::Presented(accepted) = complete(&mut pending, 8, Some(3)) else { panic!("matching frame was rejected") };
        assert_eq!(accepted.inputs, [scene::Input::MainView { x: 20, y: 30, w: 40, h: 50 }]);
        let _ = accepted.mapping.activate(&mut app, true);
        assert_eq!(app.shown_scene.seq, 99);
        assert!(pending.is_none());
    }

    #[test]
    fn a_pending_frame_cannot_revert_new_settings() {
        let mut app = app();
        let candidate = Mapping::capture(&app);
        let revision = app.input_revision;
        let mut settings = app.settings.clone();
        settings.layout.split = viewer_settings::Split::Vertical2To1;
        settings.clean_map.a.mode = viewer_settings::clean_map::Mode::Custom;
        settings.clean_map.a.hidden = viewer_settings::clean_map::UNITS;
        settings.input.capture_sensitivity = 2.5;
        app.set_settings(settings.clone());
        assert_ne!(revision, app.input_revision);
        let desired_split = app.layout.split;
        let unchanged = revision == app.input_revision;
        let _ = candidate.activate(&mut app, unchanged);
        assert_eq!(app.layout.split, desired_split);
        assert_eq!(app.layout.clean_map, settings.clean_map);
        assert_eq!(app.settings.input.capture_sensitivity, 2.5);
    }
}

pub(crate) enum Completion {
    Unrelated,
    Obsolete,
    Presented(PendingUi),
}

pub(crate) fn complete(pending: &mut Option<PendingUi>, id: u64, configuration: Option<u64>) -> Completion {
    if !pending.as_ref().is_some_and(|candidate| candidate.id == id) { return Completion::Unrelated; }
    let candidate = pending.take().expect("matched complete transaction");
    if configuration != Some(candidate.config_revision) { return Completion::Obsolete; }
    Completion::Presented(candidate)
}

#[cfg(test)]
mod wake_tests {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    #[test]
    fn worker_control_replies_use_event_delivery_without_a_redraw_or_frame() {
        let controls = Arc::new(AtomicUsize::new(0));
        let redraws = Arc::new(AtomicUsize::new(0));
        let delivered = controls.clone();
        let event_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || { delivered.fetch_add(1, Ordering::SeqCst); });
        let redrawn = redraws.clone();
        let wake = crate::worker_wake(Some(&event_wake), move || { redrawn.fetch_add(1, Ordering::SeqCst); });
        wake();
        assert_eq!(controls.load(Ordering::SeqCst), 1);
        assert_eq!(redraws.load(Ordering::SeqCst), 0, "hidden/occluded windows and a frame cap must not block SurfaceNeeded/Ready");
    }
}
