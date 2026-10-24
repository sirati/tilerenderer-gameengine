//! Add SPIR-V dual-source output indices that rust-gpu cannot express yet.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

const ENTRY_POINT: u16 = 15;
const VARIABLE: u16 = 59;
const DECORATE: u16 = 71;
const LOCATION: u32 = 30;
const INDEX: u32 = 32;
const OUTPUT: u32 = 3;

/// Only the named fragment's two unique outputs are changed. Other entry
/// points remain byte-identical, including the three-pass LCD fallback.
/// Reject malformed or unexpected compiler output instead of guessing IDs.
pub fn decorate(bytes: &[u8], entry: &str) -> Result<Vec<u8>, String> {
    transform(bytes, entry, true)
}

/// The same compiler output without the optional entry point or Index usage.
/// This module validates even when the device did not enable dual-source blending.
pub fn fallback(bytes: &[u8], entry: &str) -> Result<Vec<u8>, String> {
    transform(bytes, entry, false)
}

fn transform(bytes: &[u8], entry: &str, dual: bool) -> Result<Vec<u8>, String> {
    if bytes.len() < 20 || bytes.len() % 4 != 0 {
        return Err("invalid SPIR-V byte length".into());
    }
    let mut words: Vec<u32> = bytes.chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
    if words[0] != 0x0723_0203 { return Err("invalid SPIR-V magic".into()); }
    let mut entries = Vec::new();
    let mut outputs = Vec::new();
    let mut locations = BTreeMap::new();
    let mut indices = BTreeMap::new();
    let mut annotations_end = None;
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        let op = words[at] as u16;
        if count == 0 || count > words.len() - at {
            return Err("truncated SPIR-V instruction".into());
        }
        instructions.push((at, count, op));
        let operands = &words[at + 1..at + count];
        match op {
            ENTRY_POINT => {
                if operands.len() < 3 { return Err("invalid OpEntryPoint".into()); }
                let mut name = Vec::new();
                let mut end = None;
                for (i, word) in operands[2..].iter().enumerate() {
                    for b in word.to_le_bytes() {
                        if b == 0 { end = Some(i + 3); break; }
                        name.push(b);
                    }
                    if end.is_some() { break; }
                }
                let end = end.ok_or("unterminated entry-point name")?;
                let name = String::from_utf8(name).map_err(|_| "invalid entry-point name")?;
                entries.push((operands[0], name, operands[end..].to_vec(), operands[1], at));
            }
            VARIABLE if operands.len() >= 3 && operands[2] == OUTPUT => outputs.push(operands[1]),
            DECORATE if operands.len() >= 2 && matches!(operands[1], LOCATION | INDEX) => {
                if operands.len() != 3 { return Err("invalid output decoration".into()); }
                let map = if operands[1] == LOCATION { &mut locations } else { &mut indices };
                if map.insert(operands[0], (operands[2], at + 3)).is_some() {
                    return Err("duplicate output decoration".into());
                }
            }
            19..=39 => { annotations_end.get_or_insert(at); }
            _ => {}
        }
        at += count;
    }
    let matching: Vec<_> = entries.iter().filter(|(_, name, _, _, _)| name == entry).collect();
    if matching.len() != 1 || matching[0].0 != 4 {
        return Err(format!("expected one fragment entry point {entry}"));
    }
    let interface = &matching[0].2;
    let colors: Vec<_> = interface.iter().copied()
        .filter(|id| outputs.contains(id) && locations.contains_key(id)).collect();
    if colors.len() != 2 { return Err(format!("{entry} must have two color outputs")); }
    let mut primary = None;
    let mut secondary = None;
    for id in colors {
        let location = locations[&id].0;
        let index = indices.get(&id).map(|v| v.0).unwrap_or(0);
        match (location, index) {
            (0, 0) if primary.is_none() => primary = Some(id),
            (1, 0) | (0, 1) if secondary.is_none() => secondary = Some(id),
            _ => return Err(format!("unexpected {entry} output location/index")),
        }
        if entries.iter().any(|(_, name, ids, _, _)| name != entry && ids.contains(&id)) {
            return Err(format!("{entry} output is shared with another entry point"));
        }
    }
    let primary = primary.ok_or("missing primary output")?;
    let secondary = secondary.ok_or("missing secondary output")?;
    words[locations[&secondary].1] = if dual { 0 } else { 1 };
    if !dual {
        let function = matching[0].3;
        let entry_at = matching[0].4;
        for (at, count, op) in instructions.into_iter().rev() {
            let is_mode = matches!(op, 16 | 331) && count >= 2 && words[at + 1] == function;
            let is_index = op == DECORATE && count == 4 && words[at + 2] == INDEX
                && [primary, secondary].contains(&words[at + 1]);
            if at == entry_at || is_mode || is_index { words.drain(at..at + count); }
        }
        return Ok(words.into_iter().flat_map(u32::to_le_bytes).collect());
    }
    let mut decorations = Vec::new();
    for (id, index) in [(primary, 0), (secondary, 1)] {
        if let Some(&(_, at)) = indices.get(&id) {
            words[at] = index;
        } else {
            decorations.extend([(4 << 16) | u32::from(DECORATE), id, INDEX, index]);
        }
    }
    let insert = annotations_end.ok_or("SPIR-V module has no type section")?;
    words.splice(insert..insert, decorations);
    Ok(words.into_iter().flat_map(u32::to_le_bytes).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(words: &mut Vec<u32>, op: u16, args: &[u32]) {
        words.push((((args.len() + 1) as u32) << 16) | u32::from(op));
        words.extend_from_slice(args);
    }

    fn module(shared: bool) -> Vec<u8> {
        let mut w = vec![0x0723_0203, 0x0001_0000, 0, 30, 0];
        let name = |s: &str| {
            let mut bytes = s.as_bytes().to_vec();
            bytes.push(0);
            while bytes.len() % 4 != 0 { bytes.push(0); }
            bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>()
        };
        let mut dual = vec![4, 10];
        dual.extend(name("text_dual_fs")); dual.extend([20, 21]);
        instruction(&mut w, ENTRY_POINT, &dual);
        let mut fallback = vec![4, 11]; fallback.extend(name("text_r_fs"));
        fallback.push(if shared { 20 } else { 22 });
        instruction(&mut w, ENTRY_POINT, &fallback);
        instruction(&mut w, 16, &[10, 7]);
        instruction(&mut w, 16, &[11, 7]);
        for (id, location) in [(20, 0), (21, 1), (22, 0)] {
            instruction(&mut w, DECORATE, &[id, LOCATION, location]);
        }
        instruction(&mut w, 19, &[1]);
        for id in [20, 21, 22] { instruction(&mut w, VARIABLE, &[2, id, OUTPUT]); }
        w.into_iter().flat_map(u32::to_le_bytes).collect()
    }

    #[test]
    fn decoration_is_idempotent_and_fallback_is_independent_of_input_form() {
        let raw = module(false);
        let dual = decorate(&raw, "text_dual_fs").unwrap();
        assert_eq!(decorate(&dual, "text_dual_fs").unwrap(), dual);
        assert_eq!(fallback(&raw, "text_dual_fs").unwrap(), fallback(&dual, "text_dual_fs").unwrap());
        assert!(fallback(&dual, "text_dual_fs").unwrap().len() < raw.len());
        assert_eq!(dual.len(), raw.len() + 32);
        let words: Vec<u32> = dual.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        assert!(words.windows(4).any(|w| w == [(4 << 16) | 71, 20, INDEX, 0]));
        assert!(words.windows(4).any(|w| w == [(4 << 16) | 71, 21, INDEX, 1]));
        assert!(words.windows(4).any(|w| w == [(4 << 16) | 71, 21, LOCATION, 0]));
        assert!(words.windows(4).any(|w| w == [(4 << 16) | 71, 22, LOCATION, 0]));
    }

    #[test]
    fn explicit_zero_indices_are_normalized_without_mutating_input() {
        let raw = module(false);
        let mut words: Vec<u32> = raw.chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        let at = words.iter().position(|&w| w == ((2 << 16) | 19)).unwrap();
        let mut explicit = Vec::new();
        for id in [20, 21] { instruction(&mut explicit, DECORATE, &[id, INDEX, 0]); }
        words.splice(at..at, explicit);
        let input: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        let before = input.clone();
        let dual = decorate(&input, "text_dual_fs").unwrap();
        assert_eq!(input, before, "precompiled input remains immutable");
        assert_eq!(dual, decorate(&raw, "text_dual_fs").unwrap());
        assert_eq!(dual, decorate(&dual, "text_dual_fs").unwrap());
        assert_eq!(fallback(&input, "text_dual_fs").unwrap(), fallback(&raw, "text_dual_fs").unwrap());
        assert_eq!(fallback(&dual, "text_dual_fs").unwrap(), fallback(&raw, "text_dual_fs").unwrap());
    }

    #[test]
    fn two_optional_entries_are_normalized_and_removed_independently() {
        let mut words: Vec<u32> = module(false).chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut name = b"subpixel_dual_fs\0".to_vec();
        while name.len() % 4 != 0 { name.push(0); }
        let mut args = vec![4, 12];
        args.extend(name.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())));
        args.extend([23, 24]);
        let mut entry = Vec::new(); instruction(&mut entry, ENTRY_POINT, &args);
        words.splice(5..5, entry);
        let at = words.iter().position(|&w| w == ((2 << 16) | 19)).unwrap();
        let mut decorations = Vec::new();
        for (id, location) in [(23, 0), (24, 1)] {
            instruction(&mut decorations, DECORATE, &[id, LOCATION, location]);
        }
        words.splice(at..at, decorations);
        for id in [23, 24] { instruction(&mut words, VARIABLE, &[2, id, OUTPUT]); }
        let raw: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut dual = raw.clone();
        for entry in ["text_dual_fs", "subpixel_dual_fs"] { dual = decorate(&dual, entry).unwrap(); }
        let expected = dual.clone();
        let mut repeated = dual.clone();
        let mut raw_fallback = raw;
        let mut dual_fallback = dual;
        for entry in ["text_dual_fs", "subpixel_dual_fs"] {
            repeated = decorate(&repeated, entry).unwrap();
            raw_fallback = fallback(&raw_fallback, entry).unwrap();
            dual_fallback = fallback(&dual_fallback, entry).unwrap();
        }
        assert_eq!(raw_fallback, dual_fallback);
        assert!(decorate(&raw_fallback, "text_dual_fs").is_err());
        assert!(decorate(&raw_fallback, "subpixel_dual_fs").is_err());
        assert_eq!(repeated, expected);
    }

    #[test]
    fn shared_outputs_and_missing_entry_points_are_rejected() {
        assert!(decorate(&module(true), "text_dual_fs").unwrap_err().contains("shared"));
        assert!(decorate(&module(false), "not_present").is_err());
    }

    #[test]
    fn malformed_instruction_boundaries_are_rejected_without_panicking() {
        let raw = module(false);
        for length in 0..raw.len() {
            // Complete prefixes may omit later sections, but must never panic.
            let _ = decorate(&raw[..length], "text_dual_fs");
        }
        let mut zero = raw.clone(); zero[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(decorate(&zero, "text_dual_fs").is_err());
    }
}
