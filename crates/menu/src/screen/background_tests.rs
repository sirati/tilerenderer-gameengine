//! Caller-owned source masks preserve the original screen geometry and
//! nearest alpha grid while selecting only texture colours for dithering.

use super::*;
use render_desc::{Description, ImageLoader};
use tile_render::{Blend, Camera, PlacedSprite, RgbaImage, SpriteSetId, reference};

const CLEAR: [u8; 4] = [17, 23, 31, 255];
const SIZE: (u32, u32) = (96, 80);
const CROP: [i32; 4] = [7, 5, 64, 48];

struct MixedArt;

impl ImageLoader for MixedArt {
    fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
        assert_eq!(source, "mixed-neutral.pcx");
        let keyed = options.get("keyed").and_then(toml::Value::as_bool).unwrap_or(true);
        Ok(vec![RgbaImage::from_fn(96, 80, |x, y| {
            let alpha = if keyed { match (x + 2 * y) % 7 { 0 => 0, 1 => 91, 2 => 173, _ => 255 } } else { 255 };
            [if (x + y) % 2 == 0 { 13 } else { 239 }, ((11 * x + 7 * y) % 251) as u8, 71, alpha]
        })])
    }

    fn sizes(&self, source: &str, _: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
        assert_eq!(source, "mixed-neutral.pcx");
        Ok(vec![(96, 80)])
    }
}

fn policy(region: &Region) -> Vec<[i32; 4]> {
    if region.source == "mixed-neutral.pcx" {
        // Overlap, a one-source-pixel strip and a rectangle extending
        // outside the crop exercise union and crop-local coordinates.
        vec![[15, 12, 32, 28], [39, 25, 24, 23], [11, 9, 1, 37], [60, 42, 20, 20], [15, 12, 32, 28]]
    } else {
        Vec::new()
    }
}

fn region(keyed: bool) -> Region {
    Region { source: "mixed-neutral.pcx".into(), rect: CROP, keyed, mask: None, palette: None }
}

fn element(keyed: bool, at: (i32, i32), opacity: u8, clipped: bool) -> Element {
    let clip = if clipped { [at.0 + 3, at.1 + 4, at.0 + 60, at.1 + 44] } else { [0, 0, SIZE.0 as i32, SIZE.1 as i32] };
    if opacity == 255 {
        Element::Image { region: region(keyed), at, clip }
    } else {
        Element::OpacityImage { region: region(keyed), at, clip, opacity }
    }
}

fn description(art: &ScreenArt, split: bool) -> Description {
    let text = if split {
        art.description_with_backgrounds("keyed = true", "keyed = false", |_, _| String::new(), policy)
    } else {
        art.description("keyed = true", "keyed = false", |_, _| String::new())
    };
    Description::parse(&format!("grid = {{ layout = 'orthogonal', step = [1, 1] }}\n{text}")).unwrap()
}

fn compile(element: &Element) -> (Compiled, Compiled) {
    let mut art = ScreenArt::default();
    art.add(std::slice::from_ref(element));
    (description(&art, false).compile(&MixedArt, 128).unwrap(), description(&art, true).compile(&MixedArt, 128).unwrap())
}

fn render(elements: &[Element], compiled: &Compiled, split: bool, zoom: (u32, u32), hairline: bool) -> RgbaImage {
    let sprites = if split { sprites_with_backgrounds(elements, compiled, 7, policy) } else { sprites(elements, compiled, 7) };
    let mut world = compiled.world(1, 1, false, false);
    world.free = sprites.into_iter().map(|s| PlacedSprite {
        screen: s.screen,
        view: Some(0),
        after_pass: s.after_pass as usize,
        phase: s.phase,
        ticks_per_frame: s.ticks_per_frame,
        blend: Blend::from_scale_den(s.scale_den, s.tint),
        clip: s.clip,
        palette: s.palette,
        ..PlacedSprite::new(SpriteSetId::from_raw(s.set), s.x, s.y)
    }).collect();
    reference::render(&world, &compiled.atlas, &Camera { cells: false, zoom, hairline, ..Camera::new(SIZE) }, 0, CLEAR)
}

#[test]
fn partial_backgrounds_keep_full_source_cells_and_screen_input_geometry() {
    let elements = [element(true, (5, 7), 255, false), element(true, (19, 13), 137, true)];
    let original_elements = elements.clone();
    let mut art = ScreenArt::default();
    art.add(&elements);
    let original_art = art.clone();
    let lines = art.to_lines();
    let desc = description(&art, true);
    assert_eq!(desc.sheets.len(), 2, "copies share the same source region and complementary mask pair");
    let mut masks = desc.sheets.values().collect::<Vec<_>>();
    masks.sort_by_key(|sheet| sheet.alpha_keep.is_some());
    let expected_rects = vec![[8, 7, 32, 28], [32, 20, 24, 23], [4, 4, 1, 37], [53, 37, 11, 11]];
    assert_eq!(masks[0].alpha_clear, Some(expected_rects.clone()));
    assert_eq!(masks[1].alpha_keep, Some(expected_rects));
    for sheet in &masks {
        assert_eq!((sheet.origin, sheet.cell, sheet.grid, sheet.anchor), ([7, 5], [64, 48], Some([1, 1]), [0, 0]));
    }
    let compiled = desc.compile(&MixedArt, 128).unwrap();
    let draws = sprites_with_backgrounds(&elements, &compiled, 7, policy);
    assert_eq!(draws.len(), 4);
    for (e, pair) in elements.iter().zip(draws.chunks_exact(2)) {
        let (at, clip, opacity) = match e {
            Element::Image { at, clip, .. } => (*at, *clip, 255),
            Element::OpacityImage { at, clip, opacity, .. } => (*at, *clip, *opacity),
            _ => unreachable!(),
        };
        for draw in pair {
            assert_eq!((draw.x, draw.y, draw.clip, draw.after_pass), (at.0, at.1, Some(clip), 7));
            assert_eq!(255 - (draw.tint >> 24), u32::from(opacity));
            let id = compiled.sets.frame(SpriteSetId::from_raw(draw.set), 0).unwrap();
            let sprite = compiled.atlas.sprites[id.0 as usize];
            assert_eq!((sprite.width, sprite.height, sprite.sampling_origin_x, sprite.sampling_origin_y), (64, 48, 7, 5));
        }
        assert_ne!(pair[0].set, pair[1].set);
    }
    assert_eq!(elements, original_elements, "render metadata must not change input regions, positions or clips");
    assert_eq!(art, original_art);
    assert_eq!(art.to_lines(), lines);
    assert_eq!(ScreenArt::parse_lines(&lines), art, "preload round trips retain the original unsplit region identities");
    assert_eq!(description(&ScreenArt::parse_lines(&lines), true).sheets.len(), 2);
}

#[test]
fn complementary_background_masks_match_unsplit_nearest_at_every_scale_without_seams() {
    for keyed in [false, true] {
        for opacity in [255, 137] {
            for (at, clipped) in [((5, 7), false), ((19, 13), true), ((-9, 3), true)] {
                let elements = [element(keyed, at, opacity, clipped)];
                let (original, mut split) = compile(&elements[0]);
                for sprite in &mut split.atlas.sprites { sprite.sampling = 0; }
                for zoom in [(1, 1), (2, 1), (3, 2), (2, 3), (3, 7), (1, 4), (1, 8)] {
                    for hairline in [false, true] {
                        let expected = render(&elements, &original, false, zoom, hairline);
                        let actual = render(&elements, &split, true, zoom, hairline);
                        assert!(expected.pixels.iter().any(|&pixel| pixel != CLEAR), "the original scene must draw visible pixels");
                        assert_eq!(actual.pixels, expected.pixels, "complementary masks changed nearest output at {zoom:?}, hairline {hairline}, opacity {opacity}, keyed {keyed}, at {at:?}, clipped {clipped}");
                    }
                }
            }
        }
    }
}

fn nominal_source(element: &Element, x: u32, y: u32, (n, d): (u32, u32)) -> (i64, i64) {
    let (region, at, clip) = match element {
        Element::Image { region, at, clip } | Element::OpacityImage { region, at, clip, .. } => (region, at, clip),
        _ => unreachable!(),
    };
    let sx = (i64::from(x) * i64::from(d) / i64::from(n)).max(i64::from(at.0.max(clip[0]))) - i64::from(at.0) + i64::from(region.rect[0]);
    let sy = (i64::from(y) * i64::from(d) / i64::from(n)).max(i64::from(at.1.max(clip[1]))) - i64::from(at.1) + i64::from(region.rect[1]);
    (sx, sy)
}

#[test]
fn background_sampling_changes_only_verified_texture_colours_and_preserves_alpha() {
    let mut changed_per_scale = [0; 5];
    for opacity in [255, 137] {
        for (at, clipped) in [((5, 7), false), ((19, 13), true), ((-9, 3), true)] {
            let elements = [element(true, at, opacity, clipped)];
            let (original, split) = compile(&elements[0]);
            for zoom in [(1, 1)] {
                assert_eq!(render(&elements, &split, true, zoom, true).pixels, render(&elements, &original, false, zoom, true).pixels, "sampling must remain exact nearest at {zoom:?}");
            }
            for (case, zoom) in [(2, 3), (3, 7), (1, 8), (2, 1), (3, 2)].into_iter().enumerate() {
                let expected = render(&elements, &original, false, zoom, true);
                let actual = render(&elements, &split, true, zoom, true);
                for y in 0..SIZE.1 {
                    for x in 0..SIZE.0 {
                        let a = actual.get(x, y);
                        let b = expected.get(x, y);
                        assert_eq!((a[2], a[3]), (b[2], b[3]), "partial/keyed alpha or draw opacity changed at ({x}, {y}) zoom {zoom:?}");
                        if a != b {
                            changed_per_scale[case] += 1;
                            let (sx, sy) = nominal_source(&elements[0], x, y, zoom);
                            assert!(policy(&region(true)).iter().any(|&[rx, ry, w, h]| sx >= i64::from(rx) && sx < i64::from(rx + w) && sy >= i64::from(ry) && sy < i64::from(ry + h)), "control pixels outside verified texture union changed at ({x}, {y}), source ({sx}, {sy}), zoom {zoom:?}");
                        }
                    }
                }
            }
        }
    }
    assert!(changed_per_scale.iter().all(|&n| n > 0), "texture dithering must be exercised at every scale: {changed_per_scale:?}");
}

#[test]
fn partial_background_readiness_requires_both_halves_and_never_draws_a_half_pair() {
    let elements = [element(true, (5, 7), 255, false), element(true, (19, 13), 137, true)];
    let mut art = ScreenArt::default();
    art.add(&elements);
    let ordinary = description(&art, false).compile(&MixedArt, 128).unwrap();
    assert!(elements.iter().all(|e| e.ready(&ordinary)));
    assert!(elements.iter().all(|e| !e.ready_with_backgrounds(&ordinary, policy)));
    assert!(sprites_with_backgrounds(&elements, &ordinary, 7, policy).is_empty(), "an unsplit cache entry is not half of a split draw");
    let desc = description(&art, true);
    for keep in [false, true] {
        let mut incomplete = desc.clone();
        incomplete.sheets.retain(|_, sheet| sheet.alpha_keep.is_some() == keep);
        let incomplete = incomplete.compile(&MixedArt, 128).unwrap();
        assert!(elements.iter().all(|e| !e.ready_with_backgrounds(&incomplete, policy)));
        assert!(sprites_with_backgrounds(&elements, &incomplete, 7, policy).is_empty(), "a lone complementary half must never draw");
    }
    let complete = desc.compile(&MixedArt, 128).unwrap();
    let layout = desc.compile_layout(&MixedArt, 128).unwrap();
    for compiled in [&complete, &complete.without_pixels(), &layout, &layout.without_pixels()] {
        assert!(elements.iter().all(|e| e.ready_with_backgrounds(compiled, policy)));
        assert_eq!(sprites_with_backgrounds(&elements, compiled, 7, policy).len(), 4);
    }
}
