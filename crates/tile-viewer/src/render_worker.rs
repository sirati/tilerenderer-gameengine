//! GPU ownership belongs to this actor. The event loop only publishes immutable
//! requests and polls replies; it never waits for surface acquisition or uploads.
#![forbid(unsafe_code)]

use crate::{ATLAS_PART, ArtInbox, Assets, PendingArt, PendingArtPayload, PreparingArt};
use handoff::{Latest, Mailbox, Wake};
use render_desc::Compiled;
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tile_render::{Camera, Dirty, PlacedSprite, Present, ScreenCopy, TileRenderer, TileWorld};
use winit::window::Window;

pub(crate) struct Init {
    pub window: Arc<Window>,
    pub native_thread_init: Option<fn()>,
    pub size: (u32, u32),
    pub initial: Arc<Compiled>,
    pub base: u64,
    pub through: u64,
    pub profile: bool,
    pub bitmap: bool,
    pub wake_main: Arc<dyn Fn() + Send + Sync>,
}
#[derive(Clone)]
pub(crate) struct Config {
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub present_mode: wgpu::PresentMode,
    pub latency: u32,
}
pub(crate) struct ArtSnapshot {
    pub base: u64,
    pub whole: Option<Arc<Compiled>>,
    pub growths: Vec<(u64, Arc<render_desc::Growth>)>,
    pub reserve: (u32, u32),
    pub target: u64,
}
/// A complete immutable candidate. Incremental world updates are accepted only
/// when their predecessor was actually drawn; coalescing otherwise rebuilds it.
pub(crate) struct Frame {
    pub config_revision: u64,
    pub id: u64,
    pub base: u64,
    pub through: u64,
    pub world: Option<Arc<TileWorld>>,
    pub world_version: u64,
    pub previous_world_version: u64,
    pub replace_world: bool,
    pub dirty: Dirty,
    pub free_changed: bool,
    pub markers: Vec<PlacedSprite>,
    pub cameras: Vec<Camera>,
    pub copies: Vec<ScreenCopy>,
    pub overlay_from: usize,
    pub tick: u32,
    pub host_tick_ms: u32,
    pub clear: [u8; 4],
    pub present: Present,
    pub patches: Vec<scene::Patch>,
    pub background: tile_render::BackgroundSampling,
    pub main_prepare: Duration,
}
pub(crate) enum Event {
    SurfaceNeeded {
        instance: wgpu::Instance,
    },
    Ready {
        config: wgpu::SurfaceConfiguration,
        software: bool,
        present_modes: Vec<wgpu::PresentMode>,
        base: u64,
        through: u64,
    },
    ArtCommitted {
        base: u64,
        through: u64,
        compiled: Arc<Compiled>,
        cursor_art: Arc<crate::cursor::Art>,
    },
    Presented {
        id: u64,
        worker_prepare: Duration,
        rendered: bool,
    },
    Bitmap {
        id: u64,
        width: u32,
        height: u32,
        pixels: Vec<u32>,
        worker_prepare: Duration,
        rendered: bool,
    },
    Retry {
        id: u64,
    },
    GpuCompleted {
        id: u64,
        at: Instant,
    },
    Timings(Vec<tile_render::timing::FrameTimes>),
    Failed(String),
}
struct Reply {
    sequence: u64,
    event: Event,
}
struct Shared {
    frames: Latest<Frame>,
    art: Latest<ArtSnapshot>,
    config: Latest<Config>,
    replies: Mailbox<Reply>,
    art_reply: Latest<Reply>,
    reply_sequence: AtomicU64,
    published: AtomicU64,
    stop: AtomicBool,
    wake: Wake,
    wake_main: Arc<dyn Fn() + Send + Sync>,
    surfaces: Mailbox<Result<wgpu::Surface<'static>, String>>,
    timing: Latest<bool>,
    completions: Arc<Mailbox<(u64, Instant)>>,
}
pub(crate) struct Worker {
    shared: Arc<Shared>,
    reader: RefCell<Reader>,
}
impl Worker {
    pub fn spawn(init: Init) -> Self {
        let shared = Arc::new(Shared {
            frames: Latest::new(),
            art: Latest::new(),
            config: Latest::new(),
            replies: Mailbox::new(),
            art_reply: Latest::new(),
            reply_sequence: AtomicU64::new(0),
            published: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            wake: Wake::new(),
            wake_main: init.wake_main.clone(),
            surfaces: Mailbox::new(),
            timing: Latest::new(),
            completions: Arc::new(Mailbox::new()),
        });
        let owned = shared.clone();
        let spawn = std::thread::Builder::new()
            .name("viewer-gpu".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::thread_start::initialized(init.native_thread_init, || run(init, &owned))
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => owned.reply(Event::Failed(error)),
                    Err(panic) => {
                        let why = panic
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| panic.downcast_ref::<&str>().copied())
                            .unwrap_or("unknown panic");
                        owned.reply(Event::Failed(format!("GPU worker failed: {why}")));
                    }
                }
                owned.stop.store(true, Ordering::Release);
            });
        if let Err(error) = spawn {
            shared.stop.store(true, Ordering::Release);
            shared.reply(Event::Failed(format!("cannot start GPU worker: {error}")));
        }
        Self {
            shared,
            reader: RefCell::default(),
        }
    }
    pub fn frame(&self, frame: Frame) -> bool {
        if self.shared.stop.load(Ordering::Acquire) {
            return false;
        }
        self.shared.frames.publish(frame);
        self.shared.wake.ring();
        true
    }
    pub fn art(&self, art: ArtSnapshot) {
        self.shared.art.publish(art);
        self.shared.wake.ring();
    }
    pub fn configure(&self, config: Config) {
        self.shared.config.publish(config);
        self.shared.wake.ring();
    }
    pub fn surface(&self, surface: Result<wgpu::Surface<'static>, String>) {
        self.shared.surfaces.send(surface);
        self.shared.wake.ring();
    }
    pub fn timing(&self, enabled: bool) {
        self.shared.timing.publish(enabled);
        self.shared.wake.ring();
    }
    pub fn drain(&self) -> Vec<Event> {
        self.shared.drain_into(&mut self.reader.borrow_mut())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.ring();
    }
}
impl Shared {
    fn drain_into(&self, reader: &mut Reader) -> Vec<Event> {
        // This is a committed prefix, not a collection-time guess. A single
        // GPU publisher advances it only after storing the corresponding
        // envelope. No lower-sequence control can arrive after this drain.
        let cutoff = self.published.load(Ordering::Acquire);
        if let Some(latest) = self.art_reply.take() {
            if reader
                .art
                .as_ref()
                .is_none_or(|old| old.sequence < latest.sequence)
            {
                reader.art = Some(latest);
            }
        }
        reader.controls.extend(self.replies.drain());
        reader.release_before(cutoff)
    }
    #[cfg(test)]
    fn drain(&self) -> Vec<Event> {
        self.drain_into(&mut Reader::default())
    }
    // One GPU actor publishes replies. Driver callbacks enqueue scalar
    // results to that actor, never publish replies concurrently. The only
    // other caller is spawn failure, when no actor exists.
    fn reply(&self, event: Event) {
        let sequence = self.reply_sequence.fetch_add(1, Ordering::Relaxed);
        let reply = Reply { sequence, event };
        if matches!(reply.event, Event::ArtCommitted { .. }) {
            self.art_reply.publish(reply);
        } else {
            self.replies.send(reply);
        }
        self.published.store(sequence + 1, Ordering::Release);
        (self.wake_main)();
    }
}

#[derive(Default)]
struct Reader {
    controls: Vec<Reply>,
    art: Option<Arc<Reply>>,
}
impl Reader {
    fn release_before(&mut self, cutoff: u64) -> Vec<Event> {
        let (mut ready, future): (Vec<_>, Vec<_>) = self
            .controls
            .drain(..)
            .partition(|reply| reply.sequence < cutoff);
        self.controls = future;
        if self
            .art
            .as_ref()
            .is_some_and(|reply| reply.sequence < cutoff)
        {
            let latest = self.art.take().expect("checked above");
            match Arc::try_unwrap(latest) {
                Ok(reply) => ready.push(reply),
                Err(reply) => {
                    if let Event::ArtCommitted {
                        base,
                        through,
                        compiled,
                        cursor_art,
                    } = &reply.event
                    {
                        ready.push(Reply {
                            sequence: reply.sequence,
                            event: Event::ArtCommitted {
                                base: *base,
                                through: *through,
                                compiled: compiled.clone(),
                                cursor_art: cursor_art.clone(),
                            },
                        });
                    }
                }
            }
        }
        ready.sort_by_key(|reply| reply.sequence);
        ready.into_iter().map(|reply| reply.event).collect()
    }
}

fn complete_frame(frame: Arc<Frame>, completion: Option<Event>, shared: &Shared) {
    // The UI may immediately mutate its Arc world after this wake-up.
    // Release our immutable CPU snapshot before publishing any completion.
    drop(frame);
    if let Some(event) = completion {
        shared.reply(event);
    }
}

/// What the worker calls immediately before each `Queue::present`.
type PresentNotify = Arc<dyn Fn() + Send + Sync>;

/// Tells `window` a presentation follows (`Window::pre_present_notify`).
/// The worker calls it only immediately before `Queue::present`, never for
/// a frame it may still answer with `Retry`: on Wayland it requests a frame
/// callback, and until that callback arrives winit withholds every
/// `RedrawRequested`. Only a surface commit (a present) makes the
/// compositor send it. Notifying on hand-off, for a frame the worker then
/// retried, left the viewer without redraws until a focus change or resize
/// committed the surface: a game screen that changed on its own (the
/// loading screen's seconds, the main menu) no longer appeared. Elsewhere
/// the call does nothing; on macOS it would only be sent to the main
/// thread, so it is not made there.
fn present_notify(window: Arc<Window>) -> PresentNotify {
    Arc::new(move || {
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        window.pre_present_notify();
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let _ = &window;
    })
}

struct Gpu {
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: TileRenderer,
    bitmap: Option<BitmapTarget>,
    software: bool,
    done: Arc<AtomicBool>,
    world_version: Option<(u64, u64)>,
    present_modes: Vec<wgpu::PresentMode>,
    config_revision: u64,
    instance: Option<wgpu::Instance>,
    surface_lost: bool,
    recovery_requested: bool,
}
struct State {
    diagnostics: Option<NativeDiagnostics>,
    gpu: Option<Gpu>,
    assets: ArtInbox,
    compiled: Compiled,
    cursor_art: Arc<crate::cursor::Art>,
    whole_version: u64,
    assets_version: u64,
    compatible_from: u64,
    pending_art: Option<PendingArt>,
    preparing_art: Option<PreparingArt>,
    target: u64,
    received_through: u64,
    art_error: Option<String>,
    profile: Option<crate::profile::Profile>,
    gpu_profile: Option<crate::profile::Profile>,
    /// Called just before each actual presentation ([`present_notify`]).
    notify_present: Option<PresentNotify>,
}
/// Cumulative application counters. Surface presentation is queued by the GPU
/// actor; `presented` means its matching completion was committed by the UI.
pub(crate) struct AppDiagnostics {
    logged: Instant,
    pub accepted_world: u64,
    pub accepted_replay: u64,
    pub rejected: u64,
    pub presented: u64,
    pub retried: u64,
    pub obsolete: u64,
}
impl AppDiagnostics {
    pub fn new() -> Self {
        Self { logged: Instant::now(), accepted_world: 0, accepted_replay: 0,
            rejected: 0, presented: 0, retried: 0, obsolete: 0 }
    }
    pub fn report_if_due(&mut self, elapsed: Duration, id: u64, logical_canvas: (u32,u32), pixel_box: u32) {
        if self.logged.elapsed() < Duration::from_secs(5) { return; }
        self.logged = Instant::now();
        log::info!("civ3r-profile-v1 {{\"source\":\"app\",\"elapsed_ms\":{},\"frame_id\":{},\"accepted_world\":{},\"accepted_replay\":{},\"rejected\":{},\"presented\":{},\"retried\":{},\"obsolete\":{},\"logical_canvas\":[{},{}],\"pixel_box\":{}}}",
            elapsed.as_millis(), id, self.accepted_world, self.accepted_replay,
            self.rejected, self.presented, self.retried, self.obsolete, logical_canvas.0, logical_canvas.1, pixel_box);
    }
}

struct NativeDiagnostics {
    started: Instant,
    logged: Instant,
    taken: u64,
    retried: u64,
    submitted: u64,
    full_submitted: u64,
    reused_submitted: u64,
    present_enqueued: u64,
    timer_epoch: u64,
    last_timer_attempts: u64,
    last_timer_enabled: bool,
    gaps: GpuFrameGaps,
}
impl NativeDiagnostics {
    fn new() -> Self {
        Self { started: Instant::now(), logged: Instant::now(), taken: 0,
            retried: 0, submitted: 0, full_submitted: 0, reused_submitted: 0,
            present_enqueued: 0, timer_epoch: 0, last_timer_attempts: 0,
            last_timer_enabled: false, gaps: GpuFrameGaps::default() }
    }
    fn observe_timer(&mut self, timer: Option<tile_render::timing::TimerStats>) {
        let attempts = timer.map_or(0, |stats| stats.attempted_frames);
        if timer.is_some() != self.last_timer_enabled || attempts < self.last_timer_attempts {
            self.timer_epoch += 1;
            self.gaps.previous = None;
            self.gaps.last_token = None;
        }
        self.last_timer_enabled = timer.is_some();
        self.last_timer_attempts = attempts;
    }
    fn report_if_due(&mut self, frame: &Frame, gpu: &mut Gpu) {
        if self.logged.elapsed() < Duration::from_secs(5) { return; }
        self.logged = Instant::now();
        let timer = gpu.renderer.timer().map(|t| t.stats());
        let stats = timer.unwrap_or_default();
        self.observe_timer(timer);
        // Observe only once per profile interval. No images or outline pixels
        // are copied or hashed. The bounded sample describes draw commands.
        let observation = frame_observation(frame);
        let adapter = gpu.device.adapter_info();
        log::info!("civ3r-profile-v1 {{\"source\":\"worker\",\"elapsed_ms\":{},\"frame_id\":{},\"config_revision\":{},\"assets_base\":{},\"assets_through\":{},\"taken\":{},\"retried\":{},\"submitted\":{},\"full_submitted\":{},\"reused_submitted\":{},\"present_enqueued\":{},\"surface\":[{},{}],\"present_mode\":\"{:?}\",\"software\":{},\"gpu_vendor\":{},\"gpu_device\":{},\"timer_enabled\":{},\"timer_epoch\":{},\"timer_attempted_frames\":{},\"timer_prepared_samples\":{},\"timer_submitted_samples\":{},\"timer_collected_samples\":{},\"timer_empty_frames\":{},\"timer_discarded_frames\":{},\"timer_over_budget_frames\":{},\"timer_readback_busy_frames\":{},\"timer_map_failed_frames\":{},\"timer_mapped_range_failed_frames\":{},{} }}",
            self.started.elapsed().as_millis(), frame.id, frame.config_revision, frame.base, frame.through,
            self.taken, self.retried, self.submitted, self.full_submitted, self.reused_submitted, self.present_enqueued,
            gpu.config.width, gpu.config.height, gpu.config.present_mode, gpu.software, adapter.vendor, adapter.device,
            timer.is_some(), self.timer_epoch, stats.attempted_frames, stats.prepared_samples,
            stats.submitted_samples, stats.collected_samples, stats.empty_frames, stats.discarded_frames,
            stats.over_budget_frames, stats.readback_busy_frames, stats.map_failed_frames, stats.mapped_range_failed_frames,
            format_args!("{},{}", self.gaps.fields(), observation));
    }
}

fn fold_observation(hash: &mut u64, words: impl IntoIterator<Item = u32>) {
    for word in words { *hash = (*hash ^ u64::from(word)).wrapping_mul(0x100_0000_01b3); }
}

/// Counts refer to source commands, including ordinary/zoomed twins; they do
/// not claim that every command passed the vertex shader's visibility rules.
fn frame_observation(frame: &Frame) -> String {
    use std::fmt::Write as _;
    let mut camera_hash = 0xcbf2_9ce4_8422_2325u64;
    let mut cameras = String::from("[");
    for (i, c) in frame.cameras.iter().enumerate() {
        if i != 0 { cameras.push(','); }
        let clip = c.target_clip.map_or_else(|| "null".into(), |v| format!("{v:?}"));
        let cull = c.cull.map_or_else(|| "null".into(), |(a,b,d,e)| format!("[{a},{b},{d},{e}]"));
        let _ = write!(cameras, "{{\"xy\":[{},{}],\"origin\":[{},{}],\"viewport\":[{},{}],\"screen_shift\":[{},{}],\"zoom\":[{},{}],\"cells\":{},\"zoomed_art\":{},\"stacked\":{},\"downscale\":{},\"binding\":{},\"target_clip\":{},\"cull\":{}}}",
            c.x,c.y,c.origin.0,c.origin.1,c.viewport.0,c.viewport.1,c.screen_shift.0,c.screen_shift.1,
            c.zoom.0,c.zoom.1,c.cells,c.zoomed_art,c.stacked,c.downscale,c.binding,clip,cull);
        fold_observation(&mut camera_hash, [c.x as u32,c.y as u32,c.origin.0 as u32,c.origin.1 as u32,
            c.viewport.0,c.viewport.1,c.screen_shift.0 as u32,c.screen_shift.1 as u32,c.zoom.0,c.zoom.1,
            c.cells as u32,c.zoomed_art as u32,c.stacked as u32,c.downscale,c.binding]);
    }
    cameras.push(']');
    let (mut sprites,mut glyphs,mut screen_commands,mut screen_glyphs,mut bars,mut sample) = (0usize,0usize,0usize,0usize,0usize,0usize);
    let mut glyph_hash = 0xcbf2_9ce4_8422_2325u64;
    let mut command_hash = 0xcbf2_9ce4_8422_2325u64;
    if let Some(world) = &frame.world {
        for (i,s) in world.free.iter().enumerate() {
            screen_commands += usize::from(s.screen);
            if i < 256 { fold_observation(&mut command_hash,[s.set.raw(),s.x as u32,s.y as u32,s.screen as u32,s.view.unwrap_or(u32::MAX),s.after_pass as u32,s.category]); }
            if let Some(g) = s.text {
                glyphs += 1; screen_glyphs += usize::from(s.screen);
                bars += usize::from(matches!(g.shape,tile_render::GlyphShape::Bar {..}));
                if sample < 256 {
                    let (kind,a,b) = match g.shape { tile_render::GlyphShape::Glyph(index) => (0,index,0), tile_render::GlyphShape::Bar {top,height} => (1,top as u32,height as u32) };
                    fold_observation(&mut glyph_hash,[g.font,kind,a,b,s.x as u32,s.y as u32,g.em64,g.advance as u32,g.bold as u32,g.italic as u32,
                        u32::from_be_bytes([0,g.color[0],g.color[1],g.color[2]]),g.key.map_or(u32::MAX,|k|u32::from_be_bytes([0,k[0],k[1],k[2]])),s.screen as u32,s.view.unwrap_or(u32::MAX)]);
                    sample += 1;
                }
            } else { sprites += 1; }
        }
    }
    format!("\"world_packet\":{},\"tick\":{},\"canvas\":[{},{}],\"target\":[{},{}],\"present_zoom\":[{},{}],\"present_at\":[{},{}],\"overlay_from\":{},\"copy_count\":{},\"camera_count\":{},\"camera_hash\":\"{camera_hash:016x}\",\"command_sample_hash\":\"{command_hash:016x}\",\"glyph_sample_hash\":\"{glyph_hash:016x}\",\"glyph_sample_count\":{},\"sprite_commands\":{},\"glyph_commands\":{},\"bar_commands\":{},\"screen_commands\":{},\"screen_glyph_commands\":{},\"marker_commands\":{},\"cameras\":{}",
        frame.world.is_some(),frame.tick,frame.present.canvas.0,frame.present.canvas.1,frame.present.target.0,frame.present.target.1,
        frame.present.zoom.0,frame.present.zoom.1,frame.present.at.0,frame.present.at.1,frame.overlay_from,frame.copies.len(),frame.cameras.len(),
        sample,sprites,glyphs,bars,screen_commands,screen_glyphs,frame.markers.len(),cameras)
}

fn run(init: Init, shared: &Shared) -> Result<(), String> {
    let mut state = State::new(&init, shared)?;
    let gpu = state.gpu.as_ref().expect("initialized");
    shared.reply(Event::Ready {
        config: gpu.config.clone(),
        software: gpu.software,
        present_modes: gpu.present_modes.clone(),
        base: init.base,
        through: init.through,
    });
    let mut last_frame = 0;
    let profiling = init.profile;
    let mut timing = profiling;
    drop(init);
    while !shared.stop.load(Ordering::Acquire) {
        if let Some(profile) = &mut state.profile {
            profile.round_start();
        }
        if let Some(enabled) = shared.timing.get() {
            let enabled = enabled || profiling;
            if enabled != timing {
                let gpu = state.gpu.as_mut().expect("initialized");
                if enabled {
                    gpu.renderer.enable_timing(&gpu.device, &gpu.queue);
                } else {
                    gpu.renderer.disable_timing();
                }
                timing = enabled;
            }
        }
        if let Some(config) = shared.config.take() {
            state.configure(&config, shared)?;
        }
        if let Some(art) = shared.art.take() {
            state.receive_art(&art);
        }
        let (base, through) = (state.whole_version, state.assets_version);
        let art_started = Instant::now();
        state.upload_art();
        if let Some(error) = state.art_error.take() {
            return Err(error);
        }
        if let Some(profile) = &mut state.profile {
            profile.add("art transfers", art_started.elapsed());
        }
        if (base, through) != (state.whole_version, state.assets_version) {
            state.gpu.as_mut().expect("initialized").world_version = None;
            shared.reply(Event::ArtCommitted {
                base: state.whole_version,
                through: state.assets_version,
                compiled: Arc::new(state.compiled.without_pixels()),
                cursor_art: state.cursor_art.clone(),
            });
        }
        state.poll(shared)?;
        if let Some(frame) = shared.frames.take() {
            if frame.id > last_frame {
                last_frame = frame.id;
                if let Some(d) = &mut state.diagnostics { d.taken += 1; }
                let draw_started = Instant::now();
                let completion = state.draw(&frame, shared)?;
                if matches!(&completion, Some(Event::Retry { .. })) {
                    if let Some(d) = &mut state.diagnostics { d.retried += 1; }
                }
                if let Some(profile) = &mut state.profile {
                    profile.add("draw (all)", draw_started.elapsed());
                    if !matches!(&completion, Some(Event::Retry { .. })) {
                        let reused = state.gpu.as_ref().expect("initialized").renderer.reused();
                        profile.event(
                            if reused {
                                "frames drawn on the last picture"
                            } else {
                                "frames drawn whole"
                            },
                            1,
                        );
                    }
                }
                complete_frame(frame, completion, shared);
            }
        }
        while let Some((id, at)) = shared.completions.try_recv() {
            shared.reply(Event::GpuCompleted { id, at });
        }
        if let Some(profile) = &mut state.profile {
            profile.round_end();
            if let Some(report) = profile.report() {
                log::info!("{report}");
            }
        }
        // This actor alone parks. The main event loop never waits for it.
        shared.wake.wait(Duration::from_millis(4));
    }
    Ok(())
}
/// Device timestamp gaps include presentation pacing, uploads and other queue
/// work. They are not a measurement of CPU starvation. Missing or reordered
/// samples and timer epochs must never be bridged with a guessed gap.
#[derive(Default)]
struct GpuFrameGaps {
    previous: Option<tile_render::timing::FrameStamp>,
    last_token: Option<u64>,
    observed: u64,
    adjacent: u64,
    discontinuous: u64,
    out_of_order: u64,
}
impl GpuFrameGaps {
    fn observe(&mut self, frame: &tile_render::timing::FrameTimes) -> Option<f64> {
        self.observed += 1;
        if self.previous.is_some_and(|old| frame.stamp.frame_id <= old.frame_id) {
            self.out_of_order += 1;
            return None;
        }
        let gap = self.previous.and_then(|old| frame.stamp.gap_after(old));
        if self.previous.is_some() {
            if gap.is_some() { self.adjacent += 1; }
            else { self.discontinuous += 1; }
        }
        self.previous = Some(frame.stamp);
        self.last_token = frame.frame_token;
        gap
    }
    fn fields(&self) -> String {
        // Preserve full-width timestamp ticks for JSON readers using doubles.
        let last = self.previous.map_or_else(|| "null".into(), |s| format!(
            "{{\"frame_id\":{},\"start_ticks\":\"{}\",\"end_ticks\":\"{}\",\"period_ns\":{},\"frame_token\":{}}}",
            s.frame_id, s.start, s.end, s.period_ns,
            self.last_token.map_or_else(|| "null".into(), |id| id.to_string())));
        format!("\"gpu_gap_observed\":{},\"gpu_gap_adjacent\":{},\"gpu_gap_discontinuous\":{},\"gpu_gap_out_of_order\":{},\"gpu_last_stamp\":{}",
            self.observed, self.adjacent, self.discontinuous, self.out_of_order, last)
    }
}

/// Each delayed result is a separate GPU round, even when several arrive
/// together. Host collection intervals do not describe GPU execution pacing.
fn profile_gpu_frames(
    profile: &mut crate::profile::Profile,
    frames: &[tile_render::timing::FrameTimes],
    gaps: &mut GpuFrameGaps,
) {
    for frame in frames {
        if let Some(gap) = gaps.observe(frame) {
            profile.add_ms("gpu inter-frame gap", gap);
        }
        profile.add_ms("gpu frame span", frame.span);
        profile.add_ms(
            "gpu passes sum",
            frame.passes.iter().map(|(_, ms)| ms).sum(),
        );
        profile.count("gpu passes", frame.passes.len());
        for (label, ms) in &frame.passes {
            profile.add_ms(&format!("gpu {label}"), *ms);
        }
        for (label, ms) in &frame.increments {
            profile.add_ms(&format!("gpu increment {label}"), *ms);
        }
        profile.round_end();
    }
}

impl State {
    fn new(init: &Init, shared: &Shared) -> Result<Self, String> {
        if init.bitmap && !cfg!(target_os = "macos") {
            return Err("bitmap diagnostic presentation requires macOS".into());
        }
        let instance = wgpu::Instance::new(tile_render::gpu::instance_descriptor());
        let surface = if init.bitmap {
            None
        } else {
            shared.reply(Event::SurfaceNeeded {
                instance: instance.clone(),
            });
            loop {
                if shared.stop.load(Ordering::Acquire) {
                    return Err("GPU initialization cancelled".into());
                }
                if let Some(surface) = shared.surfaces.try_recv() {
                    break Some(surface?);
                }
                shared.wake.wait(Duration::from_millis(4));
            }
        };
        let (adapter, device, queue, config) = pollster::block_on(async {
            let adapter = tile_render::gpu::select_adapter(&instance, surface.as_ref(), None)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(surface) = &surface {
                match crate::show_with(
                    surface,
                    adapter,
                    winit::dpi::PhysicalSize::new(init.size.0, init.size.1),
                )
                .await
                {
                    Ok(shown) => Ok(shown),
                    Err((adapter, error)) => {
                        log::warn!(
                            "viewer: {} cannot show this window ({error}); trying its default adapter",
                            adapter.get_info().name
                        );
                        let adapter = tile_render::gpu::default_adapter(&instance, Some(surface))
                            .await
                            .map_err(|e| e.to_string())?;
                        crate::show_with(
                            surface,
                            adapter,
                            winit::dpi::PhysicalSize::new(init.size.0, init.size.1),
                        )
                        .await
                        .map_err(|(_, e)| e)
                    }
                }
            } else {
                let (device, queue) = tile_render::gpu::request_device(&adapter)
                    .await
                    .map_err(|e| e.to_string())?;
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    width: init.size.0.max(1),
                    height: init.size.1.max(1),
                    present_mode: wgpu::PresentMode::AutoVsync,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                    color_space: wgpu::SurfaceColorSpace::Auto,
                };
                Ok((adapter, device, queue, config))
            }
        })?;
        log::info!("viewer GPU worker adapter: {:?}", adapter.get_info());
        let present_modes = surface.as_ref().map_or_else(
            || vec![wgpu::PresentMode::Fifo],
            |surface| surface.get_capabilities(&adapter).present_modes,
        );
        let software = init.bitmap || adapter.get_info().device_type == wgpu::DeviceType::Cpu;
        let mut renderer = TileRenderer::new(&device, config.format);
        if init.profile {
            renderer.enable_timing(&device, &queue);
        }
        renderer.set_atlas(&device, &queue, &init.initial.atlas);
        let bitmap = init
            .bitmap
            .then(|| BitmapTarget::new(&device, &config))
            .transpose()?;
        let assets = ArtInbox::new(Assets::with_whole(Some(init.initial.clone()), init.base));
        assets.uploaded(init.base, init.through);
        Ok(Self {
            diagnostics: init.profile.then(NativeDiagnostics::new),
            gpu: Some(Gpu {
                surface,
                device,
                queue,
                config,
                renderer,
                bitmap,
                software,
                done: Arc::new(AtomicBool::new(true)),
                world_version: None,
                present_modes,
                config_revision: 0,
                instance: Some(instance),
                surface_lost: false,
                recovery_requested: false,
            }),
            assets,
            compiled: init.initial.without_pixels(),
            cursor_art: Arc::new(crate::cursor::Art::of(&init.initial)),
            whole_version: init.base,
            assets_version: init.through,
            compatible_from: init.through,
            pending_art: None,
            preparing_art: None,
            target: init.through,
            received_through: init.through,
            art_error: None,
            profile: init
                .profile
                .then(|| crate::profile::Profile::new("viewer GPU worker", Duration::from_secs(5))),
            gpu_profile: init.profile.then(|| {
                crate::profile::Profile::new("viewer GPU execution", Duration::from_secs(5))
            }),
            notify_present: Some(present_notify(init.window.clone())),
        })
    }
    fn receive_art(&mut self, art: &ArtSnapshot) {
        let current = self.assets.generation();
        if current > art.base {
            return;
        }
        if current != art.base {
            let Some(whole) = &art.whole else {
                return;
            };
            self.assets
                .set_with_reserve(whole.clone(), art.base, art.reserve.0, art.reserve.1);
            self.assets.receive();
            self.received_through = art.base;
        }
        for (version, growth) in &art.growths {
            if *version > self.received_through {
                self.assets.grow(growth.clone(), *version);
                self.received_through = *version;
            }
        }
        // The snapshot repeats until new art arrives. Admit capacity locally
        // once per epoch rather than enqueueing the same reserve every frame.
        self.assets.reserve_more(art.reserve);
        self.target = art.target;
    }
    fn configure(&mut self, requested: &Config, shared: &Shared) -> Result<(), String> {
        let gpu = self.gpu.as_mut().expect("initialized");
        if requested.revision < gpu.config_revision {
            return Ok(());
        }
        gpu.config_revision = requested.revision;
        if let Some(bitmap) = &mut gpu.bitmap {
            if let Some(pending) = bitmap.pending.take() {
                bitmap.readback.unmap();
                shared.reply(Event::Retry { id: pending.id });
            }
        }
        gpu.config.width = requested.width.max(1);
        gpu.config.height = requested.height.max(1);
        gpu.config.present_mode = requested.present_mode;
        gpu.config.desired_maximum_frame_latency = requested.latency;
        if let Some(surface) = &gpu.surface {
            surface.configure(&gpu.device, &gpu.config);
        }
        if gpu.bitmap.is_some() {
            gpu.bitmap = Some(BitmapTarget::new(&gpu.device, &gpu.config)?);
        }
        Ok(())
    }
    fn poll(&mut self, shared: &Shared) -> Result<(), String> {
        let gpu = self.gpu.as_mut().expect("initialized");
        if gpu.surface_lost {
            if !gpu.recovery_requested {
                shared.reply(Event::SurfaceNeeded {
                    instance: gpu
                        .instance
                        .as_ref()
                        .expect("hardware surface instance")
                        .clone(),
                });
                gpu.recovery_requested = true;
            }
            if let Some(surface) = shared.surfaces.try_recv() {
                let surface = surface?;
                surface.configure(&gpu.device, &gpu.config);
                gpu.surface = Some(surface);
                gpu.surface_lost = false;
                gpu.recovery_requested = false;
            }
        }
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        if let Some(d) = &mut self.diagnostics {
            d.observe_timer(gpu.renderer.timer().map(|timer| timer.stats()));
        }
        if let Some(timer) = gpu.renderer.timer() {
            let frames = timer.collect(&gpu.device);
            if let Some(profile) = &mut self.gpu_profile {
                let gaps = &mut self.diagnostics.as_mut().expect("profile diagnostics").gaps;
                profile_gpu_frames(profile, &frames, gaps);
                if let Some(report) = profile.report() {
                    log::info!("{report}");
                }
            }
            if !frames.is_empty() {
                shared.reply(Event::Timings(frames));
            }
        }
        if let Some(bitmap) = &mut gpu.bitmap {
            bitmap.poll(shared)?;
        }
        Ok(())
    }
    fn draw(&mut self, frame: &Frame, shared: &Shared) -> Result<Option<Event>, String> {
        let gpu = self.gpu.as_mut().expect("initialized");
        if !accepts_prefix(
            self.whole_version,
            self.compatible_from,
            self.assets_version,
            frame.base,
            frame.through,
        ) {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        if gpu.surface_lost {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        if !gpu.renderer.control_frame_available() {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        if frame.config_revision != gpu.config_revision {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        if (gpu.software && !gpu.done.load(Ordering::Acquire))
            || gpu.bitmap.as_ref().is_some_and(|b| b.pending.is_some())
        {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        let acquired = match &gpu.surface {
            Some(surface) => {
                let acquire_started = self.profile.as_ref().map(|_| Instant::now());
                let result = surface.get_current_texture();
                if let (Some(profile), Some(started)) = (&mut self.profile, acquire_started) {
                    profile.add("surface acquire (host)", started.elapsed());
                }
                match result {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
                    wgpu::CurrentSurfaceTexture::Lost => {
                        gpu.surface = None;
                        gpu.surface_lost = true;
                        gpu.recovery_requested = false;
                        if let Some(timer) = gpu.renderer.timer() {
                            timer.discard_frame();
                        }
                        return Ok(Some(Event::Retry { id: frame.id }));
                    }
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&gpu.device, &gpu.config);
                        if let Some(timer) = gpu.renderer.timer() {
                            timer.discard_frame();
                        }
                        return Ok(Some(Event::Retry { id: frame.id }));
                    }
                    _ => {
                        if let Some(timer) = gpu.renderer.timer() {
                            timer.discard_frame();
                        }
                        return Ok(Some(Event::Retry { id: frame.id }));
                    }
                }
            },
            None => None,
        };
        let prepare = Instant::now();
        let target = acquired
            .as_ref()
            .map(|f| &f.texture)
            .or_else(|| gpu.bitmap.as_ref().map(|b| &b.target))
            .expect("presentation target");
        if let Some(world) = &frame.world {
            let expected = Some((frame.base, frame.previous_world_version));
            if frame.replace_world || gpu.world_version != expected {
                gpu.renderer.set_world(&gpu.device, world);
            } else {
                if frame.dirty != Dirty::None {
                    gpu.renderer
                        .update_cells(&gpu.device, &gpu.queue, world, &frame.dirty);
                }
                if frame.free_changed {
                    gpu.renderer.update_free(&gpu.device, &gpu.queue, world);
                }
            }
            gpu.world_version = Some((frame.base, frame.world_version));
            gpu.renderer
                .set_markers(&gpu.device, &gpu.queue, &frame.markers);
            crate::apply_patches(
                &mut gpu.renderer,
                &gpu.queue,
                &self.compiled,
                frame.patches.iter(),
            );
        }
        gpu.renderer.set_host_tick_ms(frame.host_tick_ms);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let rendered = if frame.world.is_some() {
            gpu.renderer.render_layered(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                target,
                &frame.cameras,
                frame.overlay_from,
                &frame.copies,
                frame.tick,
                frame.clear,
                frame.present,
            )
        } else {
            gpu.renderer.render_cached_picture(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                target,
                &frame.copies,
                frame.clear,
                frame.present,
            )
        };
        if !rendered {
            if let Some(timer) = gpu.renderer.timer() {
                timer.discard_frame();
            }
            return Ok(Some(Event::Retry { id: frame.id }));
        }
        if let Some(timer) = gpu.renderer.timer() {
            timer.finish_frame(&mut encoder, frame.id);
        }
        if let Some(bitmap) = &gpu.bitmap {
            bitmap.copy_readback(&mut encoder);
        }
        let commands = encoder.finish();
        let worker_prepare = prepare.elapsed();
        if let Some(profile) = &mut self.profile {
            profile.add("frame encode (host)", worker_prepare);
        }
        let submit_started = self.profile.as_ref().map(|_| Instant::now());
        gpu.queue.submit([commands]);
        if let (Some(profile), Some(started)) = (&mut self.profile, submit_started) {
            profile.add("queue submit (host)", started.elapsed());
        }
        gpu.renderer.controls_submitted();
        if let Some(d) = &mut self.diagnostics {
            d.submitted += 1;
            if frame.world.is_none() || gpu.renderer.reused() { d.reused_submitted += 1; }
            else { d.full_submitted += 1; }
        }
        if crate::log_latency() {
            let completions = shared.completions.clone();
            let id = frame.id;
            gpu.queue.on_submitted_work_done(move || {
                completions.send((id, Instant::now()));
            });
        }
        if let Some(timer) = gpu.renderer.timer() {
            timer.submitted();
        }
        if gpu.software {
            gpu.done.store(false, Ordering::Release);
            let done = gpu.done.clone();
            gpu.queue
                .on_submitted_work_done(move || done.store(true, Ordering::Release));
        }
        let completion = if let Some(texture) = acquired {
            let present_started = self.profile.as_ref().map(|_| Instant::now());
            // Only here, where a commit follows: see `present_notify`.
            if let Some(notify) = &self.notify_present { notify(); }
            gpu.queue.present(texture);
            if let (Some(profile), Some(started)) = (&mut self.profile, present_started) {
                profile.add("present enqueue (host)", started.elapsed());
            }
            if let Some(d) = &mut self.diagnostics { d.present_enqueued += 1; }
            Some(Event::Presented { id: frame.id, worker_prepare, rendered })
        } else {
            if let Some(bitmap) = &mut gpu.bitmap { bitmap.begin(frame.id, worker_prepare, rendered); }
            None
        };
        if let Some(d) = &mut self.diagnostics { d.report_if_due(frame, gpu); }
        Ok(completion)
    }
    fn upload_art(&mut self) -> bool {
        use tile_render::renderer::AtlasTransferError;
        let Some(gpu) = self.gpu.as_mut() else {
            return false;
        };
        gpu.renderer.poll_atlas_transfers(&gpu.device);
        let (version, whole) = self.assets.whole();
        if self
            .pending_art
            .as_ref()
            .is_some_and(|p| p.payload.base() != version)
        {
            gpu.renderer.cancel_atlas_transfer(&gpu.queue);
            self.pending_art = None;
        }
        if let Some(preparing) = &mut self.preparing_art {
            preparing.renderer.poll_atlas_transfers(&gpu.device);
            if preparing.base != version && !preparing.obsolete {
                preparing.cancel(&gpu.queue);
            }
            if preparing.obsolete {
                // Dropping an in-flight preparer and immediately allocating
                // another would conceal abandoned GPU arrays from the cap.
                if !preparing.renderer.atlas_transfer_idle() {
                    return false;
                }
                self.preparing_art = None;
            }
        }
        if version != self.whole_version {
            if !gpu.renderer.atlas_transfer_idle() {
                return false;
            }
            if self.preparing_art.is_none() {
                let Some(compiled) = whole else { return false };
                self.preparing_art = Some(PreparingArt::new(&gpu.renderer, version, compiled));
            }
            let preparing = self.preparing_art.as_mut().expect("created above");
            if preparing.failed {
                // A failed replacement must not keep an unused intermediate
                // atlas after its callbacks have released pending work.
                if preparing.renderer.atlas_transfer_idle()
                    && preparing.renderer.atlas_resident_bytes() > 0
                {
                    preparing.renderer = gpu.renderer.atlas_preparer();
                }
                return false;
            }
            let target = self.target;
            let ready = match preparing.advance(
                &gpu.device,
                &gpu.queue,
                &gpu.renderer,
                &mut self.assets,
                target,
            ) {
                Ok((bytes, ready)) => {
                    if let Some(profile) = &mut self.profile {
                        profile.count("art transfer bytes", bytes as usize);
                    }
                    ready
                }
                Err(e) => {
                    log::error!(
                        "viewer: replacement art {} could not be prepared: {e}",
                        preparing.base
                    );
                    render_errors::report(render_errors::Kind::Atlas, "replacement art could not be prepared", format_args!("art {}: {e}", preparing.base));
                    preparing.renderer.cancel_atlas_transfer(&gpu.queue);
                    preparing.pending = None;
                    preparing.failed = true;
                    self.art_error =
                        Some(format!("replacement atlas {} failed: {e}", preparing.base));
                    return false;
                }
            };
            if !ready {
                return false;
            }
            // A replacement published from here on is taken next frame.
            if self.assets.generation() != version {
                preparing.cancel(&gpu.queue);
                return false;
            }
            match gpu.renderer.adopt_prepared_atlas(
                &gpu.device,
                &gpu.queue,
                &mut preparing.renderer,
            ) {
                Ok(()) => {}
                Err(AtlasTransferError::Busy) => return false,
                Err(e) => {
                    log::error!(
                        "viewer: replacement art {} could not be activated: {e}",
                        preparing.base
                    );
                    render_errors::report(render_errors::Kind::Atlas, "replacement art could not be activated", format_args!("art {}: {e}", preparing.base));
                    preparing.failed = true;
                    self.art_error =
                        Some(format!("replacement atlas {} failed: {e}", preparing.base));
                    return false;
                }
            }
            let prepared = self.preparing_art.take().expect("adopted above");
            self.cursor_art = Arc::new(crate::cursor::Art::of(&prepared.whole));
            self.compiled = prepared.compiled;
            self.whole_version = version;
            self.assets_version = prepared.through;
            self.compatible_from = prepared.through;
            self.assets.uploaded(version, prepared.through);
            log::info!(
                "viewer: new assets through generation {} ({} sprites, prepared over {} ms in {} frames)",
                prepared.through,
                self.compiled.sprite_names.len(),
                prepared.since.elapsed().as_millis(),
                prepared.frames
            );
            return true;
        }
        // Growth of the drawable whole keeps existing ids usable, so it
        // can continue using the renderer's single-candidate transfer path.
        if self.pending_art.is_none() {
            if let Some((generation, growth)) = self.assets.take_growth(version) {
                let reserve = self.assets.take_reserve(version).unwrap_or_default();
                self.pending_art = Some(PendingArt::new(
                    PendingArtPayload::Growth {
                        base: version,
                        version: generation,
                        growth,
                    },
                    reserve,
                ));
            } else if let Some(reserve) = self.assets.take_reserve(version) {
                self.pending_art = Some(PendingArt::new(
                    PendingArtPayload::Reserve { base: version },
                    reserve,
                ));
            }
        }
        let Some(p) = self.pending_art.as_mut().filter(|p| !p.failed) else {
            return false;
        };
        if !p.started {
            match gpu
                .renderer
                .begin_atlas_transfer(&gpu.device, p.payload.source(), p.reserve)
            {
                Ok(()) => p.started = true,
                Err(AtlasTransferError::Busy) => return false,
                Err(e) => {
                    log::error!(
                        "viewer: cannot prepare atlas generation {}: {e}",
                        p.payload.base()
                    );
                    render_errors::report(render_errors::Kind::Atlas, "atlas generation could not be prepared", format_args!("generation {}: {e}", p.payload.base()));
                    p.failed = true;
                    self.art_error = Some(format!("atlas {} failed: {e}", p.payload.base()));
                    return false;
                }
            }
        }
        p.frames += 1;
        let ready = match gpu.renderer.advance_atlas_transfer(
            &gpu.device,
            &gpu.queue,
            p.payload.source(),
            ATLAS_PART,
        ) {
            Ok(progress) => {
                if let Some(profile) = &mut self.profile {
                    profile.count("art transfer bytes", progress.bytes as usize);
                }
                progress.ready
            }
            Err(AtlasTransferError::Invalidated) => {
                gpu.renderer.cancel_atlas_transfer(&gpu.queue);
                p.started = false;
                return false;
            }
            Err(e) => {
                log::error!(
                    "viewer: atlas generation {} transfer failed: {e}",
                    p.payload.base()
                );
                render_errors::report(render_errors::Kind::Atlas, "atlas transfer failed", format_args!("generation {}: {e}", p.payload.base()));
                gpu.renderer.cancel_atlas_transfer(&gpu.queue);
                p.failed = true;
                self.art_error = Some(format!("atlas {} failed: {e}", p.payload.base()));
                false
            }
        };
        if !ready {
            return false;
        }
        if self.assets.generation() != version {
            gpu.renderer.cancel_atlas_transfer(&gpu.queue);
            self.pending_art = None;
            return false;
        }
        let committed = gpu.renderer.commit_atlas_transfer(&gpu.device, &gpu.queue);
        if !committed {
            return false;
        }
        let p = self.pending_art.take().expect("committed transfer");
        match p.payload {
            PendingArtPayload::Growth {
                version, growth, ..
            } => {
                if !self.compiled.apply_with_prefix_check(&growth) {
                    self.compatible_from = version;
                }
                self.assets_version = version;
                self.assets.uploaded(self.whole_version, version);
                log::info!(
                    "viewer: grown by {} sprites ({} KiB of pixels) over {} ms in {} frames; {} sprites",
                    growth.atlas.sprites.len(),
                    growth.atlas.bytes() >> 10,
                    p.since.elapsed().as_millis(),
                    p.frames,
                    self.compiled.atlas.sprites.len()
                );
                true
            }
            PendingArtPayload::Reserve { .. } => {
                let (rgba, index) = gpu.renderer.atlas_room();
                log::info!(
                    "viewer: atlas room for {rgba} colour and {index} index pages prepared over {} ms in {} frames",
                    p.since.elapsed().as_millis(),
                    p.frames
                );
                false
            }
            PendingArtPayload::Whole { .. } => {
                unreachable!("whole replacements use a separate preparer")
            }
        }
    }
}

fn accepts_prefix(base: u64, from: u64, through: u64, frame_base: u64, frame_through: u64) -> bool {
    frame_base == base && frame_through >= from && frame_through <= through
}
#[cfg(test)]
fn advance_prefix(from: &mut u64, before: &Compiled, after: &Compiled, through: u64) {
    if !crate::scene_prefix::preserves_render_prefix(before, after) {
        *from = through;
    }
}

struct BitmapPending {
    id: u64,
    prepare: Duration,
    rendered: bool,
    ready: Arc<AtomicBool>,
    result: Arc<Latest<Result<(), String>>>,
}
struct BitmapTarget {
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    row_bytes: u32,
    size: (u32, u32),
    pending: Option<BitmapPending>,
}
impl BitmapTarget {
    fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Result<Self, String> {
        let row_bytes = config
            .width
            .checked_mul(4)
            .and_then(|n| n.checked_add(255))
            .map(|n| n / 256 * 256)
            .ok_or("bitmap row overflow")?;
        let bytes = u64::from(row_bytes) * u64::from(config.height);
        if bytes == 0 || bytes > 64 * 1024 * 1024 {
            return Err("bitmap diagnostic output exceeds 64 MiB".into());
        }
        if config.width > device.limits().max_texture_dimension_2d
            || config.height > device.limits().max_texture_dimension_2d
        {
            return Err("bitmap exceeds GPU texture dimensions".into());
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewer diagnostic target"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer diagnostic readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            target,
            readback,
            row_bytes,
            size: (config.width, config.height),
            pending: None,
        })
    }
    fn copy_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row_bytes),
                    rows_per_image: Some(self.size.1),
                },
            },
            self.target.size(),
        );
    }
    fn begin(&mut self, id: u64, prepare: Duration, rendered: bool) {
        let result = Arc::new(Latest::new());
        let ready = Arc::new(AtomicBool::new(false));
        let output = result.clone();
        let flag = ready.clone();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |value| {
                output.publish(value.map_err(|e| e.to_string()));
                flag.store(true, Ordering::Release);
            });
        self.pending = Some(BitmapPending {
            id,
            prepare,
            rendered,
            ready,
            result,
        });
    }
    fn poll(&mut self, shared: &Shared) -> Result<(), String> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.ready.load(Ordering::Acquire))
        {
            return Ok(());
        }
        let pending = self.pending.take().expect("ready");
        let result = pending.result.load().expect("callback published");
        if let Err(error) = result.as_ref() {
            self.readback.unmap();
            return Err(format!("bitmap readback: {error}"));
        }
        let pixels = {
            let source = self
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(|e| e.to_string())?;
            let mut output = Vec::with_capacity(self.size.0 as usize * self.size.1 as usize);
            for row in source.chunks_exact(self.row_bytes as usize) {
                for rgba in row[..self.size.0 as usize * 4].chunks_exact(4) {
                    output.push(
                        u32::from(rgba[0]) << 16 | u32::from(rgba[1]) << 8 | u32::from(rgba[2]),
                    );
                }
            }
            output
        };
        self.readback.unmap();
        shared.reply(Event::Bitmap {
            id: pending.id,
            width: self.size.0,
            height: self.size.1,
            pixels,
            worker_prepare: pending.prepare,
            rendered: pending.rendered,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn shared(wake_main: Arc<dyn Fn() + Send + Sync>) -> Arc<Shared> {
        Arc::new(Shared {
            frames: Latest::new(),
            art: Latest::new(),
            config: Latest::new(),
            replies: Mailbox::new(),
            art_reply: Latest::new(),
            reply_sequence: AtomicU64::new(0),
            published: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            wake: Wake::new(),
            wake_main,
            surfaces: Mailbox::new(),
            timing: Latest::new(),
            completions: Arc::new(Mailbox::new()),
        })
    }
    fn frame(id: u64) -> Frame {
        Frame {
            id,
            config_revision: 0,
            base: 1,
            through: 1,
            world: None,
            world_version: id,
            previous_world_version: id.saturating_sub(1),
            replace_world: false,
            dirty: Dirty::None,
            free_changed: false,
            markers: vec![],
            cameras: vec![],
            copies: vec![],
            overlay_from: 0,
            tick: 0,
            host_tick_ms: 0,
            clear: [24, 40, 56, 255],
            present: Present::unscaled((4, 4)),
            patches: vec![],
            background: tile_render::BackgroundSampling::Dither,
            main_prepare: Duration::ZERO,
        }
    }
    #[test]
    fn blocked_gpu_cannot_hold_cursor_updates_or_retain_old_frame_requests() {
        let (wake_tx, wake_rx) = mpsc::channel();
        let state = shared(Arc::new(move || {
            let _ = wake_tx.send(());
        }));
        let worker = Worker {
            shared: state.clone(),
            reader: RefCell::default(),
        };
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owned = state.clone();
        let actor = std::thread::spawn(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            owned.frames.load().unwrap().id
        });
        entered_rx.recv().unwrap();
        let mut weak = Vec::new();
        let mut cursor = crate::system_cursor::SystemCursor::default();
        for id in 1..100 {
            let candidate = Arc::new(frame(id));
            weak.push(Arc::downgrade(&candidate));
            state.frames.publish_arc(candidate);
            cursor.refresh(
                &scene::Scene {
                    seq: id,
                    assets: 1,
                    cursor: scene::Cursor::Sprite(id as u32),
                    ..Default::default()
                },
                1,
                1,
                Arc::default(),
            );
            (state.wake_main)();
        }
        assert_eq!(cursor.cursor(), scene::Cursor::Sprite(99));
        assert_eq!(wake_rx.try_iter().count(), 99);
        assert!(
            weak[..98].iter().all(|old| old.upgrade().is_none()),
            "overwritten frames must release their snapshots while GPU is blocked"
        );
        assert!(weak[98].upgrade().is_some());
        release_tx.send(()).unwrap();
        assert_eq!(actor.join().unwrap(), 99);
        drop(worker);
    }
    #[test]
    fn shutdown_never_joins_a_worker_waiting_for_the_main_thread() {
        let state = shared(Arc::new(|| {}));
        let worker = Worker {
            shared: state.clone(),
            reader: RefCell::default(),
        };
        let (release_tx, release_rx) = mpsc::channel();
        let actor = std::thread::spawn(move || release_rx.recv().unwrap());
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let closer = std::thread::spawn(move || {
            drop(worker);
            dropped_tx.send(()).unwrap();
        });
        let closed = dropped_rx.recv_timeout(Duration::from_secs(2));
        release_tx.send(()).unwrap();
        actor.join().unwrap();
        closer.join().unwrap();
        assert!(closed.is_ok(), "shutdown waited for GPU/native callback");
        assert!(state.stop.load(Ordering::Acquire));
    }
    #[test]
    fn completion_releases_world_before_waking_main_to_mutate_it() {
        let compiled = compiled();
        let world = Arc::new(crate::scene_world(&compiled, &scene::Scene::default()));
        let weak = Arc::downgrade(&world);
        let state = shared(Arc::new(move || {
            assert!(
                weak.upgrade().is_none(),
                "frame world remained retained during main wake"
            )
        }));
        let mut request = frame(1);
        request.world = Some(world);
        state.frames.publish(request);
        let envelope = state.frames.take().unwrap();
        complete_frame(envelope, Some(Event::Retry { id: 1 }), &state);
        assert!(matches!(state.drain().as_slice(), [Event::Retry { id: 1 }]));
    }
    #[test]
    fn art_progress_is_coalesced_without_reordering_control_replies() {
        let state = shared(Arc::new(|| {}));
        let metadata = compiled();
        state.reply(Event::Retry { id: 1 });
        let mut old = Vec::new();
        for through in 1..100 {
            let compiled = Arc::new(metadata.without_pixels());
            old.push(Arc::downgrade(&compiled));
            state.reply(Event::ArtCommitted {
                base: 1,
                through,
                compiled,
                cursor_art: Arc::default(),
            });
        }
        state.reply(Event::Retry { id: 2 });
        assert!(old[..98].iter().all(|weak| weak.upgrade().is_none()));
        let events = state.drain();
        assert!(matches!(
            events.as_slice(),
            [
                Event::Retry { id: 1 },
                Event::ArtCommitted { through: 99, .. },
                Event::Retry { id: 2 }
            ]
        ));
        drop(events);
        assert!(old[98].upgrade().is_none());
        assert!(state.drain().is_empty());
    }
    #[test]
    fn compatible_optional_art_stream_cannot_starve_an_already_complete_ui() {
        let mut metadata = compiled().without_pixels();
        metadata.atlas.sprites.push(Default::default());
        metadata.sprite_names.push(String::new());
        let mut from = 1;
        for through in 2..32 {
            let before = metadata.without_pixels();
            metadata.atlas.sprites.push(Default::default());
            metadata.sprite_names.push(String::new());
            advance_prefix(&mut from, &before, &metadata, through);
            assert!(
                accepts_prefix(1, from, through, 1, 1),
                "unchanged old resources remain usable across optional art"
            );
        }
        let before = metadata.without_pixels();
        metadata.atlas.sprites[0].width += 1;
        advance_prefix(&mut from, &before, &metadata, 32);
        assert!(!accepts_prefix(1, from, 32, 1, 1));
        assert!(accepts_prefix(1, from, 32, 1, 32));
        assert!(
            !accepts_prefix(40, 40, 40, 1, 32),
            "a whole epoch always rejects old packet IDs"
        );
    }
    #[test]
    fn committed_cutoff_defers_new_art_until_earlier_fifo_control_can_be_delivered() {
        let state = shared(Arc::new(|| {}));
        let mut reader = Reader::default();
        let cutoff = state.published.load(Ordering::Acquire);
        // Main captured an empty FIFO/prefix. Worker publishes an older
        // control followed by newer metadata before main captures Latest.
        state.reply(Event::Retry { id: 10 });
        state.reply(Event::ArtCommitted {
            base: 1,
            through: 11,
            compiled: compiled(),
            cursor_art: Arc::default(),
        });
        reader.art = state.art_reply.take();
        reader.controls.extend(state.replies.drain());
        assert!(reader.release_before(cutoff).is_empty());
        assert!(reader.art.is_some());
        assert_eq!(reader.controls.len(), 1);
        assert!(matches!(
            state.drain_into(&mut reader).as_slice(),
            [
                Event::Retry { id: 10 },
                Event::ArtCommitted { through: 11, .. }
            ]
        ));
    }
    #[test]
    fn committed_cutoff_defers_new_fifo_control_until_earlier_art_can_be_delivered() {
        let state = shared(Arc::new(|| {}));
        let mut reader = Reader::default();
        let cutoff = state.published.load(Ordering::Acquire);
        // Main captured an empty Latest/prefix. Worker publishes metadata
        // then a control before main drains FIFO. Both must be staged.
        state.reply(Event::ArtCommitted {
            base: 1,
            through: 10,
            compiled: compiled(),
            cursor_art: Arc::default(),
        });
        state.reply(Event::Retry { id: 11 });
        reader.controls.extend(state.replies.drain());
        assert!(reader.release_before(cutoff).is_empty());
        assert_eq!(reader.controls.len(), 1);
        assert!(matches!(
            state.drain_into(&mut reader).as_slice(),
            [
                Event::ArtCommitted { through: 10, .. },
                Event::Retry { id: 11 }
            ]
        ));
    }
    #[test]
    fn envelope_is_not_visible_until_the_single_publisher_commits_it() {
        let state = shared(Arc::new(|| {}));
        let mut reader = Reader::default();
        let sequence = state.reply_sequence.fetch_add(1, Ordering::Relaxed);
        state.replies.send(Reply {
            sequence,
            event: Event::Retry { id: 9 },
        });
        assert!(state.drain_into(&mut reader).is_empty());
        state.published.store(sequence + 1, Ordering::Release);
        assert!(matches!(
            state.drain_into(&mut reader).as_slice(),
            [Event::Retry { id: 9 }]
        ));
    }
    #[test]
    fn worker_owns_gpu_types_and_requests_can_cross_threads() {
        fn send<T: Send>() {}
        send::<Gpu>();
        send::<Frame>();
        send::<ArtSnapshot>();
        send::<State>();
    }

    fn compiled() -> Arc<Compiled> {
        struct NoImages;
        impl render_desc::ImageLoader for NoImages {
            fn load(
                &self,
                _: &str,
                _: &toml::Table,
            ) -> Result<Vec<tile_render::RgbaImage>, String> {
                panic!("fixture has no images")
            }
        }
        Arc::new(
            render_desc::Description::parse("grid = { layout = \"orthogonal\", step = [1, 1] }")
                .unwrap()
                .compile(&NoImages, 16)
                .unwrap(),
        )
    }
    fn cpu_art_state() -> State {
        let compiled = compiled();
        State {
            diagnostics: None, gpu: None,
            assets: ArtInbox::new(Assets::new(compiled.clone())),
            compiled: compiled.without_pixels(), cursor_art: Arc::default(),
            whole_version: 1, assets_version: 1, compatible_from: 1,
            pending_art: None, preparing_art: None, target: 1,
            received_through: 1, art_error: None, profile: None, gpu_profile: None,
            notify_present: None,
        }
    }

    #[test]
    fn replayed_art_snapshots_admit_capacity_once_without_queueing_it_each_frame() {
        let mut state = cpu_art_state();
        let snapshot = ArtSnapshot { base: 1, whole: None, growths: Vec::new(), reserve: (1, 147), target: 1 };
        state.receive_art(&snapshot);
        assert_eq!(state.assets.take_reserve(1), Some((1, 147)));
        for _ in 0..1000 {
            state.receive_art(&snapshot);
            assert!(state.assets.take_reserve(1).is_none());
            assert!(state.assets.assets.0.added.is_empty());
        }
    }

    #[test]
    fn in_flight_retry_or_failure_keeps_its_capacity_without_duplicate_admission() {
        let mut state = cpu_art_state();
        let snapshot = ArtSnapshot { base: 1, whole: None, growths: Vec::new(), reserve: (1, 147), target: 1 };
        state.receive_art(&snapshot);
        let capacity = state.assets.take_reserve(1).unwrap();
        state.pending_art = Some(PendingArt::new(PendingArtPayload::Reserve { base: 1 }, capacity));
        for (started, failed) in [(true, false), (false, false), (false, true)] {
            let pending = state.pending_art.as_mut().unwrap();
            pending.started = started;
            pending.failed = failed;
            state.receive_art(&snapshot);
            assert!(state.assets.take_reserve(1).is_none());
            assert_eq!(state.pending_art.as_ref().unwrap().reserve, capacity);
        }
        // A recreated actor has no allocated atlas. It must receive the
        // same capacity again instead of inheriting another actor's record.
        let mut recovered = cpu_art_state();
        recovered.receive_art(&snapshot);
        assert_eq!(recovered.assets.take_reserve(1), Some(capacity));
    }

    #[test]
    fn replacement_capacity_is_independent_and_old_snapshots_cannot_restore_it() {
        let mut state = cpu_art_state();
        let old = ArtSnapshot { base: 1, whole: None, growths: Vec::new(), reserve: (1, 147), target: 1 };
        state.receive_art(&old);
        assert_eq!(state.assets.take_reserve(1), Some((1, 147)));
        let replacement = ArtSnapshot { base: 2, whole: Some(compiled()), growths: Vec::new(), reserve: (0, 3), target: 2 };
        state.receive_art(&replacement);
        assert_eq!(state.assets.take_reserve(2), Some((0, 3)));
        state.receive_art(&old);
        assert!(state.assets.take_reserve(2).is_none());
        assert_eq!(state.target, 2);
    }

    #[test]
    fn numeric_headless_gpu_work_runs_on_its_owner_and_rejects_stale_packets() {
        let caller = std::thread::current().id();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("viewer-gpu-test".into())
            .spawn(move || {
                let thread = std::thread::current().id();
                let headless =
                    tile_render::Headless::new().expect("GPU test requires an available adapter");
                #[cfg(target_os = "linux")]
                assert_eq!(
                    headless.adapter.get_info().vendor,
                    0x1002,
                    "Papa GPU checks must use AMD RADV"
                );
                let compiled = compiled();
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    width: 4,
                    height: 4,
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                    color_space: wgpu::SurfaceColorSpace::Auto,
                };
                let bitmap = BitmapTarget::new(&headless.device, &config).unwrap();
                let mut renderer = TileRenderer::new(&headless.device, config.format);
                renderer.set_atlas(&headless.device, &headless.queue, &compiled.atlas);
                let mut state = State {
                    diagnostics: None,
                    gpu: Some(Gpu {
                        surface: None,
                        device: headless.device,
                        queue: headless.queue,
                        config,
                        renderer,
                        bitmap: Some(bitmap),
                        software: true,
                        done: Arc::new(AtomicBool::new(true)),
                        world_version: None,
                        present_modes: vec![wgpu::PresentMode::Fifo],
                        config_revision: 0,
                        instance: None,
                        surface_lost: false,
                        recovery_requested: false,
                    }),
                    assets: ArtInbox::new(Assets::new(compiled.clone())),
                    compiled: compiled.without_pixels(),
                    cursor_art: Arc::default(),
                    whole_version: 1,
                    assets_version: 1,
                    compatible_from: 1,
                    pending_art: None,
                    preparing_art: None,
                    target: 1,
                    received_through: 1,
                    art_error: None,
                    profile: None,
                    gpu_profile: None,
                    notify_present: None,
                };
                // Window notifications: only for frames actually presented
                // (a retried one would hold back Wayland redraws).
                let notified = Arc::new(AtomicU64::new(0));
                state.notify_present = Some({
                    let notified = notified.clone();
                    Arc::new(move || { notified.fetch_add(1, Ordering::SeqCst); })
                });
                let shared = shared(Arc::new(|| {}));
                let mut stale = frame(1);
                stale.config_revision = 9;
                assert!(matches!(
                    state.draw(&stale, &shared).unwrap(),
                    Some(Event::Retry { id: 1 })
                ));
                stale.config_revision = 0;
                stale.base = 9;
                assert!(matches!(
                    state.draw(&stale, &shared).unwrap(),
                    Some(Event::Retry { id: 1 })
                ));
                stale.base = 1;
                stale.through = 9;
                assert!(matches!(
                    state.draw(&stale, &shared).unwrap(),
                    Some(Event::Retry { id: 1 })
                ));
                assert_eq!(notified.load(Ordering::SeqCst), 0, "a retried frame never notifies the window");
                let mut draw = frame(2);
                draw.world = Some(Arc::new(crate::scene_world(
                    &compiled,
                    &scene::Scene::default(),
                )));
                draw.replace_world = true;
                draw.cameras = vec![Camera::new((4, 4))];
                assert!(state.draw(&draw, &shared).unwrap().is_none());
                let until = Instant::now() + Duration::from_secs(15);
                loop {
                    state.poll(&shared).unwrap();
                    if let Some(pixels) = shared.drain().into_iter().find_map(|event| match event {
                        Event::Bitmap { pixels, .. } => Some(pixels),
                        _ => None,
                    }) {
                        // Reconfiguration must retire an outstanding diagnostic frame.
                        let mut pending = frame(3);
                        pending.world = draw.world.clone();
                        pending.replace_world = true;
                        pending.cameras = draw.cameras.clone();
                        assert!(state.draw(&pending, &shared).unwrap().is_none());
                        state
                            .configure(
                                &Config {
                                    revision: 1,
                                    width: 8,
                                    height: 8,
                                    present_mode: wgpu::PresentMode::Fifo,
                                    latency: 1,
                                },
                                &shared,
                            )
                            .unwrap();
                        assert!(
                            shared
                                .drain()
                                .iter()
                                .any(|e| matches!(e, Event::Retry { id: 3 }))
                        );
                        assert!(
                            state
                                .gpu
                                .as_ref()
                                .unwrap()
                                .bitmap
                                .as_ref()
                                .unwrap()
                                .pending
                                .is_none()
                        );
                        // A lost native surface requests a fresh main-thread binding,
                        // rather than configuring the invalid old surface forever.
                        {
                            let gpu = state.gpu.as_mut().unwrap();
                            gpu.bitmap = None;
                            gpu.instance =
                                Some(wgpu::Instance::new(tile_render::gpu::instance_descriptor()));
                            gpu.surface_lost = true;
                        }
                        state.poll(&shared).unwrap();
                        assert!(
                            shared
                                .drain()
                                .iter()
                                .any(|e| matches!(e, Event::SurfaceNeeded { .. }))
                        );
                        pending.config_revision = 1;
                        assert!(matches!(
                            state.draw(&pending, &shared).unwrap(),
                            Some(Event::Retry { id: 3 })
                        ));
                        // Neither retried nor bitmap-only frames present to a
                        // surface: none of them notified the window.
                        assert_eq!(notified.load(Ordering::SeqCst), 0);
                        shared
                            .surfaces
                            .send(Err("synthetic native binding failure".into()));
                        assert!(
                            state
                                .poll(&shared)
                                .unwrap_err()
                                .contains("synthetic native binding failure")
                        );
                        state.gpu.as_mut().unwrap().surface_lost = false;
                        // Transfer failure is a terminal observable result, never a
                        // permanently pending scene with only a log line.
                        state
                            .gpu
                            .as_mut()
                            .unwrap()
                            .renderer
                            .set_atlas_transfer_limit(std::num::NonZeroU64::new(1).unwrap());
                        state.receive_art(&ArtSnapshot {
                            base: 2,
                            whole: Some(compiled.clone()),
                            growths: vec![],
                            reserve: (0, 0),
                            target: 2,
                        });
                        state.upload_art();
                        assert!(
                            state
                                .art_error
                                .as_ref()
                                .is_some_and(|e| e.contains("atlas"))
                        );
                        tx.send((thread, pixels)).unwrap();
                        break;
                    }
                    assert!(Instant::now() < until, "GPU readback did not complete");
                    std::thread::yield_now();
                }
            })
            .unwrap();
        let (owner, pixels) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_ne!(caller, owner);
        assert_eq!(pixels, vec![0x182838; 16]);
    }
}

#[cfg(test)]
mod profiling_tests {
    use super::*;

    #[test]
    fn gpu_gap_diagnostics_preserve_missing_epochs_and_late_samples() {
        let frame = |id| tile_render::timing::FrameTimes {
            frame_token: Some(id + 100),
            stamp: tile_render::timing::FrameStamp { frame_id: id, start: id * 10, end: id * 10 + 5, period_ns: 1_000_000.0 },
            ..Default::default()
        };
        let mut d = NativeDiagnostics::new();
        let stats = |attempted_frames| Some(tile_render::timing::TimerStats { attempted_frames, ..Default::default() });
        d.observe_timer(stats(3));
        assert_eq!(d.gaps.observe(&frame(1)), None);
        assert_eq!(d.gaps.observe(&frame(3)), None, "missing sample is not idle time");
        assert_eq!(d.gaps.observe(&frame(2)), None, "late sample cannot move anchor backwards");
        assert_eq!(d.gaps.observe(&frame(4)), Some(5.0));
        let fields = d.gaps.fields();
        assert!(fields.contains("\"gpu_gap_discontinuous\":1"), "{fields}");
        assert!(fields.contains("\"gpu_gap_out_of_order\":1"), "{fields}");
        assert!(fields.contains("\"start_ticks\":\"40\""), "{fields}");
        assert!(fields.contains("\"frame_token\":104"), "{fields}");
        d.observe_timer(stats(0));
        assert_eq!(d.gaps.observe(&frame(1)), None, "new timer cannot bridge previous device epoch");
        d.observe_timer(None);
        d.observe_timer(stats(2));
        assert_eq!(d.gaps.observe(&frame(2)), None, "disabled timing breaks gap adjacency");
        assert_eq!(d.timer_epoch, 4);
        assert_eq!((d.gaps.observed, d.gaps.adjacent, d.gaps.discontinuous, d.gaps.out_of_order), (6, 1, 1, 1));
    }

    #[test]
    fn delayed_gpu_results_remain_separate_profile_rounds() {
        let frame = |token, span, pass| tile_render::timing::FrameTimes {
            frame_token: Some(token),
            stamp: tile_render::timing::FrameStamp { frame_id: token, start: token * 10, end: token * 10 + 5, period_ns: 1_000_000.0 },
            span,
            passes: vec![("world".into(), pass)],
            increments: vec![("world".into(), span)],
            ..Default::default()
        };
        let mut profile = crate::profile::Profile::new("test GPU", Duration::ZERO);
        let mut gaps = GpuFrameGaps::default();
        profile_gpu_frames(&mut profile, &[frame(10, 2.0, 3.0), frame(11, 4.0, 5.0)], &mut gaps);
        let report = profile
            .report()
            .expect("both delayed frames must be reported");
        assert!(report.contains("2 rounds"), "{report}");
        for label in [
            "gpu frame span",
            "gpu passes sum",
            "gpu world",
            "gpu increment world",
        ] {
            let line = report
                .lines()
                .find(|line| line.trim_start().starts_with(label))
                .expect(label);
            assert!(
                line.contains("n     2"),
                "each frame must contribute its own sample: {line}"
            );
        }
        let gap = report.lines().find(|line| line.trim_start().starts_with("gpu inter-frame gap")).unwrap();
        assert!(gap.contains("n     1") && gap.contains("p50 5.00"), "{gap}");
        assert_eq!((gaps.observed, gaps.adjacent, gaps.discontinuous, gaps.out_of_order), (2, 1, 0, 0));
        assert!(report.contains("gpu passes"));
        assert!(
            !report.contains("interval ms"),
            "readback arrival intervals are not GPU frame intervals"
        );
    }
}
