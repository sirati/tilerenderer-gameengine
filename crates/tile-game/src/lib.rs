//! Concepts of tile-based strategy games that are not any one game's: a
//! game's crate maps its own onto them (its towns onto sites, its build
//! list onto a production queue) and gets the viewer's behaviour for them.
//!
//! Each concept is an extension of the scene protocol (`scene::Extension`:
//! the driver sends it in `scene::Interface::extensions`, the viewer
//! answers with `scene::Input::Extension`) and, where the viewer does
//! something with it, a `tile_viewer::extension::InterfaceExtension`.
//!
//! - [`production_queue`]: a site's queue of things to produce, reordered
//!   by dragging its rows.

#![forbid(unsafe_code)]

pub mod production_queue;
