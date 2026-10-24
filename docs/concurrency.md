# Concurrency without locks

The bridge DLL runs inside the game's 32-bit process. Under Wine + FEX on the
Mac the game hung for a minute and more while its main thread waited on locks
our threads held. The rule since: **our code shares no lock between threads.**
Threads and tasks own their state and talk through channels. The game's thread
(every hook) only sends owned messages and polls replies at safe points. It
never awaits. One explicit exception delays a visible combat animation until
the viewer has accepted a camera frame that contains its complete visible
combat area. This parks on an atomic serial and `Wake`, with a two-second
failure watchdog. The publisher delivers receipts directly, without posting
them to the parked game thread. Rendering and the OS cursor continue. The
receipt confirms the queued camera frame and matching input mapping, not
physical scanout or a GPU completion fence.

`crates/civ3r-sync/tests/no_locks.rs` fails the build if a lock type or lock
crate shows up in `crates/` outside `crates/civ3r-sync/locks-allowlist.txt`.
The allowlist is empty.

## Building blocks

| What | Used for | Why it does not block |
|---|---|---|
| `thread_local!` `Cell`/`RefCell` | state only the game's thread touches (hooks, window procedures) | one owner, nothing shared |
| `civ3r_sync::Latest<T>` (arc-swap) | latest value wins: one side publishes, any thread reads (camera focus, go-to path, mirrored UI view, captured units, pointer override) | readers and writers never wait; a reader keeps the `Arc` it loaded |
| `civ3r_sync::Bounded<T>` (crossbeam, bounded) | the Alt mode recording's records, viewer thread to its writer thread (`tile_viewer::altrec`) | `push` never blocks: a full queue drops the record and counts it; the writer only polls, woken by a `Wake` |
| `civ3r_sync::Mailbox<T>` (crossbeam, unbounded) | commands for the game's thread, drained at its safe points (camera commands, modal actions, paint rectangles, notes) | `send` never blocks; the receiver only polls, so senders stay on the lock-free path |
| `civ3r_sync::Posted<T>` | payloads of private window messages: the publisher sends the payload and posts a token, the game's window procedure takes what was sent up to that token (world input, native sliders, preference fences) | token protocol on atomics; exactly once, in order, consecutive pointer moves merged (tests in `civ3r-sync`) |
| `civ3r_sync::Wake` | the publisher's bell (`presenter::WAKE`): rung by the game's thread when a present changes what the window shows, by the input thread and the preview thread; the publisher waits for it between rounds (at most its 33 ms fallback tick) | `ring` is an atomic swap and, once per wake-up, `Thread::unpark` (futex / `WakeByAddressSingle`); the publisher parks between rounds; the combat barrier can park the game thread before a visible fight |
| tokio `mpsc` / `oneshot` / `watch` | between our own threads and tasks (game thread to preview and replay workers, log writer, runtime tasks) | unbounded `send` is a lock-free push; our receivers may block, they are ours |
| `OnceLock` / `LazyLock`, atomics | immutable data set once, flags and counters | |

`watch` keeps its value under a lock internally. It is only used where neither
end is the game's thread; `Latest` replaces it everywhere else.

## The game's process

```
 game thread (foreign code; hooks, window procedures)
   | owns: UI mirror model, trace recording, camera state, slider
   |       transaction, go-to search, skipper, pens, unit selection
   |-- Latest --> UI view (snapshot + stats + presented)   --> publisher
   |-- Latest --> camera focus, go-to path, units capture,
   |              last map press, scenario selection,
   |              form surfaces, refusal notices            --> publisher
   |-- tokio mpsc --> preview jobs                          --> civ3r-preview
   |-- tokio mpsc --> replay operations (unbounded)         --> civ3r-replay
   |-- Mailbox --> text runs                                --> readers merge
   |-- tokio mpsc --> log lines                             --> civ3r-log
   ^
   |-- Posted<world input>, Posted<slider request>,
   |   Posted<preferences receipt> (+ PostMessage token)    <-- publisher
   |-- Mailbox<camera command>, Mailbox<modal action>       <-- publisher
   |-- Latest<pointer override>, atomics (focus, flags)     <-- publisher

 civ3r-publish (own thread, CPU heavy: a round whenever WAKE rings,
   |             at least every 33 ms)
 civ3r-input (native viewer: waits in civ3r_native's input call, hands
   |          input on through a Mailbox, rings WAKE)
 civ3r-opacity (answers which image-file regions are opaque for the UI
   |            mirror's compaction: tokio mpsc in, Latest out)
   |-- std/tokio channels --> civ3r-art, civ3r-fonts, civ3r-preview (own threads)
   |-- viewer-transport client --> viewer-send / viewer-input (own threads)
 civ3r-rt (tokio current-thread runtime): trace request server
 civ3r-log: the only writer of civ3-bridge.log
```

### Runtime choice

One tokio **current-thread** runtime on one thread, `civ3r-rt`
(`crates/civ3-bridge/src/runtime.rs`). It hosts the actors that mostly wait:
the trace request server today. The CPU-heavy actors (publisher, art, fonts,
preview, replay) keep a thread each. On a shared scheduler their
seconds-long work would starve the waiting actors. A multi-thread runtime
would add worker threads to the 32-bit process for no gain. They are still
actors: each owns its state and is reached only through channels. Features
`rt`, `time` and `sync`; no `rt-multi-thread`, no `macros`.

The game's thread never enters the runtime: it only sends.

### What each former lock became

| Where | Before | Threads | Now |
|---|---|---|---|
| inproc `hook.rs` handler (every hooked call) | `RwLock<Option<Handler>>`, read for the whole call | game + installer | `HandlerSlot` (atomic pointer; a replaced handler is leaked, never freed) |
| inproc `message.rs` window registry (every game window message) | `Mutex<HashMap>` | game + publisher | fixed table of atomics; entry filled before the procedure is swapped in |
| inproc `pointer.rs` override, kept focus | 2 × `Mutex` | publisher writes, game hooks read | `Latest<Override>`, `AtomicU32` |
| inproc `screen.rs` size, hooked count | 2 × `Mutex` | game | `OnceLock`; each hook module claims itself |
| inproc `paint.rs` pending rectangles | `Mutex<HashMap>` | any invalidator, game paints | per-window `Mailbox` + "paint posted" flag |
| inproc `logging.rs` file | `Mutex<File>` | every thread incl. game | channel to `civ3r-log`; panic hook flushes (waits at most 200 ms, panics only) |
| inproc `loader.rs` test, `hooks_x86.rs` test | `Mutex` | tests | `loader::InstallOnce`; cases run in turn |
| civ3-sys `native_camera.rs` state | `Mutex<State>` | game + publisher | game-thread `thread_local`; `Mailbox<Command>` in; `Latest<Published>` out; lifecycle atomics |
| bridge `world_input.rs` queue, pointer tile, preview, last press | 4 × `Mutex` | publisher posts, game handles | `Posted<Pending>`, `Latest`, `thread_local` + retire flag, `Latest` |
| bridge `native_sliders.rs` requests + transaction | `Mutex` | publisher + game | `Posted<Request>`, `thread_local` transaction |
| bridge `prefs/delivery.rs` receipts | `Mutex<HashMap>` | publisher + game | `Posted<Receipt>` |
| bridge `modal.rs` action queue, walk, notices | 3 × `Mutex` | publisher + game | `Mailbox`, `thread_local`, `Latest` |
| bridge `ui_capture.rs` installation, model, original drawing | 3 × `Mutex` | load callback, worker, game, publisher | atomic state machines (see below); model handed to the game's thread through a `Mailbox` |
| bridge `ui_trace.rs` model access (every UI draw), skipper, pens, audit rows, form surfaces, presented frame, seen sets | 9 × `Mutex` | game + publisher + trace server | game-thread model; `Latest<UiView>` published per present and on request; `thread_local`; `Latest` |
| bridge `ui.rs` | (model data) | | `sizes`, `sprites` are `Arc` copy-on-write; `UiModel::snapshot` |
| bridge `trace.rs` recording, recent text, fonts, text hook | 4 × `Mutex` | game + publisher + trace server | `thread_local` + on-request `Latest` copy; `Mailbox` of runs merged into `Latest`; `thread_local`; `InstallOnce` |
| bridge `targeting.rs` path | `Mutex` | game + publisher | `thread_local` + `Latest<GotoPath>` |
| bridge `unit_views.rs` capture, selection, membership | 3 × `Mutex` | game + publisher | `Latest`, `thread_local` × 2 |
| bridge `preview.rs` rules, selection, asker | 3 × `Mutex` | game hook + publisher + worker | `OnceLock`, `Latest`, `Latest` + tokio channel |
| bridge `replay_thread.rs` picture | `Mutex` + bounded channel (the game thread blocked when full) | game + worker + publisher | `Latest<Picture>`, unbounded tokio channel |
| bridge `districts/source.rs`, `live.rs` diplomacy, `game_window.rs` notes | 4 × `Mutex` | publisher, preview | `OnceLock`, `Latest`, `Mailbox` |

### Waits that remain, and why

- **Startup capture** (`ui_capture.rs`): when the game's load callback and the
  late-injection worker prepare capture at the same moment, the second one
  yields until the first publishes the outcome. Correctness needs it (the game
  must not run on with half-installed hooks). It happens at most once per
  process, before the first frame. Afterwards every check is one atomic load.
  Restoring original drawing waits likewise for an activation in progress.
- **jgl text takeover** (`jgl_hooks.rs`, `inproc::loader::InstallOnce`): two
  simultaneous loads of `jgl.dll` both return only once the slots are written.
- **Panic logging**: the panic hook waits up to 200 ms for the log writer.
- **Quitting** (`session_end.rs`, below): the game's thread, once it quits,
  waits at most 500 ms for the publisher's round to end and for the viewer
  to say it is gone, then ends the process. Polled atomics, no lock.
- Inside channel implementations: waking a *parked* receiver of ours takes a
  short internal lock (tokio's parker, crossbeam's waker). Game-thread
  receivers only poll, so nothing ever parks on the game's side.
- wgpu and its dependencies (in-process viewer) lock internally; that is
  their code, on the viewer's thread.

## The game quitting

`crates/civ3-bridge/src/session_end.rs`. Before, the game freed its world
while the publisher went on reading it and the viewer went on drawing it:
city labels went, units turned barbarian, then the map went, all over
several seconds, and only then did the window close. The publisher also
read the freed memory (safely, through `ReadProcessMemory`), and its rounds
took seconds there.

The game's quit, its order (PCGames build, `civ3_sys::quit`):

1. The main loop (`run_game`, 0x4F6270) leaves a running game when its
   "leave" flag is set (the confirmed Quit, `REALLYQUIT`; the end of a game;
   load; new game), at 0x4F6857, and ends that game. If its "restart" byte
   is 0 it then returns (0x4F68AD), as it does after the title menu's Quit.
2. `BIC::run` (0x4C9190) tears the program's objects down; `WinMain`
   returns; the C runtime's `exit` runs the `atexit` handlers and calls
   `ExitProcess`, which kills every other thread and runs the DLLs' detach.

The bridge hooks the call sites 0x4F6857 (with the restart byte 0) and
0x4F68AD, the game's `ExitProcess` import and its own DLL detach (for an
`ExitProcess` from elsewhere). The first that comes, on the game's thread
and before the game frees anything:

```
 game thread                      civ3r-publish                  viewer
 quit(): SESSION.begin()  ------> round() refuses / no publish
   WAKE.ring()                    end_session(): Update::Close ---> Display::closing:
   wait <= 500 ms for:                                              no later batch shown,
     no round reading,                                              window hidden,
     the viewer gone <----------- Input::Quit <------------------- F1-F12 given back,
   end_process (TerminateProcess)                                   Input::Quit
```

The game's thread stays in the hook while it waits, so the world stays
whole for any round still reading it. It then runs the two steps of the
game's teardown that persist something (`civ3_sys::quit`): the scroll
speed into `conquests.ini` and deleting the scenario loader's temporary
files. A quit's changes to the game folder and prefix are the same as
with the game's own exit; the rest of the teardown only frees memory and
handles. No thread of ours is joined, no GPU device is torn down, no DLL
detach runs. With the compatibility fallback
the game's own exit runs on after the wait, with nothing of ours reading
it. The same `Update::Close` closes the in-process viewer, the native
library's viewer (Linux, macOS) and a separate viewer process.

## Viewer side (native viewer process, in-process viewer, transport)

| Where | Before | Now |
|---|---|---|
| viewer-transport client input, log | 2 × `Mutex<VecDeque>` | `Mailbox` (input bound kept; log keeps the newest `MAX_LOG`, first failure only) |
| viewer-transport outgoing | `sync_channel(8)` | `Mailbox` + atomic count limit 8; writer thread parks, senders `unpark` it |
| scene `Waker` | `Arc<Mutex<Option<Box<dyn Fn>>>>` | `Latest` of the callback; input channel is crossbeam; `Display::receive` parks, `Driver::send` unparks |
| tile-viewer `Assets` slot (publisher thread and viewer thread) | `Arc<Mutex<Slot>>` | whole description in `Latest` with an epoch; growths and reservations in a `Mailbox` tagged with that epoch; the viewer thread owns an `ArtInbox` of what it has not applied |
| tile-viewer cursor latency | `Arc<Mutex<Vec<f64>>>` | `Mailbox` from the GPU completion callback |
| civ3r-viewer art queue, assets | `Mutex` + `Condvar`, `Mutex<Option<Assets>>` | crossbeam channel owned by the art worker; `OnceLock` |
| civ3r-native viewer, profile, driver log, art, pending log | 5 × `Mutex` | `OnceLock`, `thread_local`, `Mailbox`, a local of the art thread, `Latest` take/put-back |
| civ3r-native art queue | `Mutex` + `Condvar` | `Mailbox`; the art thread parks, pushes unpark it |
| civ3-assets font sheets, notes, sizes, directory cache | 4 × `Mutex` | copy-on-write `Latest` maps (a race may compute an entry twice) |

Where a thread of ours waits for work that the game's thread may send, it
parks (`std::thread::park`) and senders `unpark` it, which takes no lock;
waking through a channel's own blocking receive would.

Threads of the native viewer process: the main thread owns the winit/AppKit
window, input, layout, OS cursor, scene metadata and softbuffer bitmap output.
`viewer-gpu` owns the wgpu instance, adapter, device, queue, surface configuration
and acquisition, renderer, atlas preparation, uploads, encoding and submission.
`viewer-session` owns the socket's write side and the `Driver`;
`viewer-receive` reads the socket; `viewer-art` grows the art and publishes
it to `Assets`. `viewer-pointer` and, on macOS, `winit-pointer` handle the
pointer (below).

### Pointer threads

The main thread prepares frames (layout, scene adoption, interface
arrangement) between window events; in the game's process on macOS it also
shares AppKit's thread with Wine's own work. Frames of 40 to 60 ms happen.
Pointer handling that ran there stood still for that long.

`viewer-pointer` (`tile-viewer/src/pointer_thread.rs`) ticks at twice the
display's refresh rate (`winit` `refresh_rate_millihertz`; 120 Hz assumed
when unknown; at least 120, at most 1000 ticks a second), retimed when the
window moves to another display. It sleeps to absolute deadlines with
`std::thread::sleep` (high-resolution timers on Linux, macOS and Windows),
never spins, and checks a stop flag every tick; dropping it joins it within
one tick. Each tick it reads the pointer: from the backend's thread-safe
feed when there is one (`pointer_api::Backend::feed`), else the newest
`CursorMoved` the main thread reported (`Latest`). Its rate is logged every
10 s at debug level. Measured with the main thread spinning through 50 ms
frames: pointer updates on the main thread every 50 ms, on `viewer-pointer`
every 4.17 ms (at most 5.6 ms) at 240 Hz
(`pointer_thread::tests::pointer_intervals_under_render_load`).

`winit-pointer` (macOS only, `vendor/winit/.../macos/pointer_tap.rs`) is a
listen-only CoreGraphics event tap (`CGEventTapCreate`, session level) on a
`CFRunLoop` of its own: event-driven, it runs once per mouse event, as the
window server delivers it, independent of the main thread. While a pointer
region (split divider, zoom box, captured edge) holds the pointer, the
pointer is detached from the mouse and this thread applies each motion
delta to the region and warps the pointer (`CGWarpMouseCursorPosition`, a
CoreGraphics call any thread may make). So the visible pointer moves with
the mouse even while the main thread is busy. The main thread publishes the
region (`ArcSwap`, a new generation per region or main-thread warp) and the
view's place on the screen, and reads the position back for its
`CursorMoved` events; detaching and reattaching
(`CGAssociateMouseAndMouseCursorPosition`) stay on the main thread, where
regions are set and released. The tap also reports the free pointer in
view pixels, which `viewer-pointer` reads. The run loop wakes every 250 ms
to check its stop flag and to enable the tap again if the system disabled
it. A listen-only tap of mouse events needs no Accessibility permission; if
the system still refuses the tap, or `WINIT_POINTER_TAP=0`, the main thread
places the pointer as before. Both threads run the host's thread policy
first (`ViewerConfig::native_thread_init`: Wine's signals blocked).

Edge scrolling runs on `viewer-pointer`'s clock (`tile-viewer/src/edge_clock.rs`).
The layout publishes, through `Latest`, the view the pointer is in and how
that view meets the window's edges (`EdgeTarget`: its rectangle, the edges it
reaches, the zoom strip's row, the divider it is pushed against, the
interface's rectangles for the thin zone, the speed). It republishes each
frame and after each pointer event it handles; it clears the target when edge
scrolling is not active. Every tick, the thread finds the newest pointer's
edge and adds that tick's scrolling to the view's running total (`Totals`,
`Latest`, one writer). Each frame moves every view by how much its total grew
since the previous frame. The frame reads the total as of one tick before its
own time, interpolated between the last two ticks. Frames at any phase to the
ticks then move by exactly their own time and never back. What the thread
scrolled while scrolling was no longer active (the pointer left, a grab, a
popup at a click) is dropped. The amount taken is part of `Pan` and is
carried with the frame's other motion (`prepared_motion`). Without the
thread (tests, headless rendering) each frame scrolls by its own time.

Latency: a held pointer moves within the tap callback of its own event
(microseconds after the window server sees it); the main thread's
`CursorMoved` carries that position when it handles the event, which is when
it is free again. `viewer-pointer` sees the tap's position on its next tick
(at most one tick, 4.2 ms at 240 Hz). On Linux and Windows the system
confines the pointer itself (Wayland constraints, X11 barriers, `ClipCursor`)
and there is no feed: `viewer-pointer` reads the main thread's reports, so
it sees motion only once the main thread handled it. Button snapping (Alt
mode) still integrates device motion on the main thread.

The GPU actor creates its instance and requests a native surface binding through
an event. Only that binding runs on the main thread, because AppKit's window
handle must be obtained there. The resulting `Send` surface returns to the GPU
actor. The main thread never waits for this handshake or for a GPU operation.
The native window and CPU cursor metadata are available before GPU initialization.

Frames, art snapshots and configuration changes use `civ3r_sync::Latest`.
The actor takes each request, releasing the slot rather than retaining the last
world or full source atlas. App permits one frame in flight and keeps coalescing
new driver updates while it is busy. An immutable frame contains an `Arc` world,
markers, cameras, copies and art/configuration versions. A stale predecessor
requires a full world update; stale art or configuration requires a retry.
Only a matching successful presentation activates the retained frame's input
mapping. Surface failure preserves the previously presented mapping.

Main's `ArtInbox` is the only consumer of the publisher's additions. It sends
ordered snapshots to the GPU actor, whose private inbox deduplicates growths
within their whole-art generation. Atlas commits return immutable metadata and
render cursor flags; the normal OS cursor uses independently published CPU art
and the newest received cursor state. Its icon creation and event-loop wake do
not wait for atlas upload, surface acquisition or a frame completion.

`TILE_VIEWER_SOFTWARE_PRESENT` remains an explicit macOS diagnostic mode. The
actor renders and asynchronously maps its bounded readback buffer, then sends
completed pixels to main-thread softbuffer output. No device wait runs on the
event loop. Reconfiguring an outstanding bitmap frame returns a retry before
replacing its readback storage. Hardware Metal/Vulkan presentation remains the
default. Shutdown signals and wakes the GPU actor without joining it from the
main thread, so an outstanding native main-thread callback cannot deadlock exit.

### Game-thread ownership

State moved into `thread_local!` is touched only from hooks and window
procedures, which run on the game's thread. The UI mirror model is taken over
by the first thread that draws after preparation; drawing from any other
thread is logged once and not mirrored.

### Frame controls and GPU queue ordering

Each renderer has three small control slots. A slot contains only the
`FrameParams` camera/pass uniforms and `OutputParams` presentation/copy
uniforms, plus reusable CPU upload storage. World buffers, free-sprite data,
font outlines and atlas textures keep their existing ownership and allocation.
The GPU control buffers occupy 864,000 bytes in total with 32 views.

CPU preparation claims an unused slot, assembles its complete controls and
uploads each nonempty uniform buffer once. Encoding never exposes a partly
prepared snapshot to another frame. Up to three frames can be encoded before
submission; their bind groups reference distinct slots. When all three are
prepared, rendering refuses another frame without recording commands or
rewriting any prepared controls. The viewer retains/coalesces newer scene and
input state for a subsequent frame.

After submitting all commands encoded with a renderer, the caller must call
`TileRenderer::controls_submitted()`. The viewer's normal and cached frame
paths and the headless convenience renderer do this. This notification marks
a CPU submission boundary and does not wait for GPU completion. Wgpu queue
writes for a later submission execute after earlier submitted draws, so a
control slot can then recycle safely even while the GPU is still busy. Raw
API callers that omit the notification safely exhaust the three slots; they
must submit the prepared batch before notifying. Notification after an
unrelated submission does not release controls for commands still held in an
unsubmitted encoder.

The main GPU rendering path submits one render command buffer per presented
frame. Updates to world buffers, markers, matching texture patches and the
complete camera/UI controls precede that submission. A frame uses the scene,
art generation and layout selected at its frame boundary. Atlas preparation
can add bounded transfer submissions; adoption/retirement can add an empty
submission to flush pending writes before registering retirement callbacks.
Their staging-buffer reuse still requires completion callbacks to preserve
the documented finite upload-memory budget. Those callbacks are serviced by
nonblocking device polling. The triple control slots need no completion
callbacks, fences or readbacks.

The software adapter frame gate also uses nonblocking polling and returns to
the event loop when its previous frame is unfinished. An active cursor no
longer requests a blocking 100 ms GPU wait. Input and pending scene updates
continue to arrive while a paced retry awaits completion.

GPU timestamps are optional asynchronous readbacks through a bounded ring.
If no readback slot is free, that frame has no timing sample. The normal
Vulkan/Metal surface path does not read rendered pixels back to the CPU.
Headless tests explicitly wait for readback to verify pixels. The opt-in
macOS bitmap presenter for VM configurations without a Metal display device
also reads pixels back asynchronously on the GPU worker. Its mapping callback
only publishes completion; the main thread presents completed pixel buffers.
The legacy headless readback helper still permits an explicit blocking wait.

Vsync now requests maximum frame latency 2, allowing CPU preparation and GPU
rendering to overlap. Wgpu documents that latency 1 prevents that overlap.
Vulkan uses this as a request for three swapchain images and Metal requests
three drawables, subject to backend limits. Explicit mailbox/immediate
choices retain their existing mode and latency 3. Acquiring a drawable or
presenting can still wait in the platform backend; the frame overlay measures
those phases separately. The control slots do not change the selected
presentation mode.

### Control snapshots and mutable world uploads

The renderer has three small control slots for cameras, passes and composition.
They can be encoded before submission without duplicating the world or atlas.
After submitting every encoded draw, the caller immediately calls
`controls_submitted`. Queue ordering then places later control writes after
those draws, without waiting for GPU completion.

The slots do not snapshot mutable world, free-sprite, marker or atlas buffers.
Submit earlier draws before queue-writing updates such as `update_cells`,
`update_free`, `set_markers` and `patch_sprite`. A queue write made before
submission can affect all draws in that submission, including an earlier
encoded frame. The production GPU actor updates a packet, encodes it, submits
it and releases its control slot before accepting the next packet. Tests that
prepare several different worlds before submission use `set_world`, which
allocates separate world buffers for the encoded draws.
