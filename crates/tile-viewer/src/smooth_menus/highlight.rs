//! The driver's pointer highlight in a list, kept on the row under the
//! pointer while the list slides.
//!
//! The driver draws its highlight into the list (a list box's selection
//! that follows the pointer: a fill and another text colour). The viewer
//! has no art for it, so it learns the highlight from the driver's own
//! drawing: one row seen plain and then highlighted, with nothing else of
//! the list changed, while the driver's pointer is over that row. What the
//! highlight adds and which colours it changes must turn that row's plain
//! drawing into its highlighted one exactly, and back, or nothing is
//! learned. A learned highlight then moves from the row the driver lit to
//! the row under the pointer, in every frame, in either drawing of a
//! slide.

use tile_render::{Blend, PlacedSprite};

use super::{Picture, Rect, shifted};

/// A colour a highlight changes: text (outline glyph or ClearType colour)
/// or a solid fill.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Colour {
    Text([u8; 3]),
    Fill([u8; 4]),
}

fn colour(s: &PlacedSprite) -> Option<Colour> {
    if let Some(solid) = s.solid { return Some(Colour::Fill(solid.rgba)) }
    if let Some(text) = s.text { return Some(Colour::Text(text.color)) }
    match s.blend {
        Blend::Subpixel { color } => Some(Colour::Text(color)),
        _ => None,
    }
}

fn recoloured(mut s: PlacedSprite, c: Colour) -> PlacedSprite {
    match c {
        Colour::Fill(rgba) => if let Some(solid) = &mut s.solid { solid.rgba = rgba },
        Colour::Text(rgb) => {
            if let Some(text) = &mut s.text { text.color = rgb }
            if let Blend::Subpixel { color } = &mut s.blend { *color = rgb }
        }
    }
    s
}

/// What a sprite is apart from its colour and clip (rows are compared
/// relative to their tops; edge rows are clipped differently).
fn shape(s: &PlacedSprite) -> PlacedSprite {
    let mut s = *s;
    s.clip = None;
    match colour(&s) {
        Some(Colour::Text(_)) => recoloured(s, Colour::Text([0; 3])),
        Some(Colour::Fill(_)) => recoloured(s, Colour::Fill([0; 4])),
        None => s,
    }
}

/// A row's own sprites, relative to its top, with their extents.
type Drawing = Vec<(PlacedSprite, Rect)>;

/// Where an added sprite goes among a row's own sprites: before the n-th
/// from the front, or after the n-th from the back (rows differ in their
/// number of glyphs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum At {
    Front(usize),
    Back(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Highlight {
    added: Vec<(At, PlacedSprite, Rect)>,
    /// Plain colour, highlighted colour.
    colours: Vec<(Colour, Colour)>,
}

impl Highlight {
    /// What turns `plain` into `lit` (a row's drawings, relative): sprites
    /// added, colours changed, and nothing else, reversibly.
    fn learn(plain: &[(PlacedSprite, Rect)], lit: &[(PlacedSprite, Rect)]) -> Option<Self> {
        let mut i = 0;
        let mut added = Vec::new();
        let mut colours: Vec<(Colour, Colour)> = Vec::new();
        let mut kept: Vec<Colour> = Vec::new();
        for (l, e) in lit {
            if plain.get(i).is_some_and(|(p, pe)| shape(p) == shape(l) && pe == e) {
                match (colour(&plain[i].0), colour(l)) {
                    (Some(a), Some(b)) if a != b => {
                        // One plain colour has one highlighted colour, and back.
                        if colours.iter().any(|&(x, y)| (x == a) != (y == b)) { return None }
                        if !colours.contains(&(a, b)) { colours.push((a, b)) }
                    }
                    (Some(a), _) => if !kept.contains(&a) { kept.push(a) },
                    _ => {}
                }
                i += 1;
            } else {
                added.push((i, *l, *e));
            }
        }
        if i != plain.len() || (added.is_empty() && colours.is_empty()) { return None }
        // Undoing it must not touch a colour the row keeps.
        if colours.iter().any(|(a, b)| kept.contains(a) || kept.contains(b) || colours.iter().any(|(x, _)| x == b)) { return None }
        let n = plain.len();
        let added = added.into_iter().map(|(k, s, e)| (if k <= n - k { At::Front(k) } else { At::Back(n - k) }, s, e)).collect();
        let h = Highlight { added, colours };
        (h.lit(plain).as_deref() == Some(lit) && h.plain(lit).as_deref() == Some(plain)).then_some(h)
    }

    fn recolour(&self, s: PlacedSprite, back: bool) -> PlacedSprite {
        let Some(c) = colour(&s) else { return s };
        match self.colours.iter().find(|&&(a, b)| if back { b == c } else { a == c }) {
            Some(&(a, b)) => recoloured(s, if back { a } else { b }),
            None => s,
        }
    }

    /// A plain row highlighted.
    fn lit(&self, plain: &[(PlacedSprite, Rect)]) -> Option<Drawing> {
        if plain.is_empty() { return None }
        let n = plain.len();
        let at = |a: At| match a { At::Front(k) => k.min(n), At::Back(k) => n.saturating_sub(k) };
        let mut out = Vec::with_capacity(n + self.added.len());
        for k in 0..=n {
            out.extend(self.added.iter().filter(|(a, ..)| at(*a) == k).map(|&(_, s, e)| (s, e)));
            if let Some(&(s, e)) = plain.get(k) { out.push((self.recolour(s, false), e)) }
        }
        Some(out)
    }

    /// A highlighted row plain; `None` if it is not highlighted.
    fn plain(&self, lit: &[(PlacedSprite, Rect)]) -> Option<Drawing> {
        let mut out = lit.to_vec();
        for (_, a, _) in &self.added {
            let before = out.len();
            // A row straddling a slide's seam holds two clipped copies.
            out.retain(|(s, _)| shape(s) != shape(a) || colour(s) != colour(a));
            if out.len() == before { return None }
        }
        if self.added.is_empty() && !out.iter().any(|(s, _)| colour(s).is_some_and(|c| self.colours.iter().any(|&(_, b)| b == c))) {
            return None;
        }
        Some(out.into_iter().map(|(s, e)| (self.recolour(s, true), e)).collect())
    }
}

/// Each row's own sprites (indices into the picture's), for a list of one
/// column: those whose middle lies in the row and that are not much
/// taller than it (the list's background is no row's).
fn members(picture: &Picture) -> Option<Vec<Vec<usize>>> {
    let rows = &picture.rows;
    for (k, a) in rows.iter().enumerate() {
        if rows[k + 1..].iter().any(|b| a.rect[1] < b.rect[3] && b.rect[1] < a.rect[3]) { return None }
    }
    let mut out = vec![Vec::new(); rows.len()];
    for (k, e) in picture.extents.iter().enumerate() {
        let (middle, height) = ((e[1] + e[3]) / 2, e[3] - e[1]);
        if let Some(row) = rows.iter().position(|r| middle >= r.rect[1] && middle < r.rect[3] && 2 * height <= 3 * (r.rect[3] - r.rect[1])) {
            out[row].push(k);
        }
    }
    Some(out)
}

fn drawing(picture: &Picture, members: &[usize], top: i32) -> Drawing {
    members.iter().map(|&k| {
        let mut s = picture.sprites[k];
        s.y -= top;
        s.clip = s.clip.map(|c| shifted(c, -top));
        (s, shifted(picture.extents[k], -top))
    }).collect()
}

/// The highlight `new` shows on the row under the driver's pointer
/// (`hover`, its pixels), if `old` showed that row plain and nothing else
/// of the list changed but the row the highlight left.
pub(super) fn learn(old: &Picture, new: &Picture, hover: (i32, i32)) -> Option<Highlight> {
    if old.rect != new.rect || old.rows != new.rows || old.cameras != new.cameras { return None }
    let (a, b) = (members(old)?, members(new)?);
    let draw = |k: usize| {
        let top = new.rows[k].rect[1];
        (drawing(old, &a[k], top), drawing(new, &b[k], top))
    };
    let changed: Vec<usize> = (0..new.rows.len()).filter(|&k| { let (o, n) = draw(k); o != n }).collect();
    let under = new.row_at(hover)?.id;
    let y = new.rows.iter().position(|r| r.id == under)?;
    let r = new.rows[y].rect;
    if !changed.contains(&y) || changed.len() > 2 || r[1] < new.rect[1] || r[3] > new.rect[3] { return None }
    let (plain, lit) = draw(y);
    let h = Highlight::learn(&plain, &lit)?;
    for &x in changed.iter().filter(|&&x| x != y) {
        let (was, is) = draw(x);
        if h.plain(&was) != Some(is) { return None }
    }
    Some(h)
}

/// `picture` with the driver's highlight moved to row `want` (or only
/// removed, if `want` is not among its rows); `None` where it shows no
/// single highlighted row or already highlights `want`.
pub(super) fn restyle(picture: &Picture, h: &Highlight, want: u64) -> Option<Picture> {
    let members = members(picture)?;
    let drawings: Vec<Drawing> = picture.rows.iter().zip(&members).map(|(r, m)| drawing(picture, m, r.rect[1])).collect();
    let lit: Vec<usize> = (0..drawings.len()).filter(|&k| !drawings[k].is_empty() && h.plain(&drawings[k]).is_some()).collect();
    let [on] = lit[..] else { return None };
    let target = picture.rows.iter().position(|r| r.id == want);
    if target == Some(on) { return None }
    let mut replace = vec![(on, h.plain(&drawings[on])?)];
    if let Some(t) = target {
        replace.push((t, h.lit(&drawings[t])?));
    }
    let mut owner = vec![None; picture.sprites.len()];
    for (row, m) in members.iter().enumerate() {
        for &k in m { owner[k] = Some(row) }
    }
    let (mut sprites, mut extents) = (Vec::new(), Vec::new());
    let mut placed = vec![false; picture.rows.len()];
    for (k, s) in picture.sprites.iter().enumerate() {
        match owner[k].and_then(|row| replace.iter().find(|(r, _)| *r == row)) {
            Some((row, new)) => {
                if std::mem::replace(&mut placed[*row], true) { continue }
                let top = picture.rows[*row].rect[1];
                for &(mut s, e) in new {
                    s.y += top;
                    s.clip = s.clip.map(|c| shifted(c, top));
                    sprites.push(s);
                    extents.push(shifted(e, top));
                }
            }
            None => {
                sprites.push(*s);
                extents.push(picture.extents[k]);
            }
        }
    }
    Some(Picture { id: picture.id, rect: picture.rect, rows: picture.rows.clone(), sprites, extents, cameras: picture.cameras.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tile_render::{SolidRect, SpriteSetId};

    fn glyph(set: u32, x: i32, y: i32, rgb: [u8; 3]) -> (PlacedSprite, Rect) {
        let s = PlacedSprite { screen: true, blend: Blend::Subpixel { color: rgb }, ..PlacedSprite::new(SpriteSetId::from_raw(set), x, y) };
        (s, [x, y, x + 6, y + 10])
    }
    fn fill(rgba: [u8; 4]) -> (PlacedSprite, Rect) {
        let s = PlacedSprite { screen: true, solid: Some(SolidRect { width: 100, height: 20, rgba }), ..PlacedSprite::new(SpriteSetId::from_raw(0), 0, 0) };
        (s, [0, 0, 100, 20])
    }
    const BLACK: [u8; 3] = [0, 0, 0];
    const BLUE: [u8; 3] = [0, 112, 184];
    const BAR: [u8; 4] = [40, 40, 120, 255];

    #[test]
    fn a_fill_and_recoloured_text_are_learned_and_move_to_rows_of_other_lengths() {
        let plain = vec![glyph(1, 10, 5, BLACK), glyph(2, 16, 5, BLACK)];
        let lit = vec![fill(BAR), glyph(1, 10, 5, BLUE), glyph(2, 16, 5, BLUE)];
        let h = Highlight::learn(&plain, &lit).unwrap();
        let other = vec![glyph(7, 10, 5, BLACK), glyph(8, 16, 5, BLACK), glyph(9, 22, 5, BLACK)];
        let other_lit = h.lit(&other).unwrap();
        assert_eq!(other_lit, vec![fill(BAR), glyph(7, 10, 5, BLUE), glyph(8, 16, 5, BLUE), glyph(9, 22, 5, BLUE)]);
        assert_eq!(h.plain(&other_lit), Some(other.clone()));
        assert_eq!(h.plain(&other), None);
    }

    #[test]
    fn changes_that_are_not_a_highlight_are_not_learned() {
        let plain = vec![glyph(1, 10, 5, BLACK)];
        // Another glyph (a redrawn count), not an addition or a colour.
        assert_eq!(Highlight::learn(&plain, &[glyph(2, 10, 5, BLACK)]), None);
        // A sprite removed.
        assert_eq!(Highlight::learn(&[glyph(1, 10, 5, BLACK), glyph(2, 16, 5, BLACK)], &[glyph(1, 10, 5, BLACK)]), None);
        // Nothing changed.
        assert_eq!(Highlight::learn(&plain, &plain), None);
        // Blue text that the row also keeps blue cannot be undone.
        assert_eq!(Highlight::learn(&[glyph(1, 10, 5, BLACK), glyph(2, 16, 5, BLUE)], &[glyph(1, 10, 5, BLUE), glyph(2, 16, 5, BLUE)]), None);
    }
}
