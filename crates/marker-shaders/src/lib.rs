//! The selection marker's shader maths: the gilded arrow over the active
//! selection and its ground shadow (`marker_math`, shared with the host's
//! CPU oracle and 1:1 art), painted per pixel at the stage's scale.
//!
//! It has no entry point of its own: the arrow and its shadow are painted
//! shapes of whatever draws them (a tile renderer's free sprites), whose
//! shader module wires [`paint`] into that drawing's fragment (see
//! `viewer-shaders`).
#![cfg_attr(target_arch = "spirv", no_std)]

mod marker_math;

pub use marker_math::{ARROW_ART, ARROW_TIP, SHAPE_ARROW, SHAPE_SHADOW, paint, shadow_half};
