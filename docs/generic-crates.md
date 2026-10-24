# Generic crates

Some crates are game-agnostic: they render and arrange any tile game a
driver describes, and they know nothing of Civilization III. They never
depend on a game's crate (`civ3-*`, `civ3r-*`), on the injected-code crates
(`inproc`, `jgl*`), or on a crate that does. Their tests are game-agnostic
too. `crates/tile-viewer/tests/generic_crates.rs` checks this.

## Which crates

| Crate | Why it is generic |
| --- | --- |
| `tile-render`, `tile-render-shared`, `tile-render-shaders` | renders tiles, nothing else |
| `glass`, `glass-shared`, `glass-shaders` | the glass lens (rounded slab with a halo) over views and screen copies: host passes and CPU oracle, shared parameters, shader library |
| `marker`, `marker-shaders` | the selection marker (bouncing arrow and ground shadow): CPU oracle and art, shader maths |
| `viewer-shaders`, `viewer-spirv` | the viewer's one SPIR-V module: `viewer-shaders` links the three `no_std` shader libraries, `viewer-spirv`'s build script compiles it once per build; `tile-viewer` draws with it, `tile-render` takes the module from outside (its own GPU tests use the same module, as a dev-dependency) |
| `scene` | the driver-renderer protocol: snapshots, input, extension payloads |
| `render-desc` | TOML render descriptions compiled into atlases and layers |
| `menu` | TOML menus drawn as free sprites, with hit testing |
| `tile-viewer` | the window: arrangement, input, pointing aids, extension points |
| `tile-game` | concepts of tile strategy games that are no single game's (see below) |
| `viewer-settings` | the viewer's settings schema; `scene`, `menu` and `tile-viewer` depend on it, so it has to be generic |
| `viewer-transport` | the authenticated local socket between driver and viewer; it carries `scene` bytes |
| `render-errors` | error counting and logging; `tile-render` depends on it |
| `handoff` (was `civ3r-sync`) | lock-free hand-off primitives |
| `gpu-text` | font outlines for GPU text |
| `pointer-api`, `pointer-control`, `pointer-fallback`, `pointer-macos`, `pointer-wayland`, `pointer-windows`, `pointer-windows-ffi`, `pointer-x11`, `wl-pointer` | pointer capture, confinement and warps for the viewer's window |
| `application-icon(-macos)`, `function-keys(-macos)` | window icon; F1-F12 for whatever runs in front |

The Civilization III crates sit on top of them: `civ3-ui` on `menu` and
`tile-game` (the interface, the viewer extensions, the screen roles),
`civ3-bridge` and `civ3-sys` (the game side), `civ3-assets` (file formats),
and the hosts `civ3r-viewer` and `civ3r-native`, which plug the Civ III
parts into the generic viewer (`civ3_ui::viewer`).

## The audit (October 2026)

What the generic crates knew of Civ III before this pass, and where it went:

- **handoff**: only its name `civ3r-sync` and its description ("between
  the game's thread and the renderer's"). Renamed; `no_locks` and
  `docs/concurrency.md` follow.
- **scene**: the city screen's build queue was part of the protocol
  (`Interface::build_queue`, `Input::BuildQueue`); the viewer told screens
  apart by the driver's names ("City", "Menu"); `UnitOrder` and docs on
  cities, advisors, Civ III's dialogs and `civ3_assets`. See the protocol
  section below.
- **tile-viewer**: depended on `civ3r-sync`. Had the build queue drag in the
  city screen's parchment colours (`build_queue.rs`), seven checks of
  screen names (`"City"`, `"Menu"`) in `layout.rs` and `lib.rs`, unit orders
  documented with Civ6 and Civ III command names, `CIV3R_*` environment
  variables (launcher file, launcher scale, recording folder), the `civ3r`
  settings path, and tests named after cities, units and advisors
  (`city_screen_tests.rs`, `snap/city_work.rs`, `unit_motions`), one reading
  `assets/civ3/loading-cursor.toml`.
- **tile-render, tile-render-shared, tile-render-shaders**: glass and the
  selection marker (not tiles) lived in the tile shader crate and renderer;
  text tests read the font of a Civ III installation; comments and test
  names spoke of Civ III terrain, cities and `civ3_bridge`.
- **render-desc**: a Civ III palette comment, `civ` palette fixtures, a font
  test reading a Civ III installation.
- **tile-render-shared** also named Civ III's thirteen Clean Map categories
  (`CATEGORY_CITIES`, `CATEGORY_UNITS`, ...): now it only reserves the top
  four category bits (`CATEGORY_RESERVED`); the driver's categories are its
  own (`viewer_settings::clean_map`).
- **menu**: six tests parsed `assets/civ3/preferences.toml`; fixture names
  (`Art/Advisors`, "unit picture", "Explosion Palette").
- **viewer-settings**: the `civ3r` settings folder and `CIV3R_SETTINGS`, a
  Civilization IV comment, the game's Clean Map wording.
- **render-errors**: the file name `civ3-render-errors.log`, `civ3r` names
  for its log target and thread, leader-portrait tests.
- **gpu-text**: tests reading a Civ III installation's font.
- **pointer-windows**: depended on `inproc`, the injected-code crate, for
  four system calls.
- **viewer-transport, the other pointer crates, wl-pointer,
  application-icon, function-keys**: only the `civ3r-sync` dependency, or
  nothing ("the game" in their docs means whatever runs in front).

### What moved where

- `civ3r-sync` → `handoff`.
- The build queue → `tile-game::production_queue` (a site's production
  queue: model, wire codec, the drag as a `tile_viewer::extension::
  InterfaceExtension`, colours as a `QueueStyle`). `civ3-ui::production_queue`
  has the city screen's colours; `civ3_ui::viewer::extensions()` plugs the
  drag into every viewer host; the bridge sends and applies the queue
  through `tile-game`'s types.
- Screen names → `scene::ScreenRole` (`Other`, `Menu`, `Site`), set by the
  driver; `civ3_ui::screens::role` maps Civ III's names. The viewer never
  reads a name.
- Unit orders → world orders (`Input::WorldOrder`, `tile-viewer`'s
  `world_orders`), and the frame's order-button place → `order_buttons`.
- `CIV3R_*` and the settings path → `ViewerConfig::{startup_ready_file,
  launcher_scale, settings_path}` and `civ3_ui::viewer`, which reads the
  variables for the hosts.
- The render-error log's file name → the bridge.
- The four Windows pointer calls → `pointer-windows-ffi` (the unsafe shim).
- Tests on Civ III data → `civ3-ui/tests/preferences_panels.rs`; the viewer's
  loading cursor art → `assets/viewer/loading-cursor.toml` (it was never the
  game's art); font tests read `TEST_FONT` (the dev shell points it at the
  same Lucida Sans).
- Glass and the selection marker → `glass-shaders` and `marker-shaders`,
  linked with `tile-render-shaders` by `viewer-shaders` (see below).

## The protocol: a generic core and extensions

`scene` is the generic core. A game kind's own messages are extensions: a
`scene::Extension { kind, data }` in `Interface::extensions` from the
driver, an `Input::Extension { kind, data }` back. The crate that defines
an extension owns its `kind` (four letters of its name, `b"PQue"`) and its
codec. A renderer without the extension ignores it.

This beats generalising the build queue into a core message: a production
queue is a strategy-game concept, not a renderer one, so it belongs with the
other strategy-game concepts in `tile-game`, which can grow without wire
version bumps of the core for each concept. It also beats a type parameter
on `Scene`, which would have spread through every crate that handles
scenes. The extension's bytes cost one small decode per frame on the viewer
side. The wire version is 65.

Screen roles are core: arranging a site's screen or a menu is the viewer's
job, so the viewer needs a role, and the name stays an opaque label for
logs, settings and tests.

## tile-game

`tile-game` holds concepts that tile strategy games share and that no game
owns: so far a site's production queue, reordered by dragging. It sits on
`scene`, `tile-render` and `tile-viewer`, and plugs into the viewer through
`tile_viewer::extension`, so the viewer stays small and knows nothing of
it. A game's crate maps its own concepts onto these (Civ III's city build
queue onto a production queue) and gives the look (`QueueStyle`).

Concepts that stay inside `tile-viewer`, because they are part of how the
viewer maps pointer and camera, are game-agnostic there: pieces walking
along a path (`piece_motions`, `scene::Walk`), world orders (preview, then
confirm a destination), the order buttons' place, the anchored popups.

## Words

- **site**: a place of the world with a screen of its own that produces
  things (a city, a town, a base, a star system). "City" exists only in
  the Civ III crates, which map it onto a site.
- **piece**: what moves over the map (Civ III's units). "Unit" is a common
  strategy-game word too; persisted settings keys (`unit_buttons`,
  `UnitMouse`, `active_unit_arrow`, `unit_rings`) and the Clean Map
  categories keep it, to keep users' settings files and the menus' control
  names as they are.
- **report screen**: a named screen that is no site's and no menu (Civ III's
  advisors, its Civilopedia). Test names use "Report", "Overview", ...
- **world order**: a go-to order for the selected piece.

## What is still game-flavoured, and why

- `viewer-settings`: the persisted keys above, the Clean Map category names
  (rivers, cities, units, yields: a strategy map's layers; they are control
  names in the menu descriptions and settings files), and
  `Layout::SCREENS`, the screen names older settings files stored per-screen
  places under (kept to read those files as they were). Splitting the
  settings into a generic schema and a game section is the next step.
- The `bridge` and `native` origin names in `render-errors`' log lines (the
  process topology, not the game; the log text stays as it was).
- The selection marker's art generator in `render-desc` (`gilded_arrow`) is
  generic code with a warm palette chosen to suit a parchment interface.
