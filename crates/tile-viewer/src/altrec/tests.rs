use super::format::{self, Encoder, Header};
use super::replay::{self, Options};
use super::writer::{self, Limits};
use super::*;
use crate::layout::Layout;
use crate::snap::{Rates, SnapInput, TargetKind, Tuning};
use scene::{Input, Interactable, InteractableKind, Scene, View, ViewState};

const ALT: u32 = 0x12;
/// The live runs step every this many microseconds, as the replays.
const TICK: u64 = 8333;

fn button(x: f64, y: f64, id: u64) -> Target {
    let rect = [x, y, 60.0, 30.0];
    Target { rect, area: rect, spread: false, anchor: None, kind: TargetKind::Button, id, alt_right_click: false }
}

/// The driver's main view and a strip of 16 buttons at the bottom: two
/// rows of 8, 100 pixels apart.
fn scene() -> Scene {
    let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
    let ui = View { interface_only: true, ..Default::default() };
    let interactables = (0..16)
        .map(|k| Interactable { region: None, alt_right_click: false, anchor: None, rect: [20 + 100 * (k % 8), if k < 8 { 510 } else { 555 }, 60, 30], kind: InteractableKind::Button, id: 100 + k as u64 })
        .collect();
    Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        interactables,
        ..Default::default()
    }
}

fn frame(l: &mut Layout, s: &Scene) {
    let bounds = crate::layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
    l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], Some(bounds));
}

fn layout(s: &Scene) -> Layout {
    let mut l = Layout::default();
    l.configure("main=pointer").unwrap();
    frame(&mut l, s);
    frame(&mut l, s);
    l
}

fn key(vk: u32, down: bool) -> Input {
    Input::Key { vk, down, text: None }
}

fn press(down: bool, (x, y): (i32, i32)) -> Input {
    Input::MouseButton { button: Button::Left, down, x, y }
}

fn encode(records: &[Record]) -> Vec<u8> {
    let base = records.first().map_or(0, Record::t);
    let mut out = Vec::new();
    Header { wall_us: 1_700_000_000_000_000, base_us: base, part: 0 }.write(&mut out);
    let mut e = Encoder::new(base);
    for r in records {
        e.record(r, &mut out);
    }
    out
}

/// A live run of the viewer's Alt mode, recorded: steps on the replay's
/// grid, motion and clicks between them.
struct Live {
    snap: SnapInput,
    l: Layout,
    s: Scene,
    /// The step last taken.
    k: u64,
    /// Motion given while the pointer was in flight.
    in_flight: usize,
    given: Vec<(u64, Point)>,
}

impl Live {
    fn new() -> Self {
        let s = scene();
        let mut l = layout(&s);
        l.route(Input::MouseMove { x: 50, y: 525 });
        let mut snap = SnapInput::default();
        snap.set_recorder(Some(Recorder::in_memory(1 << 16)));
        Self { snap, l, s, k: 0, in_flight: 0, given: Vec::new() }
    }

    fn alt(&mut self) {
        let now = seconds(self.k * TICK + 1000);
        self.snap.route(key(ALT, true), &mut self.l, &self.s, now, Rates::default());
        self.snap.route(key(ALT, false), &mut self.l, &self.s, now, Rates::default());
    }

    /// `n` steps, with `motion` raw counts before each.
    fn steps(&mut self, n: u64, motion: Point) {
        for _ in 0..n {
            if motion != (0.0, 0.0) {
                let t = self.k * TICK + 4000;
                if !self.snap.snapper.flight().done(seconds(t)) {
                    self.in_flight += 1;
                }
                self.snap.motion_at(motion, seconds(t));
                self.given.push((t, motion));
            }
            self.k += 1;
            self.snap.tick(&mut self.l, &self.s, seconds(self.k * TICK), Rates::default());
        }
    }

    fn click(&mut self, at: (i32, i32)) {
        let t = self.k * TICK + 2000;
        self.snap.route(press(true, at), &mut self.l, &self.s, seconds(t), Rates::default());
        self.snap.route(press(false, at), &mut self.l, &self.s, seconds(t + 1000), Rates::default());
    }

    fn records(&self) -> Vec<Record> {
        self.snap.recorder().unwrap().drain()
    }
}

/// Alt on, a few moves right and down between the strip's buttons (some
/// of them while the pointer still flies), clicks, Alt off.
fn session() -> Live {
    let mut live = Live::new();
    live.alt();
    live.steps(10, (0.0, 0.0));
    // Right, one button's width a step or so, on through the flight.
    live.steps(40, (4.0, 0.0));
    live.steps(80, (0.0, 0.0));
    live.click((0, 0));
    live.steps(20, (0.0, 0.0));
    // Down and a little left; then a nudge within the dead zone.
    live.steps(12, (-1.5, 6.0));
    live.steps(60, (0.0, 0.0));
    live.steps(2, (1.0, 0.0));
    live.steps(60, (0.0, 0.0));
    live.click((0, 0));
    // Right along the lower row, a quick flick.
    live.steps(6, (10.0, 1.0));
    live.steps(80, (0.0, 0.0));
    live.click((0, 0));
    live.steps(5, (0.0, 0.0));
    live.alt();
    live.steps(5, (0.0, 0.0));
    live
}

#[test]
fn a_snapshot_is_written_only_when_the_targets_change() {
    let mut r = Recorder::in_memory(64);
    let mut targets = vec![button(10.0, 10.0, 0xdead_0000), button(100.0, 10.0, 0xdead_0010)];
    assert_eq!(r.observe(0.1, &targets), 1);
    assert_eq!(r.observe(0.2, &targets), 1, "the same: none");
    targets[1].rect[0] += 0.5;
    targets[1].area = targets[1].rect;
    assert_eq!(r.observe(0.3, &targets), 2, "a box moved");
    assert_eq!(r.observe(0.4, &targets), 2);
    // Another element in the same box: a new snapshot too.
    targets[0].id = 0xbeef;
    assert_eq!(r.observe(0.5, &targets), 3);
    let records = r.drain();
    let snapshots: Vec<_> = records.iter().filter_map(|r| match r { Record::Snapshot { number, targets, .. } => Some((*number, targets.clone())), _ => None }).collect();
    assert_eq!(snapshots.len(), 3);
    // Keys, not the driver's ids: the same element keeps its key.
    assert_eq!(snapshots[0].1.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(snapshots[1].1.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(snapshots[2].1.iter().map(|t| t.id).collect::<Vec<_>>(), [2, 1]);
    // Records refer to the last one written.
    r.pointer(0.6, PointerKind::Rest, (40.0, 25.0), Some(0xdead_0010));
    assert!(matches!(r.drain()[..], [Record::Pointer { snapshot: 3, target: Some(1), .. }]));
}

#[test]
fn every_record_comes_back_from_the_file_as_it_was() {
    let targets = vec![
        Target { anchor: Some((12.5, 20.0)), ..button(10.0, 10.0, 0) },
        // A check box with its label: a larger area, spread.
        Target { area: [10.0, 50.0, 200.0, 30.0], spread: true, ..button(10.0, 50.0, 1) },
        Target { kind: TargetKind::Slider { track: [10.0, 100.0, 180.0, 0.0], value: -3 }, ..button(10.0, 90.0, 2) },
        Target { kind: TargetKind::List { scroll: [300.0, 0.0, 200.0, 400.0], step: 18.75, has_items: true }, ..button(300.0, 0.0, 3) },
        Target { kind: TargetKind::ListItem { list: Some(3) }, ..button(300.0, 0.1, 4) },
        Target { kind: TargetKind::ListItem { list: None }, ..button(-1e-300, f64::MAX, 5) },
        Target::capture([0, 0, 800, 600], (400, 300)),
    ];
    let records = vec![
        Record::Rate { t: 10, updates_per_second: 3.0 },
        Record::Snapshot { t: 10, number: 1, targets },
        Record::Mode { t: 11, change: ModeChange::On, reason: Reason::MenuOpened, at: (50.0, -525.0), snapshot: 1 },
        Record::Pointer { t: 12, kind: PointerKind::Origin, at: (1.0 / 3.0, 2.0), snapshot: 1, target: None },
        Record::Motion { t: 13, d: (0.1, -7.0) },
        Record::Motion { t: 13, d: (1.0 / 1.25, 1e9) },
        Record::Pointer { t: 400_000, kind: PointerKind::Rest, at: (40.0, 25.0), snapshot: 1, target: Some(4) },
        Record::Click { t: 400_001, button: Button::Right, down: true, used: ClickUse::Pointer, snapshot: 1, target: Some(6) },
        Record::Click { t: 400_002, button: Button::Left, down: false, used: ClickUse::Swallowed, snapshot: 1, target: None },
        Record::Click { t: 400_003, button: Button::Middle, down: true, used: ClickUse::SliderBegin, snapshot: 1, target: Some(2) },
        Record::Click { t: 400_004, button: Button::Left, down: true, used: ClickUse::SliderEnd, snapshot: 1, target: Some(2) },
        Record::Mode { t: 500_000, change: ModeChange::Land, reason: Reason::Alt, at: (0.0, 0.0), snapshot: 0 },
        Record::Mode { t: 500_000, change: ModeChange::Stop, reason: Reason::WorldTarget, at: (0.0, 0.0), snapshot: 0 },
        Record::GaveUp { t: 3_000_000, since_exit_us: 2_500_000, snapshot: 1, target: 3, at: (301.0, 2.0), button: Button::Left },
        Record::Dropped { t: 3_000_000, count: 1 << 40 },
        Record::End { t: u64::MAX / 2 },
    ];
    let mut bytes = encode(&records);
    let file = format::decode(&bytes).unwrap();
    assert_eq!(file.header, Header { wall_us: 1_700_000_000_000_000, base_us: 10, part: 0 });
    assert_eq!(file.records, records);
    assert_eq!(file.cut, None);
    // Compact: a motion of whole counts in a few bytes.
    let mut one = Vec::new();
    Encoder::new(0).record(&Record::Motion { t: 5, d: (3.0, -2.0) }, &mut one);
    assert!(one.len() <= 7, "{one:?}");
    // A type a reader does not know is skipped; a record cut short ends
    // the file, what came before kept.
    let mut e = Vec::new();
    e.extend_from_slice(&[200, 3, 0, 1, 2]);
    bytes.splice(format::HEADER_LEN..format::HEADER_LEN, e);
    assert_eq!(format::decode(&bytes).unwrap().records, records);
    bytes.truncate(bytes.len() - 3);
    let cut = format::decode(&bytes).unwrap();
    assert!(cut.cut.is_some());
    assert_eq!(cut.records, records[..records.len() - 1]);
    assert!(format::decode(b"not a recording").is_err());
}

#[test]
fn motion_in_flight_is_recorded_as_it_comes() {
    let live = session();
    assert!(live.in_flight > 10, "{}", live.in_flight);
    let records = live.records();
    let motion: Vec<(u64, Point)> = records.iter().filter_map(|r| match r { Record::Motion { t, d } => Some((*t, *d)), _ => None }).collect();
    assert_eq!(motion, live.given, "every motion, at its time, as given");
    // Each burst starts at where the pointer was, the target it was on.
    let first = records.iter().position(|r| matches!(r, Record::Motion { .. })).unwrap();
    assert!(matches!(records[first - 1], Record::Pointer { kind: PointerKind::Origin, target: Some(_), .. }), "{:?}", &records[first - 1]);
    let origins = records.iter().filter(|r| matches!(r, Record::Pointer { kind: PointerKind::Origin, .. })).count();
    assert_eq!(origins, 4, "one a burst");
    // Mode on and off, why.
    let modes: Vec<_> = records.iter().filter_map(|r| match r { Record::Mode { change, reason, .. } => Some((*change, *reason)), _ => None }).collect();
    assert_eq!(modes, [(ModeChange::On, Reason::Alt), (ModeChange::Land, Reason::Alt)]);
    assert!(matches!(records[0], Record::Rate { updates_per_second: 3.0, .. }));
}

#[test]
fn a_replay_of_a_recording_lands_and_clicks_as_the_live_run_did() {
    let live = session();
    let records = format::decode(&encode(&live.records())).unwrap().records;
    let summary = replay::summarize(&records, &Tuning::default());
    assert_eq!((summary.sessions, summary.clicks, summary.clicks_on_target), (1, 3, 3), "{summary:?}");
    assert!(summary.landings >= 3, "{summary:?}");
    assert_eq!(summary.wrong_landings, 0, "{summary:?}");
    // The landings the recording shows, with their elements.
    let mut targets: Vec<Target> = Vec::new();
    let mut landed = Vec::new();
    let mut clicked = Vec::new();
    for r in &records {
        match r {
            Record::Snapshot { targets: shown, .. } => targets.clone_from(shown),
            Record::Pointer { t, kind: PointerKind::Rest, at, target, .. } => landed.push((*t, targets[target.unwrap() as usize].id, *at)),
            Record::Click { down: true, target, .. } => clicked.push(targets[target.unwrap() as usize].id),
            _ => {}
        }
    }
    let run = replay::run(&records, &Options { tick_us: TICK, ..Options::default() });
    assert_eq!(run.landings.iter().map(|l| (l.t, l.key, l.at)).collect::<Vec<_>>(), landed);
    assert_eq!(run.clicks.iter().map(|c| c.replayed.unwrap()).collect::<Vec<_>>(), clicked);
    assert_eq!(run.reached(), 3);
    // Other values pick otherwise: a cone of 1° reaches less.
    let narrow = replay::run(&records, &Options { tick_us: TICK, tuning: Tuning { cone: 1f64.to_radians(), ..Tuning::default() }, ..Options::default() });
    assert!(narrow.reached() < 3 || narrow.landings != run.landings, "{narrow:?}");
}

#[test]
fn a_click_on_a_box_of_the_mode_soon_after_it_is_giving_up() {
    let targets = vec![button(10.0, 10.0, 7), button(100.0, 10.0, 8)];
    let gave_up = |r: &Recorder| r.drain().into_iter().filter(|r| matches!(r, Record::GaveUp { .. })).collect::<Vec<_>>();
    let on = |r: &mut Recorder, now: f64| r.mode_on(now, Reason::Alt, (0.0, 0.0), &targets, 3.0);
    // A box of the last snapshot, 2 s after: recorded, once.
    let mut r = Recorder::in_memory(64);
    on(&mut r, 1.0);
    r.mode_off(3.0, ModeChange::Land, Reason::Alt);
    r.click_after(5.0, Button::Left, (110.0, 20.0), Some(targets[1].rect));
    r.click_after(5.5, Button::Left, (110.0, 20.0), Some(targets[1].rect));
    assert_eq!(gave_up(&r), [Record::GaveUp { t: 5_000_000, since_exit_us: 2_000_000, snapshot: 1, target: 1, at: (110.0, 20.0), button: Button::Left }]);
    // A changed box, a new one, nothing: none; then the box still counts.
    on(&mut r, 6.0);
    r.mode_off(7.0, ModeChange::Stop, Reason::FocusLost);
    r.click_after(7.5, Button::Left, (110.0, 20.0), Some([100.0, 12.0, 60.0, 30.0]));
    r.click_after(7.6, Button::Left, (500.0, 20.0), Some([480.0, 10.0, 60.0, 30.0]));
    r.click_after(7.7, Button::Left, (700.0, 700.0), None);
    assert!(gave_up(&r).is_empty());
    r.click_after(16.9, Button::Right, (20.0, 20.0), Some(targets[0].rect));
    assert!(matches!(gave_up(&r)[..], [Record::GaveUp { target: 0, button: Button::Right, .. }]));
    // After 10 s: none.
    on(&mut r, 20.0);
    r.mode_off(21.0, ModeChange::Land, Reason::Esc);
    r.click_after(31.5, Button::Left, (20.0, 20.0), Some(targets[0].rect));
    assert!(gave_up(&r).is_empty());
    // The mode back on ends the watch.
    on(&mut r, 40.0);
    r.mode_off(41.0, ModeChange::Land, Reason::Alt);
    on(&mut r, 42.0);
    r.click_after(43.0, Button::Left, (20.0, 20.0), Some(targets[0].rect));
    assert!(gave_up(&r).is_empty());
}

#[test]
fn a_click_on_a_button_after_the_mode_is_recorded_with_the_viewers_hit_test() {
    let mut live = session();
    let records = live.records();
    let targets = records.iter().rev().find_map(|r| match r { Record::Snapshot { targets, .. } => Some(targets.clone()), _ => None }).unwrap();
    let k = targets.iter().position(|t| t.kind == TargetKind::Button).unwrap();
    let [x, y, w, h] = targets[k].rect;
    let middle = ((x + w / 2.0) as i32, (y + h / 2.0) as i32);
    // On the world: nothing; on the button: recorded.
    live.click((400, 200));
    live.click(middle);
    let after = live.records();
    assert!(matches!(after[..], [Record::GaveUp { target, at, .. }] if target as usize == k && at == (middle.0 as f64, middle.1 as f64)), "{after:?}");
    let summary = replay::summarize(&[records, after].concat(), &Tuning::default());
    assert_eq!(summary.gave_up, 1);
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("altrec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_writer_drops_and_counts_rather_than_wait() {
    // Its queue full, a push returns at once and is counted.
    let sink = writer::Sink::new(4);
    let started = std::time::Instant::now();
    for t in 0..100 {
        sink.push(Record::Motion { t, d: (1.0, 0.0) });
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!((sink.queue.len(), sink.queue.dropped()), (4, 96));
    // With a writer: what it got is in the file, what it did not counted.
    let dir = temp_dir("drops");
    let sink = writer::start(dir.clone(), 0, Limits { queue: 8, ..Limits::default() }).unwrap();
    for t in 0..5000 {
        sink.push(Record::Motion { t, d: (1.0, 0.0) });
    }
    assert!(writer::finish(&sink, std::time::Duration::from_secs(5)));
    let files = writer::recordings(&dir);
    assert_eq!(files.len(), 1);
    let file = format::decode(&std::fs::read(&files[0].0).unwrap()).unwrap();
    let written = file.records.iter().filter(|r| matches!(r, Record::Motion { .. })).count() as u64;
    let dropped: u64 = file.records.iter().map(|r| if let Record::Dropped { count, .. } = r { *count } else { 0 }).sum();
    assert_eq!(written + dropped, 5000);
    assert!(matches!(file.records.last(), Some(Record::End { .. })));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn recordings_stay_within_their_total_size() {
    let dir = temp_dir("cap");
    // Another file of the folder is never touched.
    std::fs::write(dir.join("driver.log"), vec![b'x'; 50_000]).unwrap();
    std::fs::write(dir.join("alt-mode-0000000001-1-000.altrec"), vec![0; 3000]).unwrap();
    let limits = Limits { part: 2000, total: 6000, queue: 1 << 16 };
    let sink = writer::start(dir.clone(), 2_000_000_000, limits).unwrap();
    let targets: Vec<Target> = (0..8).map(|k| button(k as f64 * 70.5, 3.25, k)).collect();
    for t in 0..400u64 {
        if t % 50 == 0 {
            sink.push(Record::Snapshot { t: t * 1000, number: (t / 50 + 1) as u32, targets: targets.clone() });
        }
        sink.push(Record::Motion { t: t * 1000, d: (0.3, 1.0 / 7.0) });
        if t % 20 == 19 {
            // (Written in small pieces, so that parts fill one after another.)
            sink.wake.ring();
            while !sink.queue.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    assert!(writer::finish(&sink, std::time::Duration::from_secs(5)));
    let files = writer::recordings(&dir);
    let total: u64 = files.iter().map(|f| f.1).sum();
    assert!(total <= limits.total, "{total}: {files:?}");
    assert!(files.len() >= 2, "{files:?}");
    assert!(!dir.join("alt-mode-0000000001-1-000.altrec").exists(), "the oldest went first");
    assert_eq!(std::fs::metadata(dir.join("driver.log")).unwrap().len(), 50_000);
    // Each part stands alone: a header, the rate and targets in effect.
    for (path, _) in &files {
        let file = format::decode(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(file.cut, None);
        assert!(file.header.part > 0 || path.ends_with(format!("alt-mode-0000002000-{}-000.altrec", std::process::id())));
        if file.header.part > 0 {
            assert!(matches!(file.records.first(), Some(Record::Snapshot { .. })), "{path:?}");
        }
    }
    let newest = format::decode(&std::fs::read(&files.last().unwrap().0).unwrap()).unwrap();
    assert!(matches!(newest.records.last(), Some(Record::End { .. })));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_landing_short_of_the_element_clicked_is_the_picker_getting_it_wrong() {
    // A, B, C in a row; motion from A towards C lands on B, then on C,
    // which is clicked. Then from C back to B, clicked at once.
    let targets = vec![button(0.0, 0.0, 0), button(100.0, 0.0, 1), button(200.0, 0.0, 2)];
    let centre = |k: usize| targets[k].center();
    let records = vec![
        Record::Snapshot { t: 0, number: 1, targets: targets.clone() },
        Record::Mode { t: 0, change: ModeChange::On, reason: Reason::Alt, at: centre(0), snapshot: 1 },
        Record::Pointer { t: 10, kind: PointerKind::Rest, at: centre(0), snapshot: 1, target: Some(0) },
        Record::Pointer { t: 20, kind: PointerKind::Origin, at: centre(0), snapshot: 1, target: Some(0) },
        Record::Motion { t: 20, d: (150.0, 5.0) },
        Record::Pointer { t: 500, kind: PointerKind::Rest, at: centre(1), snapshot: 1, target: Some(1) },
        Record::Pointer { t: 600, kind: PointerKind::Origin, at: centre(1), snapshot: 1, target: Some(1) },
        Record::Motion { t: 600, d: (100.0, 0.0) },
        Record::Pointer { t: 900, kind: PointerKind::Rest, at: centre(2), snapshot: 1, target: Some(2) },
        Record::Click { t: 1000, button: Button::Left, down: true, used: ClickUse::Pointer, snapshot: 1, target: Some(2) },
        Record::Click { t: 1001, button: Button::Left, down: false, used: ClickUse::Pointer, snapshot: 1, target: Some(2) },
        Record::Pointer { t: 1100, kind: PointerKind::Origin, at: centre(2), snapshot: 1, target: Some(2) },
        Record::Motion { t: 1100, d: (-120.0, 0.0) },
        Record::Click { t: 1200, button: Button::Left, down: true, used: ClickUse::Pointer, snapshot: 1, target: Some(1) },
        Record::Mode { t: 2000, change: ModeChange::Land, reason: Reason::Alt, at: (0.0, 0.0), snapshot: 0 },
    ];
    let s = replay::summarize(&records, &Tuning::default());
    assert_eq!((s.sessions, s.clicks, s.landings), (1, 2, 3), "{s:?}");
    // The mode snapping on is no landing of motion; B was on the way to C.
    assert_eq!((s.wrong_landings, s.aimed_wrong_landings), (1, 1), "{s:?}");
}

/// The live session through the writer to a file, read back as
/// the driver's altrec tool reads it. (`ALTREC_SAMPLE_DIR`: the file is left
/// there, to try the tool on.)
#[test]
fn a_written_session_reads_back_and_reports() {
    let keep = std::env::var_os("ALTREC_SAMPLE_DIR").map(std::path::PathBuf::from);
    let dir = keep.clone().unwrap_or_else(|| temp_dir("report"));
    std::fs::create_dir_all(&dir).unwrap();
    let live = session();
    let records = live.records();
    let sink = writer::start(dir.clone(), 1_761_300_000_000_000, Limits::default()).unwrap();
    for r in &records {
        sink.push(r.clone());
    }
    assert!(writer::finish(&sink, std::time::Duration::from_secs(5)));
    let sessions = replay::sessions(std::slice::from_ref(&dir)).unwrap();
    let ours = sessions.iter().find(|s| s.name == format!("alt-mode-1761300000-{}", std::process::id())).unwrap();
    assert_eq!(ours.records[..records.len()], records[..]);
    assert!(matches!(ours.records[records.len()..], [Record::End { .. }]));
    let report = replay::report(ours, &Options { tick_us: TICK, ..Options::default() });
    assert!(report.contains("on the clicked element at 3 of 3 clicks (100 %)"), "{report}");
    if keep.is_none() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
