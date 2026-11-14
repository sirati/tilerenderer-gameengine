//! Check the compiled artifact's library boundary, including precompiled inputs.
fn entries(bytes: &[u8]) -> Vec<String> {
    assert!(bytes.len() >= 20 && bytes.len() % 4 == 0);
    let words: Vec<u32> = bytes.chunks_exact(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect();
    assert_eq!(words[0], 0x0723_0203);
    let mut found = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0 && at + count <= words.len());
        if words[at] as u16 == 15 {
            assert!(count >= 4);
            let name: Vec<u8> = words[at+3..at+count].iter().flat_map(|w| w.to_le_bytes()).take_while(|b| *b != 0).collect();
            found.push(String::from_utf8(name).unwrap());
        }
        at += count;
    }
    found
}

#[test]
fn compiled_module_contains_only_its_owned_effect_entry_points() {
    for (bytes, dual) in [(viewer_spirv::SPIRV, true), (viewer_spirv::SPIRV_FALLBACK, false)] {
        let found = entries(bytes);
        for required in ["tile_vs", "tile_fs", "output_fs", "painted_fs"] {
            assert!(found.iter().any(|name| name == required), "missing {required}: {found:?}");
        }
        for forbidden in ["lens_vs", "lens_fs", "lens_copy_fs"] {
            assert!(!found.iter().any(|name| name == forbidden), "unexpected {forbidden}: {found:?}");
        }
        assert_eq!(found.iter().any(|name| name == "text_dual_fs"), dual);
        assert_eq!(found.iter().any(|name| name == "subpixel_dual_fs"), dual);
    }
}
