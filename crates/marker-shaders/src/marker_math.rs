//! The gilded arrow over the active selection and its ground shadow,
//! painted at any scale: shared by a render description's 1:1 art, the
//! GPU (painting them at the target's resolution) and its CPU oracle.

#![allow(unexpected_cfgs)]
#[cfg(target_arch = "spirv")]
use spirv_std::num_traits::Float;

/// The shapes [`paint`] paints (the painter's shape numbers).
pub const SHAPE_ARROW: u32 = 0;
pub const SHAPE_SHADOW: u32 = 1;

/// The arrow's 1:1 art: its size, and its tip in it.
pub const ARROW_ART: (f32, f32) = (33.0, 48.0);
pub const ARROW_TIP: (f32, f32) = (16.0, 44.0);

/// Half the width and height of the arrow's ground shadow, in pixels of
/// the 1:1 art, with the arrow `bob` of them below its rest: wider the
/// lower it is (41 by 13 at rest).
pub fn shadow_half(bob: f32) -> (f32, f32) {
    ((41.0 + bob) / 2.0, (13.0 + bob / 2.0) / 2.0)
}

/// Shape `shape` ([`SHAPE_ARROW`] or [`SHAPE_SHADOW`], the latter shaped by
/// `bob`) over the pixel whose top-left is (x, y) from its reference point
/// (the arrow's tip, the shadow's centre), at `scale`, in `rgba`: RGBA
/// 0..255 (see [`arrow_pixel`], [`shadow_pixel`]).
pub fn paint(shape: u32, x: f32, y: f32, scale: f32, bob: f32, rgba: [f32; 4]) -> [f32; 4] {
    if shape == SHAPE_ARROW { arrow_pixel(x, y, scale, rgba) } else { shadow_pixel(x, y, scale, bob, rgba) }
}

/// The arrow's outline in the pixels of its 1:1 art (33×48): slightly
/// unequal shoulders and a tapered, chamfered shaft, the tip at (16, 44).
const POINTS: [(f32, f32); 11] = [(13.0, 3.0), (21.0, 3.6), (23.0, 5.5), (22.0, 23.0),
    (29.5, 23.8), (29.0, 27.0), (16.0, 44.0), (3.5, 26.0),
    (4.0, 23.0), (11.5, 23.4), (11.0, 5.0)];

/// Whether art point (x, y) is inside the arrow, how far it is from the
/// outline, and how the nearest edge faces the light.
fn surface(x: f32, y: f32) -> (bool, f32, f32) {
    let mut inside = false;
    let (mut distance, mut light) = (f32::MAX, 0.0);
    let mut i = 0;
    while i < POINTS.len() {
        let a = POINTS[i];
        let b = POINTS[if i + 1 == POINTS.len() { 0 } else { i + 1 }];
        if (a.1 > y) != (b.1 > y) && x < (b.0-a.0)*(y-a.1)/(b.1-a.1)+a.0 { inside = !inside; }
        let (dx, dy) = (b.0-a.0, b.1-a.1);
        let t = ((x-a.0)*dx+(y-a.1)*dy)/(dx*dx+dy*dy);
        let t = t.clamp(0.0, 1.0);
        let d = ((x-a.0-t*dx).powi(2)+(y-a.1-t*dy).powi(2)).sqrt();
        if d < distance {
            distance = d;
            light = (dx-dy)/(dx*dx+dy*dy).sqrt() * 0.707;
        }
        i += 1;
    }
    (inside, distance, light)
}

/// The grain of 1:1 art pixel (gx, gy).
fn grain_at(gx: i32, gy: i32) -> f32 {
    (gx*37+gy*61+gx*gy*7).rem_euclid(17) as f32/16.0-0.5
}

/// The arrow's colour (RGB, 0..255) at art point (px, py), its metal's
/// midtone `color`; `false` outside it. Warm metal with a broad bevel, a
/// brown ink edge, a rounded central ridge and patches of tarnish. Its
/// grain varies per pixel of the 1:1 art; `smooth`: blended between them
/// (painted larger, it shows no blocks of them).
pub fn sample(px: f32, py: f32, color: [f32; 3], smooth: bool) -> (bool, [f32; 3]) {
    let (inside, edge, light) = surface(px, py);
    if !inside { return (false, [0.0; 3]); }
    let grain = if smooth {
        let (x, y) = (px - 0.5, py - 0.5);
        let (gx, gy) = (x.floor(), y.floor());
        let (fx, fy) = (x - gx, y - gy);
        let (gx, gy) = (gx as i32, gy as i32);
        let top = grain_at(gx, gy) * (1.0 - fx) + grain_at(gx + 1, gy) * fx;
        let bottom = grain_at(gx, gy + 1) * (1.0 - fx) + grain_at(gx + 1, gy + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    } else {
        grain_at(px.floor() as i32, py.floor() as i32)
    };
    let mottle = (px*0.77+py*0.32).sin()*(py*0.63-px*0.21).sin();
    let worn_edge = 0.95+grain*0.30+(py*1.7).sin()*0.12;
    let bevel = ((3.8-edge)/2.8).clamp(0.0, 1.0);
    let ridge = (-((px-16.0)/3.7).powi(2)).exp()*0.17;
    let shade = 0.76+ridge+0.08*(py*0.16).cos()+mottle*0.055+grain*0.045;
    let rims = if light > 0.0 { [255.0, 239.0, 167.0] } else { [96.0, 53.0, 22.0] };
    let inks = [55.0, 34.0, 19.0];
    let mut out = [0.0; 3];
    let mut c = 0;
    while c < 3 {
        let body = color[c]*shade;
        let rim = rims[c];
        let metal = body+(rim-body)*bevel*light.abs().sqrt();
        let ink = inks[c]+mottle*7.0;
        let t = ((edge-worn_edge)*2.2).clamp(0.0, 1.0);
        out[c] = ink+(metal-ink)*t;
        c += 1;
    }
    (true, out)
}

/// The arrow (in `rgba`: its midtone and opacity) over the pixel of a
/// stage showing it `scale` times as large as its 1:1 art whose top-left
/// is (x, y) from the tip: RGBA 0..255, from 4×4 samples, as the 1:1 art
/// paints its pixels.
pub fn arrow_pixel(x: f32, y: f32, scale: f32, rgba: [f32; 4]) -> [f32; 4] {
    let mut sum = [0.0f32; 4];
    let mut k = 0;
    while k < 16 {
        let (sx, sy) = ((k % 4) as f32, (k / 4) as f32);
        let px = (x + (sx + 0.5) / 4.0) / scale + ARROW_TIP.0;
        let py = (y + (sy + 0.5) / 4.0) / scale + ARROW_TIP.1;
        let (inside, c) = sample(px, py, [rgba[0], rgba[1], rgba[2]], true);
        if inside {
            sum[0] += c[0];
            sum[1] += c[1];
            sum[2] += c[2];
            sum[3] += 1.0;
        }
        k += 1;
    }
    if sum[3] == 0.0 { return [0.0; 4]; }
    let channel = |v: f32| (v / sum[3]).clamp(0.0, 255.0).floor();
    [channel(sum[0]), channel(sum[1]), channel(sum[2]), (sum[3] * rgba[3] / 16.0).round()]
}

/// The arrow's ground shadow over the pixel whose top-left is (x, y) from
/// its centre, at `scale` (as [`arrow_pixel`]), with the arrow `bob` 1:1
/// pixels below its rest: wider and darker the lower the arrow is
/// (a soft ellipse: 41 by 13 pixels at rest, alpha 67 of `rgba`'s 255).
/// RGBA 0..255.
pub fn shadow_pixel(x: f32, y: f32, scale: f32, bob: f32, rgba: [f32; 4]) -> [f32; 4] {
    let v = bob;
    let (hx, hy) = shadow_half(v);
    let (rx, ry) = (hx * scale, hy * scale);
    let (dx, dy) = ((x + 0.5) / rx, (y + 0.5) / ry);
    let r = dx * dx + dy * dy;
    let alpha = ((-3.5 * r).exp() - (-3.5f32).exp()).max(0.0);
    [rgba[0], rgba[1], rgba[2], (alpha * (67.0 + 3.0 * v) * rgba[3] / 255.0).round()]
}
