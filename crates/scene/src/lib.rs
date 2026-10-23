//! What a game driver tells a renderer, and what the renderer tells back.
//!
//! The renderer owns the whole world: a [`Scene`] with the grid shape, for
//! every layer of the render description one value per cell (0 = nothing;
//! other values were resolved from set names via the description), the
//! world's sprites, and any number of views of it. The driver (the only
//! game-specific part) sends [`Update`]s; the renderer applies them, draws
//! every view and queues [`Input`] for the driver to apply to the game.
//!
//! [`connect`] links the two sides, each on its own thread.

pub mod wire;
pub mod loading_cursor;


use std::sync::mpsc;
use std::time::Duration;

/// The visible part of the world.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// World pixel at the top-left of the viewport.
    pub camera_x: i32,
    pub camera_y: i32,
    /// Screen pixels per world pixel (a renderer may arrange it otherwise).
    pub zoom: (u32, u32),
    /// Viewport size in screen pixels.
    pub viewport: (u32, u32),
    /// Draw only cells whose logical anchor lies in this world-pixel
    /// rectangle (x0, y0, x1, y1), if set (games that clip by tile range).
    pub cull: Option<(i32, i32, i32, i32)>,
    /// Where the view sits in the window (its top-left, window pixels).
    pub origin: (i32, i32),
    /// Shows only its overlay, not the world (an interface on top).
    pub interface_only: bool,
    /// An interface-only view showing menus (screens and popups over the
    /// game), rather than the frame of the game around the world (its
    /// buttons and panels): a renderer may scale the two differently.
    pub menu: bool,
    /// Shows the world this many times smaller (0 or 1: as it is), from the
    /// same art, as a game's zoomed-out view (see
    /// `tile_render::Camera::downscale`): the camera and the cull rectangle
    /// are then in the view's own pixels (the world's divided), as are the
    /// world pixels the renderer reports for it (`Input::Look`, ...) and
    /// [`Scene::focus`]; cells and world sprites stay at full size.
    pub downscale: u32,
    /// Category bits hidden in this view. Category zero is always visible.
    pub hidden_categories: u32,
    /// Which data slots the view draws its layers from ([`Scene::bindings`]):
    /// 0, or a binding the scene does not have, draws every layer from its
    /// own cells ([`Scene::layers`]); views of one binding share its slots
    /// (see docs/per-view-layers.md).
    pub binding: u32,
}

/// Cells of a layer kept apart from its own ([`Scene::layers`]), which the
/// views of a binding listing it draw instead: e.g. one view's fog of war
/// while the others keep the player's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerSlot {
    /// Chosen by the driver.
    pub id: u32,
    /// The layer whose cells these are (in the description's layer order).
    pub layer: u32,
    /// As many as the layer's, with phases and offsets where it has them.
    pub cells: LayerCells,
}

/// What a binding's views draw from slots: per layer listed, the slot (its
/// id) holding the cells they draw it from; the rest from the layers' own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Binding {
    /// Chosen by the driver, not 0 (every view's binding unless it names
    /// another).
    pub id: u32,
    /// (layer, slot id), by layer.
    pub slots: Vec<(u32, u32)>,
}

/// Shape of the cell grid (layout and spacing come from the description).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldShape {
    pub cols: u32,
    pub rows: u32,
    pub wrap_x: bool,
    pub wrap_y: bool,
}

/// Cell values of one layer, row-major, `cols × rows`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerCells {
    pub values: Vec<u32>,
    /// Per-cell animation phase, if the layer has one.
    pub phases: Option<Vec<u8>>,
    /// Per-cell pixel offset (x, y), if the layer has one: the cell's sprite
    /// is drawn that far from its place (world pixels, see
    /// `tile_render::TileWorld::set_offset`).
    pub offsets: Option<Vec<(i16, i16)>>,
}

/// Explicit destination-dependent native composition for free sprites.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SpriteBlend {
    #[default]
    Alpha,
    /// RGB8 source and raw alpha byte composed over RGB555 with a / 256.
    PaletteAlpha256,
}

/// A sprite not tied to a cell (units, effects, UI).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FreeSprite {
    /// Sprite set (animation) id, resolved from a set name by the
    /// description.
    pub set: u32,
    /// Anchor position: world pixels, or viewport pixels if `screen`.
    pub x: i32,
    pub y: i32,
    pub screen: bool,
    /// Drawn after this render pass (beyond the last: on top).
    pub after_pass: u32,
    pub phase: u32,
    pub ticks_per_frame: u32,
    /// 0: alpha blending; otherwise darkens like a scale layer.
    pub scale_den: u32,
    /// Native palette composition overrides the legacy scale/tint blend.
    pub blend: SpriteBlend,
    /// The colour of ClearType-like text (`scale_den` = the renderer's
    /// `SCALE_SUBPIXEL`), 0xRRGGBB.
    /// With ordinary alpha blending: the high byte is inverse opacity.
    pub tint: u32,
    /// 0 any zoom; 1 the 1:1 twin of sprites made for zooms, left out in
    /// views at a zoom the scene has art for ([`Scene::zooms`]); 2 made for
    /// `zoom`: drawn in views at that zoom, at the sprite's 1:1 position
    /// scaled plus `zoom_offset` (pixels at that zoom).
    pub zoom_art: u8,
    pub zoom: (u16, u16),
    pub zoom_offset: (i32, i32),
    /// Drawn only inside this viewport rectangle (left, top, right,
    /// bottom; exclusive).
    pub clip: Option<[i32; 4]>,
    /// A world sprite drawn in every view but this one (a view id): what
    /// the driver's own view does not show there, other views do.
    pub not_in_view: Option<u32>,
    /// A glyph of text drawn from its font's outline instead of the sprite
    /// (`set` does not matter), in `tint`: zoomed art for every zoom
    /// (`zoom_art` 2, `zoom` (0, 0)), its pen at the sprite's position on
    /// the baseline.
    pub glyph: Option<Glyph>,
    /// An atlas-independent solid rectangle, anchored at x,y.
    pub solid: Option<SolidRect>,
    /// The named palette its indexed sprites are drawn with (an index into
    /// the render description's palettes; 0: none).
    pub palette: u32,
    /// Visibility category bit for per-camera clean-map filtering.
    pub category: u32,
}

/// A procedural rectangle with unpremultiplied RGBA. Like UI sprite art,
/// its dimensions stay fixed under world downscale; camera zoom still applies.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SolidRect {
    pub width: u32,
    pub height: u32,
    pub rgba: [u8; 4],
}

/// A glyph drawn from a font's outline (see [`FreeSprite::glyph`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The font: an index into the render description's fonts.
    pub font: u32,
    pub ch: char,
    /// The em at 1:1 in 64ths of a pixel.
    pub em64: u32,
    /// The pen advance at 1:1 in pixels: the glyph is centred in it, scaled.
    pub advance: i32,
    /// Emboldened, slanted (a regular face made bold or italic).
    pub bold: bool,
    pub italic: bool,
    /// Drawn onto this colour key (opaque where it differs from it).
    pub key: Option<[u8; 3]>,
    /// An underline instead of the glyph: its top below the baseline and
    /// its height, at 1:1 (`advance` wide).
    pub bar: Option<(i32, i32)>,
}

/// One view of the world: where it sits in the window, what it looks at,
/// and what is drawn over it in screen space (an interface).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Chosen by the driver; views are drawn in id order (higher on top).
    pub id: u32,
    pub view: View,
    /// Screen-space sprites (window pixels) drawn in this view only.
    pub overlay: Vec<FreeSprite>,
}

/// A range of world sprites with a lifetime on the viewer's monotonic clock.
/// A nonzero serial identifies one telling; resending it must not restart
/// its timer. Drivers allocate serials in increasing order across the session.
/// `sprites` is a half-open range in [`Scene::sprites`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteLifetime {
    pub serial: u64,
    pub sprites: (u32, u32),
    pub duration_ms: u32,
}

/// What the renderer knows: the whole world and the views of it. Drivers
/// send [`Update`]s; the renderer applies them ([`Scene::apply`]) and draws
/// every view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Increases with every applied batch of updates.
    pub seq: u64,
    /// Animation clock.
    pub tick: u32,
    pub shape: WorldShape,
    /// In the description's layer order.
    pub layers: Vec<LayerCells>,
    /// Cells of layers that some views draw instead of the layers' own
    /// (by id; see [`View::binding`]), and which views draw which:
    /// [`Binding`]s by id. A slot costs a layer's cells; views drawing the
    /// same cells share one, and one no view differs by is not sent.
    pub slots: Vec<LayerSlot>,
    pub bindings: Vec<Binding>,
    /// World-space sprites (units, effects), drawn in every view, in
    /// drawing order (within the pass each follows).
    pub sprites: Vec<FreeSprite>,
    pub sprite_lifetimes: Vec<SpriteLifetime>,
    /// By id.
    pub views: Vec<ViewState>,
    /// The zooms the scene has zoomed art for (see `FreeSprite::zoom_art`);
    /// (0, 0): every zoom (text drawn from outlines, `FreeSprite::glyph`).
    pub zooms: Vec<(u32, u32)>,
    /// The generation of the art (render assets) the scene's sprite ids
    /// refer to; 0: whatever the renderer has.
    pub assets: u64,
    /// The world pixel the driver considers its point of interest (e.g. the
    /// unit a player is moving), if any (in the pixels of its views, see
    /// [`View::downscale`]).
    pub focus: Option<(i32, i32)>,
    /// Semantic main-camera request independent of the native rendering camera.
    /// A new serial repeats a request even when its destination is unchanged.
    /// None retains the driver-camera behavior of older drivers.
    pub camera_focus: Option<CameraFocus>,
    /// Complete visible combat bounds requiring a presented camera receipt.
    pub camera_arrival: Option<CameraArrival>,
    /// Map displacement for one wheel notch in current game-pixel units.
    /// Positive magnitudes; Shift selects X, otherwise Y. Zero preserves
    /// ordinary driver wheel routing for drivers without this metadata.
    pub map_wheel_step: (i32, i32),
    /// The driver's minimap where the renderer may point at it (none: no
    /// minimap, or one the renderer must leave to the driver).
    pub minimap: Option<Minimap>,
    /// New pixels for atlas sprites whose content changes at run time.
    pub patches: Vec<Patch>,
    /// The renderer's own settings as the driver last sent them (the driver
    /// keeps them, e.g. edited on a preferences screen; `None`: none sent,
    /// the renderer goes by its own copy).
    pub settings: Option<viewer_settings::Settings>,
    /// Temporary render multiplier awaiting viewer-clock confirmation.
    pub settings_preview: Option<SettingsPreview>,
    /// What the driver's interface shows that a renderer may arrange.
    pub interface: Interface,
    /// What the user can point at and act on in the driver's interface
    /// (buttons, list entries, lists), for pointer aids such as snapping
    /// the pointer to buttons.
    pub interactables: Vec<Interactable>,
    /// The mouse pointer the driver's interface shows now (e.g. a game's
    /// targeting cursor while the player picks a target).
    pub cursor: Cursor,
    /// The size of the driver's screen (its interface's window pixels;
    /// (0, 0): unknown, the renderer's window). A renderer whose window is
    /// larger may show it scaled up (see [`Input::Resized`]).
    pub screen_size: (u32, u32),
    /// How some of them look in their other states (hover, pressed,
    /// selected), for a renderer to show those itself the moment the
    /// pointer asks for them, rather than wait for the driver's interface to
    /// draw them.
    pub looks: Vec<Looks>,
}

/// An explicit request to focus the primary renderer view.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraFocus {
    pub serial: u64,
    /// Center in the driver's current zoom-world pixels.
    pub center: (i32, i32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CameraArrival { pub serial: u64, pub bounds: [i32; 4] }

/// The driver's minimap: a picture of the whole world, linear along both
/// axes, shown on its screen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Minimap {
    /// The driver's screen pixel where the picture's pixel (0, 0) is.
    pub at: (i32, i32),
    /// The picture's size, in its pixels.
    pub size: (u32, u32),
    /// Where it shows on the driver's screen (x, y, width, height).
    pub shown: [i32; 4],
    /// The world pixels (at the current game zoom) the whole picture spans.
    pub world: (i32, i32),
}

impl Minimap {
    /// The world pixel the driver's screen pixel `p` points at, if it is
    /// on the minimap where it shows.
    pub fn world_at(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        let [x, y, w, h] = self.shown;
        let inside = p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h;
        if !inside || self.size.0 == 0 || self.size.1 == 0 {
            return None;
        }
        // (The middle of the picture's pixel.)
        let axis = |v: i32, at: i32, size: u32, world: i32| ((v - at) as f64 + 0.5) * world as f64 / size as f64;
        let (wx, wy) = (axis(p.0, self.at.0, self.size.0, self.world.0), axis(p.1, self.at.1, self.size.1, self.world.1));
        (wx >= 0.0 && wy >= 0.0 && wx < self.world.0 as f64 && wy < self.world.1 as f64).then_some((wx as i32, wy as i32))
    }
}

/// A mouse pointer.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    /// Whatever the renderer shows by default.
    #[default]
    Default,
    /// The first frame of this sprite set (the raw id, as
    /// [`FreeSprite::set`]), its anchor at the hot spot. A renderer shows it
    /// as the system's pointer where it can (it needs the set's pixels: see
    /// the renderer for which sets it keeps them of), and draws it itself
    /// where it shows the pointer on its own.
    Sprite(u32),
    /// One of the system's standard pointers.
    System(SystemCursor),
}

/// The standard pointers of a system.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SystemCursor {
    #[default]
    Arrow,
    /// Busy: the program takes no input.
    Wait,
    /// Busy in the background: input is still taken.
    Progress,
    Crosshair,
    Text,
    Hand,
    NotAllowed,
    Move,
    /// Resizing: up and down, left and right, along the diagonal from the
    /// top left, along the one from the top right.
    ResizeNs,
    ResizeEw,
    ResizeNwse,
    ResizeNesw,
}

impl SystemCursor {
    /// All of them, in the order of their wire tags.
    pub const ALL: [SystemCursor; 12] = [
        SystemCursor::Arrow,
        SystemCursor::Wait,
        SystemCursor::Progress,
        SystemCursor::Crosshair,
        SystemCursor::Text,
        SystemCursor::Hand,
        SystemCursor::NotAllowed,
        SystemCursor::Move,
        SystemCursor::ResizeNs,
        SystemCursor::ResizeEw,
        SystemCursor::ResizeNwse,
        SystemCursor::ResizeNesw,
    ];
}

/// Parts of the driver's interface a renderer may arrange (all in window
/// pixels; views by id). A renderer that does not arrange anything draws
/// every view where the driver put it, as always.
/// The visible modal overlays in paint order, excluding the named screen.
/// Civ III's dialog machinery permits at most four open forms.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ModalStack(pub [Option<u32>; 4]);

impl ModalStack {
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ { self.0.iter().flatten().copied() }
    pub fn contains(&self, id: u32) -> bool { self.iter().any(|view| view == id) }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// The screen the interface shows, named by the driver (e.g. "F1",
    /// "City", "Menu"); none: its usual screen over the world. A renderer
    /// may place a named screen in one of its own views.
    pub screen: Option<String>,
    /// Keep the complete menu inside the visible canvas or assigned menu
    /// region, even when user menu scaling is disabled. Set for the title
    /// and Preferences using the driver's screen state, not rendered text.
    pub menu_fit: bool,
    /// An interface-only view holding controls that belong with whichever
    /// of the renderer's views the pointer is in (e.g. a game's menu buttons
    /// in the top-left corner, with the tooltip they show; the view's
    /// `origin` and `viewport` are their rectangle). The other interface
    /// views do not have them: a renderer draws this view where it places
    /// it, where the driver put it if nowhere else (and maps pointer input
    /// there back to where the driver put it).
    pub roaming: Option<u32>,
    /// An interface-only view holding what shows in the roaming controls'
    /// place while they cannot be used (the mouse captured: a hint how to
    /// let it go); its `origin` and `viewport` are its rectangle. A renderer
    /// draws it where it would draw `roaming`, instead of it, when so.
    pub capture_hint: Option<u32>,
    /// An interface-only view holding the open popup (its `origin` and
    /// `viewport` are the popup's rectangle; the other interface views do
    /// not have it). A renderer may draw it elsewhere (and map pointer input
    /// there back to where the driver put it).
    pub popup: Option<u32>,
    /// Independently placed modal views, bottom to top. `popup` is the top.
    pub modal_stack: ModalStack,
    /// Per entry of `modal_stack`: the view is part of the one under it,
    /// not placed on its own (e.g. the list a combo box in a dialog
    /// opened, which the driver shows as a form of its own just below the
    /// box, reaching past the dialog). A renderer draws it right above that
    /// view, moved and scaled exactly as that view (`Interface::popup_root`),
    /// and maps pointer input on it back the same way.
    pub modal_attached: [bool; 4],
    /// An interface-only view holding the named screen (`screen`) where it
    /// lies over the window, smaller than it, and what is drawn over it
    /// (its `origin` and `viewport` are its rectangle; the other interface
    /// views do not have it). A renderer may move it out from under its own
    /// views over the window (and map pointer input there back).
    pub screen_view: Option<u32>,
    /// The named screen's main control (x, y, w, h, in the driver's window
    /// pixels; e.g. a city screen's production box): where a renderer that
    /// lets go of a captured mouse when the screen opens puts the pointer.
    pub focus_control: Option<[i32; 4]>,
    /// A rectangle (x, y, w, h) of controls the pointer has to reach from
    /// every view of the renderer's (e.g. the active unit's orders), while
    /// they are shown.
    pub reachable: Option<[i32; 4]>,
    /// The tooltip the driver shows (x, y, w, h), if any: drawn over
    /// everything for a moment while the pointer rests on a control. A
    /// renderer that arranges the interface draws it with the control it
    /// belongs to, but arranges nothing around it (it is no panel of its
    /// own and takes no room).
    pub tooltip: Option<[i32; 4]>,
    /// The driver asks for its screen and its world side by side (e.g. a
    /// menu with a preview of the world it picks).
    pub beside: Option<Beside>,
    /// The open popup belongs at a place of the world a click picked (a
    /// context menu, a tile's information box): a renderer places it there
    /// in the view the click was made in, not with the screens.
    pub popup_anchor: Option<PopupAnchor>,
    /// The open popup is one the driver opened at a click on its named
    /// screen (`screen`; e.g. the menu of a unit in a city screen's
    /// garrison): it belongs where the driver put it on that screen, placed
    /// and scaled with the screen, not over it as a modal of the screen's.
    pub popup_on_screen: bool,
    /// Popups the driver opened at a world click and closed again at once,
    /// leaving their picture to the renderer (`DetachedPopup`): the tile
    /// information box. Each stays where its click was, in the view the
    /// click was made in, while every view goes on playing, until the
    /// renderer closes it (`Input::CloseDetached`).
    pub detached: Vec<DetachedPopup>,
    /// A command of the driver's waits for the player to pick a place of
    /// the world (e.g. a unit's go-to destination, a bombard target): the
    /// next click that matters is one on the world (a renderer may give
    /// the world the pointer for it).
    pub targeting: bool,
    /// The command waiting for its tile is a move (a unit's or a stack's
    /// go-to, whose target shows the turns it takes): a renderer may show
    /// the cursor tile by tile. Other commands (bombard, rebase, airlift,
    /// ...) keep the cursor at the pointer.
    pub snap_to_tiles: bool,
}

impl Interface {
    pub fn is_popup_view(&self, id: u32) -> bool { self.popup == Some(id) || self.modal_stack.contains(id) }
    pub fn popup_views(&self) -> impl Iterator<Item = u32> + '_ {
        self.modal_stack.iter().chain(self.popup.filter(|id| !self.modal_stack.contains(*id)))
    }
    /// The popup view placed for `id`: `id` itself, or for a view attached
    /// to the one under it (`modal_attached`), that one's (repeatedly).
    pub fn popup_root(&self, id: u32) -> u32 {
        let Some(mut k) = self.modal_stack.0.iter().position(|v| *v == Some(id)) else { return id };
        while k > 0 && self.modal_attached[k] && self.modal_stack.0[k - 1].is_some() {
            k -= 1;
        }
        self.modal_stack.0[k].unwrap_or(id)
    }
    /// The popup a renderer places: `popup`'s root (`popup_root`), e.g. the
    /// dialog whose combo box list is open on top of it.
    pub fn placed_popup(&self) -> Option<u32> {
        self.popup.map(|id| self.popup_root(id))
    }
}


/// Where a click-opened popup goes (`Interface::popup_anchor`): its top-left
/// corner `offset` (the popup's own pixels, scaled with it) from world
/// pixel `world` (as `Scene::focus`: the driver's view pixels).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupAnchor {
    pub world: (i32, i32),
    pub offset: (i32, i32),
    /// The popup shows a piece of the world at the game's own size (the
    /// tile information box's picture of the tile): a renderer draws it at
    /// the scale the world has in the view the click was made in, so that
    /// its picture matches the tiles there as in the game at 1:1 (not at
    /// the menus' scale).
    pub world_scaled: bool,
}

/// A popup's picture the driver keeps sending after it closed it in its
/// own interface (`Interface::detached`): an interface-only view (`view`,
/// its `origin` and `viewport` the popup's rectangle), placed as
/// `Interface::popup_anchor` places one, in the view the click at world
/// pixel `click` was made in. Nothing in it takes input: the renderer
/// closes it (`Input::CloseDetached`) where the driver's popup would have
/// closed.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DetachedPopup {
    pub view: u32,
    /// Names this popup for its lifetime (increasing, never reused).
    pub serial: u64,
    pub anchor: PopupAnchor,
    /// The world pixel of the press that opened it (as `Input::WorldButton`
    /// sent it).
    pub click: (i32, i32),
}

/// The driver's screen and its world side by side (`Interface::beside`):
/// the part of the screen that matters (`screen`, x, y, w, h in the
/// driver's window pixels, e.g. a dialog) goes to one edge of the window,
/// its interface moved with it (pointer input mapped back), and the
/// world (all of it, or the part `world` names) fitted into the rest (shown as small as it takes,
/// see `View::downscale`). The world is not the driver's view then: pointer
/// input on it does not reach the driver.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Beside {
    pub screen: [i32; 4],
    /// The window edge the screen goes to.
    pub edge: Edge,
    /// The part of the world to fit into the rest (x, y, w, h in world
    /// pixels as the driver's views show it at full size, e.g. what a
    /// preview has revealed; it may reach past the edge of a world that
    /// wraps): all of the world if none.
    pub world: Option<[i32; 4]>,
}

/// An edge of the window.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// Something in the driver's interface the user can point at and act on,
/// in the driver's window pixels (those of `Input` pointer events and of
/// its interface's overlay).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Interactable {
    /// x, y, width, height.
    pub rect: [i32; 4],
    /// Preferred pointer tip within `rect`, in the same driver pixels.
    /// None places the pointer at the control centre. The hit rectangle
    /// stays intact when an entry has blank space after its icon or text.
    pub anchor: Option<[i32; 2]>,
    /// What a person sees as the whole control, when that is more than
    /// `rect` (x, y, width, height, the same driver pixels): a check box
    /// and its label, for one. Button snapping is drawn to it from
    /// anywhere in it, and still points at and clicks the control where
    /// `rect` and `anchor` say. None: `rect` alone.
    pub region: Option<[i32; 4]>,
    pub kind: InteractableKind,
    /// Chosen by the driver, the same while the element is there (e.g. the
    /// address of a game's control object): a renderer may keep pointing at
    /// the same element across updates.
    pub id: u64,
    /// In button snapping only, a left click on it reaches the driver as a
    /// right click at the same point: for elements whose only action is
    /// on the right button (the F1 advisor's production cells). Ordinary
    /// mouse input is never changed.
    pub alt_right_click: bool,
}

/// What an [`Interactable`] is.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum InteractableKind {
    /// Pressed with a click (also tabs, check boxes, a menu's entries).
    #[default]
    Button,
    /// An entry of a list (selected with a click).
    ListItem,
    /// A list whose entries scroll: `scroll` is the rectangle (x, y, width,
    /// height) they scroll in, `step` how many pixels they move for one
    /// wheel notch (`Input::Wheel` with a delta of 120; 0: not known).
    List { scroll: [i32; 4], step: i32 },
    /// A slider's knob-center travel rectangle and current position in
    /// millionths of its range. A tall track is vertical, otherwise horizontal.
    Slider { track: [i32; 4], value: i32 },
}

/// The state an interactable is drawn in: where the pointer is and whether
/// it is selected.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LookState {
    pub pointer: PointerState,
    /// Selected (a list's entry, a tab) or checked (a check box).
    pub selected: bool,
}

/// What the pointer does to an interactable.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerState {
    /// Elsewhere.
    #[default]
    Normal,
    /// Over it, no button down.
    Hover,
    /// The left button went down on it and the pointer is still over it.
    Pressed,
}

/// What a click (left button down and up on it) does to an interactable's
/// selection.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Select {
    /// Nothing (a button).
    #[default]
    None,
    /// Flips it (a check box).
    Toggle,
    /// Selects it, and unselects the others of its group (a list's
    /// entries, tabs).
    Pick { group: u64 },
}

/// How the interactable [`Interactable::id`] `id` looks in the states a
/// renderer may draw it in itself: when the pointer moves onto it, presses
/// it, leaves it, or a click selects it, the renderer draws that state's
/// sprites in place of what the driver's interface draws for it, until the
/// driver shows the same (then the sprites drawn are the driver's own
/// again, and look the same). A state the driver shows that is none of
/// these (e.g. disabled) comes without looks: the driver's drawing stays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Looks {
    pub id: u64,
    pub select: Select,
    /// The state the driver's interface shows it in now.
    pub shown: LookState,
    /// Per view drawing it (e.g. a copy of it in another view).
    pub views: Vec<ViewLooks>,
}

/// An interactable's looks in one view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewLooks {
    pub view: u32,
    /// The sprites of the view's overlay drawing it now (start, end): what
    /// another state's sprites replace.
    pub sprites: (u32, u32),
    /// Its sprites in each state there is art for (overlay sprites of that
    /// view: screen space).
    pub states: Vec<(LookState, Vec<FreeSprite>)>,
}

impl ViewLooks {
    /// Its sprites in `state`, if there is art for it.
    pub fn state(&self, state: LookState) -> Option<&[FreeSprite]> {
        self.states.iter().find(|(s, _)| *s == state).map(|(_, v)| &v[..])
    }
}

/// The pixels of one atlas sprite (row-major RGBA, its exact size).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    /// Sprite name (`Compiled::sprite_names`): ids change with every
    /// compile, and patches outlive compiles.
    pub sprite: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Whether the driver's host answers (e.g. a game's thread): sent now and
/// then (a few times a second) by a driver that knows, also while nothing
/// else changes, so that a renderer can tell it is busy or stuck (see
/// [`Display::host`]). Not part of the [`Scene`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HostStatus {
    /// For how long the host has not answered when this was sent, in
    /// milliseconds (0: it answers; waiting for input counts as answering).
    pub stale_ms: u32,
    /// Counts the statuses sent.
    pub serial: u32,
}

/// One change of a [`Scene`].
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// A new world: shape and every layer (replaces all cells).
    World { shape: WorldShape, layers: Vec<LayerCells> },
    /// Some cells changed: (layer, cell index, value).
    Cells(Vec<(u32, u32, u32)>),
    /// Some cells' phases changed: (layer, cell index, phase).
    Phases(Vec<(u32, u32, u8)>),
    /// Some cells' offsets changed: (layer, cell index, offset).
    Offsets(Vec<(u32, u32, (i16, i16))>),
    /// Slot `id` added or replaced (layer, cells), or removed (`None`).
    Slot { id: u32, slot: Option<(u32, LayerCells)> },
    /// Some cells of slots changed: (slot id, cell index, value).
    SlotCells(Vec<(u32, u32, u32)>),
    /// See [`Scene::bindings`] (replaced).
    Bindings(Vec<Binding>),
    /// The world-space sprites (replaced).
    Sprites(Vec<FreeSprite>),
    /// Replaces the lifetimes of world sprite ranges.
    SpriteLifetimes(Vec<SpriteLifetime>),
    /// View `id` added or changed (its overlay kept), or removed (`None`).
    View { id: u32, view: Option<View> },
    /// The screen-space sprites over view `id` (replaced).
    Overlay { view: u32, sprites: Vec<FreeSprite> },
    /// Sprite pixels to write into the atlas.
    Patches(Vec<Patch>),
    Tick(u32),
    /// The point of interest moved (see [`Scene::focus`]).
    Focus(Option<(i32, i32)>),
    /// See [`Scene::camera_focus`].
    CameraFocus(Option<CameraFocus>),
    CameraArrival(Option<CameraArrival>),
    /// See [`Scene::map_wheel_step`].
    MapWheelStep((i32, i32)),
    /// See [`Scene::minimap`].
    Minimap(Option<Minimap>),
    /// The scene now refers to art generation `n` (see [`Scene::assets`]).
    Assets(u64),
    /// See [`Scene::zooms`].
    Zooms(Vec<(u32, u32)>),
    /// The renderer's settings changed (see [`Scene::settings`]): it takes them
    /// in place of its own.
    Settings(viewer_settings::Settings),
    SettingsPreview(Option<SettingsPreview>),
    /// See [`Scene::interface`].
    Interface(Interface),
    /// See [`Scene::interactables`] (replaced).
    Interactables(Vec<Interactable>),
    /// See [`Scene::cursor`].
    Cursor(Cursor),
    /// See [`Scene::screen_size`].
    ScreenSize((u32, u32)),
    /// See [`Scene::looks`] (replaced).
    Looks(Vec<Looks>),
    /// The host's [`HostStatus`]; kept by the [`Display`], not the scene
    /// (a batch of these alone is no new scene).
    Host(HostStatus),
}

impl Scene {
    pub fn empty(shape: WorldShape, layer_count: usize) -> Self {
        let n = (shape.cols * shape.rows) as usize;
        Scene { shape, layers: vec![LayerCells { values: vec![0; n], phases: None, offsets: None }; layer_count], ..Default::default() }
    }

    /// An empty scene whose layers carry per-cell phases and offsets as
    /// `layers` says (one `(phases, offsets)` per layer).
    pub fn empty_with(shape: WorldShape, layers: impl IntoIterator<Item = (bool, bool)>) -> Self {
        let n = (shape.cols * shape.rows) as usize;
        let layers = layers
            .into_iter()
            .map(|(phases, offsets)| LayerCells { values: vec![0; n], phases: phases.then(|| vec![0; n]), offsets: offsets.then(|| vec![(0, 0); n]) })
            .collect();
        Scene { shape, layers, ..Default::default() }
    }

    /// Sets a cell's phase (on a layer with phases; others ignore it).
    pub fn set_phase(&mut self, layer: usize, col: u32, row: u32, phase: u8) {
        let i = (row * self.shape.cols + col) as usize;
        if let Some(p) = self.layers[layer].phases.as_mut().and_then(|p| p.get_mut(i)) {
            *p = phase;
        }
    }

    /// Sets a cell's offset (on a layer with offsets; others ignore it).
    pub fn set_offset(&mut self, layer: usize, col: u32, row: u32, offset: (i16, i16)) {
        let i = (row * self.shape.cols + col) as usize;
        if let Some(o) = self.layers[layer].offsets.as_mut().and_then(|o| o.get_mut(i)) {
            *o = offset;
        }
    }

    /// Bytes the cells take (values, phases, offsets), for diagnostics.
    pub fn cell_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(|l| l.values.len() * 4 + l.phases.as_ref().map_or(0, Vec::len) + l.offsets.as_ref().map_or(0, |o| o.len() * 4))
            .sum()
    }

    pub fn set(&mut self, layer: usize, col: u32, row: u32, value: u32) {
        let i = (row * self.shape.cols + col) as usize;
        self.layers[layer].values[i] = value;
    }

    /// View `id`, if there is one.
    pub fn view(&self, id: u32) -> Option<&ViewState> {
        self.views.iter().find(|v| v.id == id)
    }

    /// View `id`, added (at the origin, nothing over it) if missing.
    pub fn view_mut(&mut self, id: u32) -> &mut ViewState {
        let i = match self.views.binary_search_by_key(&id, |v| v.id) {
            Ok(i) => i,
            Err(i) => {
                self.views.insert(i, ViewState { id, ..Default::default() });
                i
            }
        };
        &mut self.views[i]
    }

    /// Applies one update.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::World { shape, layers } => {
                self.shape = shape;
                self.layers = layers;
            }
            Update::Cells(changes) => {
                for (layer, cell, value) in changes {
                    if let Some(v) = self.layers.get_mut(layer as usize).and_then(|l| l.values.get_mut(cell as usize)) {
                        *v = value;
                    }
                }
            }
            Update::Phases(changes) => {
                for (layer, cell, phase) in changes {
                    if let Some(p) = self.layers.get_mut(layer as usize).and_then(|l| l.phases.as_mut()).and_then(|p| p.get_mut(cell as usize)) {
                        *p = phase;
                    }
                }
            }
            Update::Offsets(changes) => {
                for (layer, cell, offset) in changes {
                    if let Some(o) = self.layers.get_mut(layer as usize).and_then(|l| l.offsets.as_mut()).and_then(|o| o.get_mut(cell as usize)) {
                        *o = offset;
                    }
                }
            }
            Update::Slot { id, slot } => {
                let at = self.slots.binary_search_by_key(&id, |s| s.id);
                match (at, slot) {
                    (Ok(i), Some((layer, cells))) => self.slots[i] = LayerSlot { id, layer, cells },
                    (Err(i), Some((layer, cells))) => self.slots.insert(i, LayerSlot { id, layer, cells }),
                    (Ok(i), None) => drop(self.slots.remove(i)),
                    (Err(_), None) => {}
                }
            }
            Update::SlotCells(changes) => {
                for (slot, cell, value) in changes {
                    let Ok(i) = self.slots.binary_search_by_key(&slot, |s| s.id) else { continue };
                    if let Some(v) = self.slots[i].cells.values.get_mut(cell as usize) {
                        *v = value;
                    }
                }
            }
            Update::Bindings(b) => self.bindings = b,
            Update::Sprites(sprites) => self.sprites = sprites,
            Update::SpriteLifetimes(lifetimes) => self.sprite_lifetimes = lifetimes,
            Update::View { id, view: Some(view) } => self.view_mut(id).view = view,
            Update::View { id, view: None } => self.views.retain(|v| v.id != id),
            Update::Overlay { view, sprites } => self.view_mut(view).overlay = sprites,
            // Newer pixels replace older ones not taken yet (a renderer that
            // does not draw for a while keeps the latest only).
            Update::Patches(patches) => {
                for p in patches {
                    self.patches.retain(|q| q.sprite != p.sprite);
                    self.patches.push(p);
                }
            }
            Update::Tick(tick) => self.tick = tick,
            Update::Focus(focus) => self.focus = focus,
            Update::CameraFocus(focus) => self.camera_focus = focus,
            Update::CameraArrival(arrival) => self.camera_arrival = arrival,
            Update::MapWheelStep(step) => self.map_wheel_step = step,
            Update::Minimap(m) => self.minimap = m,
            Update::Assets(n) => self.assets = n,
            Update::Zooms(z) => self.zooms = z,
            Update::Settings(s) => self.settings = Some(s),
            Update::SettingsPreview(p) => self.settings_preview = p,
            Update::Interface(i) => self.interface = i,
            Update::Interactables(i) => self.interactables = i,
            Update::Cursor(c) => self.cursor = c,
            Update::ScreenSize(s) => self.screen_size = s,
            Update::Looks(l) => self.looks = l,
            // (Not the scene's: see `Display::host`.)
            Update::Host(_) => {}
        }
    }

    /// The updates that turn `old` into `self` (as small as cheaply found:
    /// changed cells rather than whole layers when the shape is the same).
    pub fn updates_since(&self, old: &Scene) -> Vec<Update> {
        let mut out = Vec::new();
        let same_layout = self.shape == old.shape
            && self.layers.len() == old.layers.len()
            && self.layers.iter().zip(&old.layers).all(|(a, b)| {
                let len = |v: &Option<Vec<_>>| v.as_ref().map(Vec::len);
                a.values.len() == b.values.len() && len(&a.phases) == len(&b.phases) && a.offsets.as_ref().map(Vec::len) == b.offsets.as_ref().map(Vec::len)
            });
        if !same_layout {
            out.push(Update::World { shape: self.shape, layers: self.layers.clone() });
        } else {
            let (mut cells, mut phases, mut offsets) = (Vec::new(), Vec::new(), Vec::new());
            for (l, (new, old)) in self.layers.iter().zip(&old.layers).enumerate() {
                for (i, (a, b)) in new.values.iter().zip(&old.values).enumerate() {
                    if a != b {
                        cells.push((l as u32, i as u32, *a));
                    }
                }
                if let (Some(new), Some(old)) = (&new.phases, &old.phases) {
                    phases.extend(new.iter().zip(old).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, _))| (l as u32, i as u32, *a)));
                }
                if let (Some(new), Some(old)) = (&new.offsets, &old.offsets) {
                    offsets.extend(new.iter().zip(old).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, _))| (l as u32, i as u32, *a)));
                }
            }
            if !cells.is_empty() {
                out.push(Update::Cells(cells));
            }
            if !phases.is_empty() {
                out.push(Update::Phases(phases));
            }
            if !offsets.is_empty() {
                out.push(Update::Offsets(offsets));
            }
        }
        // Slots: their changed values where only those changed, else whole.
        let mut slot_cells = Vec::new();
        for s in &old.slots {
            if self.slots.binary_search_by_key(&s.id, |n| n.id).is_err() {
                out.push(Update::Slot { id: s.id, slot: None });
            }
        }
        for s in &self.slots {
            let before = old.slots.binary_search_by_key(&s.id, |o| o.id).ok().map(|i| &old.slots[i]);
            match before {
                Some(b) if b == s => {}
                Some(b) if b.layer == s.layer && b.cells.values.len() == s.cells.values.len() && b.cells.phases == s.cells.phases && b.cells.offsets == s.cells.offsets => {
                    slot_cells.extend(s.cells.values.iter().zip(&b.cells.values).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, _))| (s.id, i as u32, *a)));
                }
                _ => out.push(Update::Slot { id: s.id, slot: Some((s.layer, s.cells.clone())) }),
            }
        }
        if !slot_cells.is_empty() {
            out.push(Update::SlotCells(slot_cells));
        }
        if self.bindings != old.bindings {
            out.push(Update::Bindings(self.bindings.clone()));
        }
        if self.sprites != old.sprites {
            out.push(Update::Sprites(self.sprites.clone()));
        }
        if self.sprite_lifetimes != old.sprite_lifetimes {
            out.push(Update::SpriteLifetimes(self.sprite_lifetimes.clone()));
        }
        for v in &old.views {
            if self.view(v.id).is_none() {
                out.push(Update::View { id: v.id, view: None });
            }
        }
        for v in &self.views {
            let before = old.view(v.id);
            if before.map(|b| b.view) != Some(v.view) {
                out.push(Update::View { id: v.id, view: Some(v.view) });
            }
            let overlay_before = before.map_or(&[][..], |b| &b.overlay[..]);
            if overlay_before != &v.overlay[..] {
                out.push(Update::Overlay { view: v.id, sprites: v.overlay.clone() });
            }
        }
        if !self.patches.is_empty() {
            out.push(Update::Patches(self.patches.clone()));
        }
        if self.tick != old.tick {
            out.push(Update::Tick(self.tick));
        }
        if self.camera_focus != old.camera_focus {
            out.push(Update::CameraFocus(self.camera_focus));
        }
        if self.camera_arrival != old.camera_arrival {
            out.push(Update::CameraArrival(self.camera_arrival));
        }
        if self.map_wheel_step != old.map_wheel_step {
            out.push(Update::MapWheelStep(self.map_wheel_step));
        }
        if self.minimap != old.minimap {
            out.push(Update::Minimap(self.minimap));
        }
        if self.focus != old.focus {
            out.push(Update::Focus(self.focus));
        }
        if self.assets != old.assets {
            out.push(Update::Assets(self.assets));
        }
        if self.zooms != old.zooms {
            out.push(Update::Zooms(self.zooms.clone()));
        }
        if let Some(s) = self.settings.as_ref().filter(|s| old.settings.as_ref() != Some(s)) {
            out.push(Update::Settings(s.clone()));
        }
        if self.settings_preview != old.settings_preview {
            out.push(Update::SettingsPreview(self.settings_preview));
        }
        if self.interface != old.interface {
            out.push(Update::Interface(self.interface.clone()));
        }
        if self.interactables != old.interactables {
            out.push(Update::Interactables(self.interactables.clone()));
        }
        if self.cursor != old.cursor {
            out.push(Update::Cursor(self.cursor));
        }
        if self.screen_size != old.screen_size {
            out.push(Update::ScreenSize(self.screen_size));
        }
        if self.looks != old.looks {
            out.push(Update::Looks(self.looks.clone()));
        }
        out
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// A unit order gesture keeps the native go-to preview open until a separate
/// click confirms its destination. A driver must reject a confirmation whose
/// tile, selected unit, or world differs from the last preview.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnitOrderAction {
    Preview,
    Confirm,
    Cancel,
}

/// An Alt slider edit is a transaction. Begin must preserve the current
/// value, Preview updates it, Commit accepts it, and Cancel restores the
/// exact value from Begin. A driver ignores unknown or inactive controls.
///
/// Step is a keyboard step (an arrow key on the selected slider): its value
/// is a signed count of the control's own steps (the native arrow step, or
/// the menu slider's step), clamped by the driver. Inside a transaction it
/// belongs to the edit (Cancel undoes it); outside one it is a whole edit.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SliderAction {
    Begin,
    Preview,
    Commit,
    Cancel,
    Step,
}

/// Stable identity and rollback value for a temporary render multiplier.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SettingsPreview {
    pub session: u64,
    pub id: u64,
    pub pixel_scale: u32,
    pub previous_pixel_scale: u32,
}

/// User input captured by the renderer, in viewport pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// The matching combat area is fully visible in an accepted presented mapping.
    CameraArrived { serial: u64 },
    /// No eligible camera can show the complete combat area, or the device failed.
    CameraUnavailable { serial: u64 },
    /// Viewer-local confirmation, scoped to one Preferences session/change.
    SettingsPreviewDecision { session: u64, id: u64, keep: bool },
    /// Edit a driver's slider without pressing or moving its pointer. The
    /// value is a fraction of its range in millionths, clamped by the driver.
    Slider { id: u64, action: SliderAction, value: i32 },
    MouseMove { x: i32, y: i32 },
    MouseButton { button: Button, down: bool, x: i32, y: i32 },
    Wheel { delta: i32, x: i32, y: i32 },
    /// A scroll's own motion along both axes, as the window reported it
    /// (viewer-local; the renderer pans its map with it and never passes it
    /// on). `dx`, `dy` in thousandths of a line, or of a window pixel if
    /// `pixels` (a trackpad); positive moves the content right and down.
    /// The `Wheel` notches it makes for the driver's interface follow it.
    Scroll { dx: i32, dy: i32, pixels: bool, x: i32, y: i32 },
    /// A key, as a Windows virtual-key code (the de-facto lingua franca of
    /// keyboard input), with the character it produced, if any.
    Key { vk: u32, down: bool, text: Option<char> },
    /// The renderer's drawing area now has this size in the driver's pixels
    /// (sent at start and on every change): its window's, or the driver's
    /// screen ([`Scene::screen_size`]) where the renderer shows that scaled up
    /// in a larger window (pointer positions are then in its pixels too).
    Resized { width: u32, height: u32 },
    /// The renderer's window in the screen's own (physical) pixels, and the
    /// desktop's scale there in thousandths (2000 at 200 %), sent at start
    /// and on every change: for showing what the renderer's settings make
    /// of them (e.g. the size it renders at).
    Window { width: u32, height: u32, scale_milli: u32 },
    /// The renderer's window was closed.
    Quit,
    /// The world rectangles (x, y, w, h; w 0: unused) the renderer shows in
    /// views of its own besides the driver's main view (a split pane,
    /// insets): a driver may mark them (e.g. on a minimap). They are in
    /// the world pixels of views shown `downscale` times smaller
    /// ([`View::downscale`], 0 or 1: full size), as of the renderer's frame:
    /// a driver whose zoom changed since takes them to its own (the same
    /// size in view pixels, the centre scaled; [`rescale_view_rect`]).
    Views { views: [(i32, i32, i32, i32); 4], downscale: u32 },
    /// The world rectangle (x, y, w, h) the renderer shows of the driver's
    /// main view where that is not all of it, as the driver has it (a split
    /// or a zoom strip crops it, a zoom shows less or more, it follows
    /// something else); w 0: all of it. A driver may mark that rather than
    /// its own view (e.g. on a minimap). In the world pixels of views
    /// shown `downscale` times smaller, as [`Input::Views`].
    MainView { x: i32, y: i32, w: i32, h: i32, downscale: u32 },
    /// The renderer shows the world at these zooms (`(num, den)` reduced;
    /// `(0, 0)`: unused): a driver may make art for them (e.g. text at that
    /// size, `FreeSprite::zoom_art` 2) and list them in `Scene::zooms`.
    Zooms([(u8, u8); 8]),
    /// The user wants to see world pixel (x, y) in the driver's main view
    /// (e.g. clicked it in another view): a driver may move its camera there.
    Look { x: i32, y: i32 },
    /// The pointer moved over a world pixel the driver's original window
    /// does not show. A driver can update hover and targeting directly
    /// without moving its camera. Coordinates use the main world scale.
    WorldMove { x: i32, y: i32 },
    /// Preview, confirm, or cancel a unit go-to order at a world pixel.
    /// Releasing the initiating button never confirms the order. Confirmation
    /// is valid only for the tile of the most recent preview.
    UnitOrder { action: UnitOrderAction, x: i32, y: i32 },
    /// A button went down or up on world pixel (x, y) in a view of the
    /// renderer's own whose pointer input does not reach the driver's
    /// window (it shows what the driver's main view does not, or what the
    /// driver's interface covers there): a driver may act on the world at
    /// that pixel.
    WorldButton { button: Button, down: bool, x: i32, y: i32 },
    /// A key went down or up while the pointer was over world pixel (x, y)
    /// in such a view, sent instead of `Key`: a driver may act on the world
    /// there, or take it as the `Key` it would have been.
    WorldKey { vk: u32, down: bool, text: Option<char>, x: i32, y: i32 },
    /// The pointer rests at the edge of the renderer's window over a view
    /// that follows the driver's main view (unmoved, each window pixel the
    /// driver's own, or zoomed about its middle): it is at the driver's
    /// screen edge as much as at the window's, and a driver may scroll the
    /// way it does when its own pointer is there (that view follows). Per axis: 0 if the pointer is not within
    /// [`EDGE_ZONE`] pixels of that axis's edges, else how far from the edge
    /// it is plus one, negative for the left (top) edge, positive for the
    /// right (bottom) one; `(0, 0)` once it is at no edge (or left the window
    /// or such a view). Sent when it changes. A renderer that divides its
    /// window may also send it while the pointer pushes against a divider
    /// from such a view (its edge there, at the outermost pixel).
    Edge { x: i32, y: i32 },
    /// Close the popup the driver opened at a world click (`Interface::
    /// popup_anchor`: a context menu, a tile's information box) the way
    /// the driver's own user dismisses it, if it is still open; nothing
    /// else (the click outside it that asked for this acts on nothing).
    DismissPopup,
    /// The renderer is done with detached popup `serial`
    /// (`Interface::detached`): the driver stops sending it.
    CloseDetached { serial: u64 },
}

/// How near the window's edge the pointer counts as at it ([`Input::Edge`]).
pub const EDGE_ZONE: i32 = 16;

/// Connects a driver (the game side) and a renderer, each on its own
/// thread: updates flow to the renderer, input flows back, both through
/// multiple-producer single-consumer queues ([`std::sync::mpsc`]; input
/// through a `crossbeam_channel`, so that a [`Driver`] may be shared).
pub fn connect() -> (Driver, Display) {
    let (updates_tx, updates_rx) = mpsc::channel();
    let (input_tx, input_rx) = crossbeam_channel::unbounded();
    let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wake = Waker::default();
    (
        Driver { updates: updates_tx, input: input_rx, in_flight: in_flight.clone(), wake: wake.clone() },
        Display { updates: updates_rx, input: input_tx, scene: Scene::default(), in_flight, new_settings: None, wake, changes: Changes { world: true, ..Default::default() }, changes_from_seq: 0, host: None },
    )
}

/// A sender of update batches (any number of them may feed one renderer).
pub type Updater = mpsc::Sender<Vec<Update>>;
/// A sender of input (any number of views or windows may report to one
/// driver).
pub type InputSender = crossbeam_channel::Sender<Input>;

/// What the driver's end calls after each batch it sends (see
/// [`Display::set_wake`]).
#[derive(Clone, Default)]
struct Waker {
    call: std::sync::Arc<civ3r_sync::Latest<Box<dyn Fn() + Send + Sync>>>,
    /// The thread waiting in [`Display::receive`], unparked by each batch.
    /// The renderer parks rather than waiting in the channel, so a send
    /// never touches the channel's waiter list (a lock).
    parked: std::sync::Arc<civ3r_sync::Latest<std::thread::Thread>>,
}

impl Waker {
    fn wake(&self) {
        if let Some(t) = self.parked.load() {
            t.unpark();
        }
        if let Some(f) = self.call.load() {
            f();
        }
    }
}

/// The driver's end: sends updates, takes input.
pub struct Driver {
    updates: Updater,
    input: crossbeam_channel::Receiver<Input>,
    /// Batches sent but not yet taken by the renderer.
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    wake: Waker,
}

impl Driver {
    /// Sends a batch of updates, applied by the renderer at once; false
    /// once the renderer is gone.
    pub fn send(&self, batch: Vec<Update>) -> bool {
        if batch.is_empty() {
            return true;
        }
        self.in_flight.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let sent = self.updates.send(batch).is_ok();
        self.wake.wake();
        sent
    }

    /// Whether the renderer has not taken everything sent yet: a driver may
    /// hold back its next batch (folding it into a later one) rather than
    /// queue up work for a slow renderer.
    pub fn busy(&self) -> bool {
        self.in_flight.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    /// Another sender to the same renderer.
    pub fn updater(&self) -> Updater {
        self.updates.clone()
    }

    /// All input queued so far.
    pub fn take_input(&self) -> Vec<Input> {
        self.input.try_iter().collect()
    }

    /// All input queued, waiting at most `timeout` for some when there is
    /// none (on a thread of the driver's own that hands it on).
    pub fn wait_input(&self, timeout: std::time::Duration) -> Vec<Input> {
        match self.input.recv_timeout(timeout) {
            Ok(first) => std::iter::once(first).chain(self.input.try_iter()).collect(),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Vec::new(),
            // (No renderer any more: no input ever, waited for all the same.)
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(timeout);
                Vec::new()
            }
        }
    }
}

/// The renderer's end: owns the [`Scene`] the updates build.
pub struct Display {
    updates: mpsc::Receiver<Vec<Update>>,
    input: InputSender,
    scene: Scene,
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Settings received and not taken yet (see [`Display::take_settings`]).
    new_settings: Option<viewer_settings::Settings>,
    wake: Waker,
    /// What changed since [`Display::take_changes`] last took it.
    changes: Changes,
    /// The source sequence the accumulated cell/sprite/view deltas start
    /// from. A rejected preparation can restore an older drawing snapshot.
    changes_from_seq: u64,
    /// The host's status last received, and when (see [`Display::host`]).
    host: Option<(HostStatus, std::time::Instant)>,
}

/// What of a [`Scene`] changed (see [`Display::take_changes`]), for a
/// renderer that keeps what it made of it and updates only that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// A new world: its shape and every cell (`cells` is then empty).
    pub world: bool,
    /// The cells whose value, phase or offset changed: (layer, cell
    /// index), maybe more than once.
    pub cells: Vec<(u32, u32)>,
    /// The world's sprites.
    pub sprites: bool,
    /// A view (added, changed, removed) or the sprites over one.
    pub views: bool,
    /// Which slots there are, or which bindings draw them (a slot added,
    /// replaced or removed; the bindings changed): `slot_cells` is then
    /// empty.
    pub slots: bool,
    /// The cells of slots whose value changed: (slot id, cell index).
    pub slot_cells: Vec<(u32, u32)>,
}

/// More changed cells than this are a new world to a renderer (see
/// [`Changes`]).
const MOST_CHANGED_CELLS: usize = 1 << 16;

impl Changes {
    /// Adds what `update` changes.
    fn note(&mut self, update: &Update) {
        let mut cells = |c: &mut dyn Iterator<Item = (u32, u32)>| {
            if !self.world {
                self.cells.extend(c);
                if self.cells.len() > MOST_CHANGED_CELLS {
                    self.world = true;
                    self.cells = Vec::new();
                }
            }
        };
        match update {
            Update::World { .. } => {
                self.world = true;
                self.cells = Vec::new();
            }
            Update::Cells(c) => cells(&mut c.iter().map(|&(l, i, _)| (l, i))),
            Update::Phases(c) => cells(&mut c.iter().map(|&(l, i, _)| (l, i))),
            Update::Offsets(c) => cells(&mut c.iter().map(|&(l, i, _)| (l, i))),
            Update::Sprites(_) | Update::SpriteLifetimes(_) => self.sprites = true,
            Update::View { .. } | Update::Overlay { .. } => self.views = true,
            Update::Slot { .. } | Update::Bindings(_) => {
                self.slots = true;
                self.slot_cells = Vec::new();
            }
            Update::SlotCells(c) if !self.slots => {
                self.slot_cells.extend(c.iter().map(|&(s, i, _)| (s, i)));
                if self.slot_cells.len() > MOST_CHANGED_CELLS {
                    self.slots = true;
                    self.slot_cells = Vec::new();
                }
            }
            _ => {}
        }
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }
}

impl Display {
    /// Has `wake` called whenever the driver has sent a batch (through
    /// [`Driver::send`]; not through other [`Updater`]s), on the driver's
    /// thread: a renderer that waits for its window's events rather than
    /// for updates ([`Display::receive`]) learns of them at once.
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake.call.publish(Box::new(wake));
    }

    /// Applies every batch received so far, waiting up to `timeout` for the
    /// first if none is there; whether anything changed.
    pub fn receive(&mut self, timeout: Duration) -> bool {
        let mut changed = false;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            while let Ok(batch) = self.updates.try_recv() {
                changed |= self.apply(batch);
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if changed || left.is_zero() {
                return changed;
            }
            // Batches from other `Updater`s wake no one: they wait for the
            // timeout or the next driver batch.
            let me = std::thread::current();
            if self.wake.parked.load().is_none_or(|t| t.id() != me.id()) {
                self.wake.parked.publish(me);
                // Look again: a batch sent before this was seen unparks no one.
                continue;
            }
            std::thread::park_timeout(left);
        }
    }

    /// Applies a batch; whether it changed the scene (a batch of host
    /// statuses alone does not: no new scene for the renderer to take).
    fn apply(&mut self, batch: Vec<Update>) -> bool {
        let _ = self.in_flight.try_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| Some(n.saturating_sub(1)));
        let (host, batch): (Vec<Update>, Vec<Update>) = batch.into_iter().partition(|u| matches!(u, Update::Host(_)));
        if let Some(Update::Host(h)) = host.last() {
            self.host = Some((*h, std::time::Instant::now()));
        }
        if batch.is_empty() {
            return false;
        }
        for u in batch {
            if let Update::Settings(s) = &u {
                self.new_settings = Some(s.clone());
            }
            self.changes.note(&u);
            self.scene.apply(u);
        }
        self.scene.seq += 1;
        true
    }

    /// The host's status last received and when it came (the renderer's
    /// clock), if the driver sends any: how long the host has not answered
    /// is its `stale_ms` plus the time since (no status for a while is the
    /// driver or the connection gone silent).
    pub fn host(&self) -> Option<(HostStatus, std::time::Instant)> {
        self.host
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// What changed in the scene since the last call (at first: the
    /// world), and the scene.
    pub fn take_changes(&mut self) -> (Changes, &Scene) {
        self.changes_from_seq = self.scene.seq;
        (std::mem::take(&mut self.changes), &self.scene)
    }

    /// Takes the deltas only when `previous` is their actual predecessor.
    /// A consumer restored after a rejected preparation may be older than
    /// the last consumed batch. Rebuild every source-dependent collection
    /// in that case, even if no later update has arrived.
    pub fn take_changes_from(&mut self, previous: u64) -> (Changes, &Scene) {
        let matches = previous == self.changes_from_seq;
        let (changes, scene) = self.take_changes();
        if matches { (changes, scene) }
        else { (Changes { world: true, sprites: true, views: true, slots: true, ..Default::default() }, scene) }
    }

    /// What changed in the scene since [`Display::take_changes`] was last
    /// called, without taking it.
    pub fn changes(&self) -> &Changes {
        &self.changes
    }

    /// The sprite pixels received since the last call (to write into the
    /// atlas once).
    pub fn take_patches(&mut self) -> Vec<Patch> {
        std::mem::take(&mut self.scene.patches)
    }

    /// The settings the driver sent since the last call, if it sent any (the
    /// latest; [`Scene::settings`] keeps them too).
    pub fn take_settings(&mut self) -> Option<viewer_settings::Settings> {
        self.new_settings.take()
    }

    pub fn send_input(&self, input: Input) {
        let _ = self.input.send(input);
    }

    /// Another sender of input to the same driver.
    pub fn input_sender(&self) -> InputSender {
        self.input.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driver_may_be_shared_without_a_lock() {
        fn shared<T: Send + Sync>() {}
        shared::<Driver>();
    }

    #[test]
    fn a_waiting_renderer_wakes_for_each_batch_at_once() {
        let (driver, mut display) = connect();
        let sender = std::thread::spawn(move || {
            for tick in 1..=50 {
                std::thread::sleep(Duration::from_micros(200));
                driver.send(vec![Update::Tick(tick)]);
            }
            driver
        });
        let started = std::time::Instant::now();
        let mut ticks = 0;
        while display.scene().tick != 50 {
            assert!(display.receive(Duration::from_secs(10)) || display.scene().tick == 50);
            ticks += 1;
        }
        assert!(ticks <= 50 && started.elapsed() < Duration::from_secs(5));
        let _driver = sender.join().unwrap();
        // Nothing sent: the wait ends at its timeout.
        assert!(!display.receive(Duration::from_millis(5)));
    }

    #[test]
    fn host_statuses_are_kept_by_the_display_and_alone_are_no_new_scene() {
        let (driver, mut display) = connect();
        assert_eq!(display.host(), None);
        driver.send(vec![Update::Host(HostStatus { stale_ms: 700, serial: 1 })]);
        assert!(!display.receive(Duration::ZERO));
        assert_eq!(display.scene().seq, 0);
        assert_eq!(display.host().map(|h| h.0), Some(HostStatus { stale_ms: 700, serial: 1 }));
        assert!(!driver.busy(), "taken");
        // With other updates: a new scene, the latest status kept.
        driver.send(vec![Update::Tick(3), Update::Host(HostStatus { stale_ms: 0, serial: 2 })]);
        assert!(display.receive(Duration::ZERO));
        assert_eq!((display.scene().seq, display.scene().tick), (1, 3));
        assert_eq!(display.host().map(|h| h.0.serial), Some(2));
    }

    #[test]
    fn updates_and_input_cross_threads() {
        let (driver, mut display) = connect();
        let t = std::thread::spawn(move || {
            let mut s = Scene::empty(WorldShape { cols: 2, rows: 2, ..Default::default() }, 1);
            let empty = Scene::default();
            driver.send(s.updates_since(&empty));
            let before = s.clone();
            s.set(0, 1, 1, 42);
            s.view_mut(0).view.camera_x = 5;
            s.view_mut(3).overlay.push(FreeSprite { set: 9, screen: true, ..Default::default() });
            assert_eq!(s.updates_since(&before).len(), 4);
            driver.send(s.updates_since(&before));
            driver
        });
        let driver = t.join().unwrap();
        // Phases and offsets travel as changes, not as a new world.
        let shape = WorldShape { cols: 2, rows: 1, ..Default::default() };
        let old = Scene::empty_with(shape, [(true, true), (false, false)]);
        let mut new = old.clone();
        new.set_phase(0, 1, 0, 3);
        new.set_offset(0, 0, 0, (-5, 2));
        new.set_offset(1, 0, 0, (9, 9));
        assert_eq!(new.updates_since(&old), vec![Update::Phases(vec![(0, 1, 3)]), Update::Offsets(vec![(0, 0, (-5, 2))])]);
        let mut applied = old.clone();
        new.updates_since(&old).into_iter().for_each(|u| applied.apply(u));
        assert_eq!(applied, new);
        assert_eq!(new.cell_bytes(), 2 * 4 + 2 + 2 * 4 + 2 * 4);
        assert!(display.receive(Duration::from_secs(5)));
        assert_eq!(display.scene().layers[0].values, vec![0, 0, 0, 42]);
        assert_eq!(display.scene().views.iter().map(|v| v.id).collect::<Vec<_>>(), vec![0, 3]);
        assert_eq!(display.scene().view(0).unwrap().view.camera_x, 5);
        assert!(!display.receive(Duration::from_millis(1)));
        display.send_input(Input::Key { vk: 0x0D, down: true, text: None });
        let other = display.input_sender();
        other.send(Input::Quit).unwrap();
        assert_eq!(driver.take_input().len(), 2);
        assert!(driver.take_input().is_empty());
    }

    #[test]
    fn the_display_says_what_changed() {
        let (driver, mut display) = connect();
        let shape = WorldShape { cols: 3, rows: 2, ..Default::default() };
        let mut s = Scene::empty_with(shape, [(true, false), (false, false)]);
        driver.send(s.updates_since(&Scene::default()));
        assert!(display.receive(Duration::from_secs(5)));
        assert!(display.take_changes().0.world);
        assert!(display.take_changes().0.is_empty());
        let before = s.clone();
        s.set(1, 2, 1, 7);
        s.set_phase(0, 0, 1, 3);
        s.view_mut(4).view.camera_x = 1;
        driver.send(s.updates_since(&before));
        assert!(display.receive(Duration::from_secs(5)));
        let (changes, scene) = display.take_changes();
        assert_eq!(changes, Changes { world: false, cells: vec![(1, 5), (0, 3)], sprites: false, views: true, ..Default::default() });
        assert_eq!(scene.layers[1].values[5], 7);
    }

    #[test]
    fn slots_and_bindings_travel_incrementally() {
        let (driver, mut display) = connect();
        let shape = WorldShape { cols: 3, rows: 2, ..Default::default() };
        let base = Scene::empty_with(shape, [(false, false), (false, true)]);
        driver.send(base.updates_since(&Scene::default()));
        assert!(display.receive(Duration::from_secs(5)));
        let _ = display.take_changes();
        // A screen opens in view 1: its own fog (a slot of layer 1) bound.
        let mut s = base.clone();
        let fog = LayerCells { values: vec![2, 2, 0, 2, 2, 2], phases: None, offsets: Some(vec![(0, 0); 6]) };
        s.slots.push(LayerSlot { id: 4, layer: 1, cells: fog });
        s.bindings.push(Binding { id: 1, slots: vec![(1, 4)] });
        s.view_mut(1).view.binding = 1;
        let opened = s.updates_since(&base);
        assert!(opened.contains(&Update::Bindings(s.bindings.clone())));
        driver.send(opened);
        assert!(display.receive(Duration::from_secs(5)));
        let (changes, scene) = display.take_changes();
        assert!(changes.slots && changes.slot_cells.is_empty() && changes.cells.is_empty() && !changes.world);
        assert_eq!((scene.slots.clone(), scene.bindings.clone(), scene.view(1).unwrap().view.binding), (s.slots.clone(), s.bindings.clone(), 1));
        // Its city moves: only the slot's changed values travel.
        let before = s.clone();
        s.slots[0].cells.values[2] = 2;
        s.slots[0].cells.values[0] = 1;
        assert_eq!(s.updates_since(&before), vec![Update::SlotCells(vec![(4, 0, 1), (4, 2, 2)])]);
        driver.send(s.updates_since(&before));
        assert!(display.receive(Duration::from_secs(5)));
        let (changes, scene) = display.take_changes();
        assert_eq!((changes.slots, changes.slot_cells), (false, vec![(4, 0), (4, 2)]));
        assert_eq!(scene.slots, s.slots);
        // It closes: the slot and the binding go.
        let before = s.clone();
        let closed = base.clone();
        let updates = closed.updates_since(&before);
        assert!(updates.contains(&Update::Slot { id: 4, slot: None }) && updates.contains(&Update::Bindings(Vec::new())));
        let mut applied = before.clone();
        updates.into_iter().for_each(|u| applied.apply(u));
        assert_eq!((applied.slots.len(), applied.bindings.len()), (0, 0));
    }

    #[test]
    fn the_driver_wakes_the_renderer_and_the_cursor_travels() {
        let (driver, mut display) = connect();
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        display.set_wake(move || {
            w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let mut s = Scene::default();
        // Nothing to send: no wake.
        driver.send(s.updates_since(&Scene::default()));
        assert_eq!(woken.load(std::sync::atomic::Ordering::SeqCst), 0);
        s.cursor = Cursor::Sprite(7);
        assert_eq!(s.updates_since(&Scene::default()), vec![Update::Cursor(Cursor::Sprite(7))]);
        driver.send(s.updates_since(&Scene::default()));
        assert_eq!(woken.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(display.receive(Duration::ZERO));
        assert_eq!(display.scene().cursor, Cursor::Sprite(7));
    }

    #[test]
    fn settings_go_once_when_they_change_and_are_taken_once() {
        let (driver, mut display) = connect();
        let mut s = Scene::default();
        let mut settings = viewer_settings::Settings::default();
        s.settings = Some(settings.clone());
        let first = s.updates_since(&Scene::default());
        assert_eq!(first, vec![Update::Settings(settings.clone())]);
        let before = s.clone();
        assert!(s.updates_since(&before).is_empty());
        settings.layout.smooth_world_movement = true;
        s.settings = Some(settings.clone());
        assert_eq!(s.updates_since(&before), vec![Update::Settings(settings.clone())]);
        // No settings in a scene: the renderer keeps those it has.
        assert!(Scene::default().updates_since(&s).is_empty());
        driver.send(first);
        driver.send(s.updates_since(&before));
        assert!(display.receive(Duration::from_secs(5)));
        assert_eq!(display.take_settings(), Some(settings.clone()));
        assert_eq!(display.take_settings(), None);
        assert_eq!(display.scene().settings, Some(settings));
    }
    #[test]
    fn settings_preview_metadata_repeats_neither_lease_nor_settings_and_clears_explicitly() {
        let mut active = Scene::default();
        active.settings = Some(viewer_settings::Settings::default());
        active.settings_preview = Some(SettingsPreview {session:1,id:2,pixel_scale:3,previous_pixel_scale:1});
        assert!(active.updates_since(&active).is_empty());
        let mut cleared = active.clone();
        cleared.settings_preview = None;
        let updates = cleared.updates_since(&active);
        assert_eq!(updates,vec![Update::SettingsPreview(None)]);
        for update in updates { active.apply(update); }
        assert_eq!(active,cleared);
    }

}

#[cfg(test)]
mod delta_predecessor_tests {
    use super::*;
    fn display() -> (Driver, Display) {
        let (driver, mut display) = connect();
        assert!(driver.send(vec![Update::World { shape: WorldShape { cols: 2, rows: 1, ..Default::default() }, layers: vec![LayerCells { values: vec![1; 2], ..Default::default() }] }]));
        display.receive(Duration::ZERO);
        display.take_changes_from(0);
        (driver, display)
    }
    fn assert_full(changes: Changes) {
        assert!(changes.world && changes.sprites && changes.views && changes.slots);
        assert!(changes.cells.is_empty() && changes.slot_cells.is_empty());
    }
    #[test]
    fn consumed_batch_requires_full_recovery_from_an_older_predecessor() {
        let (driver, mut display) = display();
        assert!(driver.send(vec![Update::Cells(vec![(0, 0, 9)])]));display.receive(Duration::ZERO);
        let changes = display.take_changes_from(1).0;
        assert!(!changes.world);
        assert_eq!(changes.cells, [(0, 0)]);
        assert_full(display.take_changes_from(1).0);
        assert_eq!(display.scene().layers[0].values, [9, 1]);
    }
    #[test]
    fn legacy_take_changes_also_advances_the_delta_predecessor() {
        let (driver, mut display) = display();
        assert!(driver.send(vec![Update::Cells(vec![(0, 0, 9)])]));display.receive(Duration::ZERO);
        display.take_changes();
        assert!(driver.send(vec![Update::Cells(vec![(0, 1, 7)])]));display.receive(Duration::ZERO);
        assert_full(display.take_changes_from(1).0);
        assert_eq!(display.scene().layers[0].values, [9, 7]);
    }
    #[test]
    fn matching_control_only_and_empty_batches_keep_incremental_invalidation() {
        let (driver, mut display) = display();
        assert!(driver.send(vec![Update::Tick(17)]));display.receive(Duration::ZERO);
        assert!(display.take_changes_from(1).0.is_empty());
        assert_eq!(display.scene().tick, 17);
        assert!(display.take_changes_from(2).0.is_empty());
    }
}

/// World rectangle `rect` (x, y, w, h) a renderer reported for views shown
/// `from` times smaller ([`Input::Views`], [`Input::MainView`]) as it shows
/// once they are shown `to` times smaller: a renderer keeps a view's centre
/// on the same place of the world and its size on the screen, so the centre
/// scales by `from / to` and the size stays (w 0: unused, as it is).
pub fn rescale_view_rect(rect: (i32, i32, i32, i32), from: u32, to: u32) -> (i32, i32, i32, i32) {
    let (from, to) = (from.max(1) as i64, to.max(1) as i64);
    let (x, y, w, h) = rect;
    if from == to || w <= 0 || h <= 0 {
        return rect;
    }
    // (Twice the centre, so odd sizes round as the renderer's centres do.)
    let centre2 = |p: i32, len: i32| (2 * p as i64 + len as i64) * from / to;
    let place = |c2: i64, len: i32| ((c2 - len as i64).div_euclid(2)) as i32;
    (place(centre2(x, w), w), place(centre2(y, h), h), w, h)
}

#[cfg(test)]
mod rescale_view_rect_tests {
    use super::rescale_view_rect;

    #[test]
    fn a_view_keeps_its_centre_and_screen_size_across_a_zoom() {
        // Full size to half size: the centre halves, the size stays.
        assert_eq!(rescale_view_rect((1000, 400, 800, 600), 1, 2), (300, 50, 800, 600));
        // And back again.
        assert_eq!(rescale_view_rect((300, 50, 800, 600), 2, 1), (1000, 400, 800, 600));
        // 0 is full size; nothing changes at the same zoom or unused.
        assert_eq!(rescale_view_rect((1000, 400, 800, 600), 0, 1), (1000, 400, 800, 600));
        assert_eq!(rescale_view_rect((0, 0, 0, 0), 1, 2), (0, 0, 0, 0));
        // Negative coordinates (round the wrap) keep their centre.
        assert_eq!(rescale_view_rect((-400, -300, 200, 100), 1, 2), (-250, -175, 200, 100));
    }
}
