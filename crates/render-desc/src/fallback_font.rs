//! Fixed vector outlines for viewer status and confirmation text.
//! Reuses the FPS character rows; no font file or CPU text rasterizer.
use super::{GLYPHS, GLYPH_ROWS};

const EXTRA: [(char,[u8;7]);12]=[
    ('e',[0,0,14,17,31,16,15]),
    ('o',[0,0,14,17,17,17,14]),
    ('c',[0,0,15,16,16,16,15]),
    ('h',[16,16,22,25,17,17,17]),
    ('v',[0,0,17,17,17,10,4]),
    ('w',[0,0,17,17,21,21,10]),
    ('y',[0,0,17,17,15,1,14]),
    ('?',[14,17,1,2,4,0,4]),
    ('D',[30,17,17,17,17,17,30]),
    ('R',[30,17,17,30,20,18,17]),
    ('K',[17,18,20,24,20,18,17]),
    (',',[0,0,0,0,12,4,8]),
];

pub(super) fn face()->tile_render::gpu_text::Face {
    let glyphs:Vec<_>=GLYPHS.chars().zip(GLYPH_ROWS).chain(EXTRA).map(|(ch,rows)| {
        let mut curves=Vec::new();
        for (row,bits) in rows.into_iter().enumerate() {
            for column in 0..5 {
                if bits & (1<<(4-column)) == 0 {continue;}
                let (x,y)=(column as f64,(6-row) as f64);
                let points=[(x,y),(x,y+1.0),(x+1.0,y+1.0),(x+1.0,y)];
                for i in 0..4 {
                    let (a,b)=(points[i],points[(i+1)%4]);
                    curves.push([a,((a.0+b.0)/2.0,(a.1+b.1)/2.0),b]);
                }
            }
        }
        (ch,curves,6)
    }).collect();
    tile_render::gpu_text::Face::from_curves(8,&glyphs)
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtin_face_index_and_arc_survive_incremental_art_growth() {
        struct NoFiles;
        impl crate::ImageLoader for NoFiles {
            fn load(&self,_:&str,_:&toml::Table)->Result<Vec<tile_render::RgbaImage>,String> {Err("no files".into())}
        }
        let desc=crate::Description::parse("grid={layout='orthogonal',step=[1,1]}").unwrap();
        let (mut compiled,mut growing)=desc.compile_growing(&NoFiles,16,true).unwrap();
        let (index,_)=compiled.font(crate::FALLBACK_FONT).unwrap();
        let face=compiled.fonts[index as usize].1.clone();
        let growth=growing.grow(&compiled,"[fonts.delayed]\nsource='missing.ttf'\n",&NoFiles).unwrap();
        assert!(growth.is_empty(),"a missing font must not regenerate the fallback");
        compiled.apply(&growth);
        assert_eq!(compiled.font(crate::FALLBACK_FONT).unwrap().0,index);
        assert!(std::sync::Arc::ptr_eq(&compiled.fonts[index as usize].1,&face));
    }

    #[test]
    fn builtin_outline_face_covers_warnings_and_settings_confirmation() {
        let face=super::face();
        for text in ["Game appears busy", "Game appears stuck", "Do you want to keep this?", "Reverting in 0123456789 seconds", "Keep", "Revert", "During the last second, 1234567890 rendering errors happened."] {
            for ch in text.chars() {
                assert!(face.glyph(ch).is_some(),"missing {ch:?}");
                assert_eq!(face.advance(ch),Some(6));
            }
        }
    }
}
