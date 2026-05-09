//! Unicode lookup tables for evasion-resistant normalization.
//!
//! Provides mappings for confusable characters (homoglyphs), special Unicode
//! ranges (enclosed alphanumerics, superscripts), half-width katakana, and
//! katakana voicing composition.

/// Map Cyrillic/Greek homoglyphs to their Latin lookalikes.
#[must_use]
pub(crate) fn confusable_to_latin(ch: char) -> Option<char> {
    #[rustfmt::skip]
    let mapped = match ch {
        // → a: Cyrillic а/А, Greek α/Α
        '\u{0430}' | '\u{0410}' | '\u{03B1}' | '\u{0391}' => 'a',
        // → b: Greek Β
        '\u{0392}' => 'b',
        // → c: Cyrillic с/С
        '\u{0441}' | '\u{0421}' => 'c',
        // → e: Cyrillic е/Е, Greek ε/Ε
        '\u{0435}' | '\u{0415}' | '\u{03B5}' | '\u{0395}' => 'e',
        // → h: Cyrillic һ, Greek Η
        '\u{04BB}' | '\u{0397}' => 'h',
        // → i: Cyrillic і/І, Greek ι/Ι
        '\u{0456}' | '\u{0406}' | '\u{03B9}' | '\u{0399}' => 'i',
        // → j: Cyrillic ј/Ј
        '\u{0458}' | '\u{0408}' => 'j',
        // → k: Greek Κ
        '\u{039A}' => 'k',
        // → m: Greek Μ
        '\u{039C}' => 'm',
        // → n: Greek Ν
        '\u{039D}' => 'n',
        // → o: Cyrillic о/О, Greek ο/Ο
        '\u{043E}' | '\u{041E}' | '\u{03BF}' | '\u{039F}' => 'o',
        // → p: Cyrillic р/Р, Greek ρ/Ρ
        '\u{0440}' | '\u{0420}' | '\u{03C1}' | '\u{03A1}' => 'p',
        // → s: Cyrillic ѕ/Ѕ
        '\u{0455}' | '\u{0405}' => 's',
        // → t: Greek Τ
        '\u{03A4}' => 't',
        // → u: Greek υ
        '\u{03C5}' => 'u',
        // → x: Cyrillic х/Х, Greek Χ
        '\u{0445}' | '\u{0425}' | '\u{03A7}' => 'x',
        // → y: Cyrillic у/У, Greek Υ
        '\u{0443}' | '\u{0423}' | '\u{03A5}' => 'y',
        // → z: Greek Ζ
        '\u{0396}' => 'z',
        _ => return None,
    };
    Some(mapped)
}

/// Map enclosed alphanumerics, superscript/modifier letters to ASCII.
#[must_use]
pub(crate) fn special_unicode_to_ascii(ch: char) -> Option<char> {
    let code = ch as u32;

    // Enclosed lowercase: ⓐ (U+24D0) .. ⓩ (U+24E9)
    if (0x24D0..=0x24E9).contains(&code) {
        return char::from_u32(code - 0x24D0 + u32::from(b'a'));
    }
    // Enclosed uppercase: Ⓐ (U+24B6) .. Ⓩ (U+24CF)
    if (0x24B6..=0x24CF).contains(&code) {
        return char::from_u32(code - 0x24B6 + u32::from(b'a'));
    }

    // Modifier / superscript letters
    #[rustfmt::skip]
    let mapped = match ch {
        '\u{1D43}' => 'a',
        '\u{1D47}' => 'b',
        '\u{1D48}' => 'd',
        '\u{1D49}' => 'e',
        '\u{02E0}' | '\u{1D4D}' => 'g',
        '\u{02B0}' => 'h',
        '\u{2071}' => 'i',
        '\u{02B2}' => 'j',
        '\u{1D4F}' => 'k',
        '\u{02E1}' => 'l',
        '\u{1D50}' => 'm',
        '\u{207F}' => 'n',
        '\u{1D52}' => 'o',
        '\u{1D56}' => 'p',
        '\u{02B3}' => 'r',
        '\u{02E2}' => 's',
        '\u{1D57}' => 't',
        '\u{1D58}' => 'u',
        '\u{1D5B}' => 'v',
        '\u{02B7}' => 'w',
        '\u{02B8}' => 'y',
        _ => return None,
    };
    Some(mapped)
}

/// Half-width katakana (U+FF65..U+FF9F) → full-width katakana.
///
/// Returns `None` for characters outside this range.
#[must_use]
pub(crate) fn halfwidth_kana_to_fullwidth(ch: char) -> Option<char> {
    #[rustfmt::skip]
    static HW_TO_FW: &[char] = &[
        // FF65: ・ → ・
        '\u{30FB}',
        // FF66: ヲ
        '\u{30F2}',
        // FF67..FF6B: ァィゥェォ
        '\u{30A1}', '\u{30A3}', '\u{30A5}', '\u{30A7}', '\u{30A9}',
        // FF6C..FF6E: ャュョ
        '\u{30E3}', '\u{30E5}', '\u{30E7}',
        // FF6F: ッ
        '\u{30C3}',
        // FF70: ー
        '\u{30FC}',
        // FF71..FF75: アイウエオ
        '\u{30A2}', '\u{30A4}', '\u{30A6}', '\u{30A8}', '\u{30AA}',
        // FF76..FF7A: カキクケコ
        '\u{30AB}', '\u{30AD}', '\u{30AF}', '\u{30B1}', '\u{30B3}',
        // FF7B..FF7F: サシスセソ
        '\u{30B5}', '\u{30B7}', '\u{30B9}', '\u{30BB}', '\u{30BD}',
        // FF80..FF84: タチツテト
        '\u{30BF}', '\u{30C1}', '\u{30C4}', '\u{30C6}', '\u{30C8}',
        // FF85..FF89: ナニヌネノ
        '\u{30CA}', '\u{30CB}', '\u{30CC}', '\u{30CD}', '\u{30CE}',
        // FF8A..FF8E: ハヒフヘホ
        '\u{30CF}', '\u{30D2}', '\u{30D5}', '\u{30D8}', '\u{30DB}',
        // FF8F..FF93: マミムメモ
        '\u{30DE}', '\u{30DF}', '\u{30E0}', '\u{30E1}', '\u{30E2}',
        // FF94..FF96: ヤユヨ
        '\u{30E4}', '\u{30E6}', '\u{30E8}',
        // FF97..FF9B: ラリルレロ
        '\u{30E9}', '\u{30EA}', '\u{30EB}', '\u{30EC}', '\u{30ED}',
        // FF9C: ワ
        '\u{30EF}',
        // FF9D: ン
        '\u{30F3}',
        // FF9E: ゙ (dakuten combining)
        '\u{3099}',
        // FF9F: ゚ (handakuten combining)
        '\u{309A}',
    ];

    let code = ch as u32;
    if (0xFF65..=0xFF9F).contains(&code) {
        let idx = (code - 0xFF65) as usize;
        HW_TO_FW.get(idx).copied()
    } else {
        None
    }
}

/// Compose a full-width katakana base + voicing mark into a single character.
///
/// - Dakuten (U+3099): voiced — e.g. カ+゙ → ガ
/// - Handakuten (U+309A): semi-voiced — e.g. ハ+゚ → パ
#[must_use]
pub(crate) fn compose_voicing(base: char, mark: char) -> Option<char> {
    let base_code = base as u32;

    match mark {
        '\u{3099}' => {
            // Dakuten: Ka-row, Sa-row, Ta-row, Ha-row get +1
            // U+30AB(カ)..U+30C9(ド) and U+30CF(ハ)..U+30DD(ポ)
            match base_code {
                // カ(30AB),キ(30AD),ク(30AF),ケ(30B1),コ(30B3)
                // サ(30B5),シ(30B7),ス(30B9),セ(30BB),ソ(30BD)
                // タ(30BF),チ(30C1),ツ(30C4),テ(30C6),ト(30C8)
                0x30AB | 0x30AD | 0x30AF | 0x30B1 | 0x30B3 | 0x30B5 | 0x30B7 | 0x30B9 | 0x30BB
                | 0x30BD | 0x30BF | 0x30C1 | 0x30C4 | 0x30C6 | 0x30C8 => {
                    char::from_u32(base_code + 1)
                }
                // ハ(30CF),ヒ(30D2),フ(30D5),ヘ(30D8),ホ(30DB)
                0x30CF | 0x30D2 | 0x30D5 | 0x30D8 | 0x30DB => char::from_u32(base_code + 1),
                // ウ(30A6) → ヴ(30F4)
                0x30A6 => Some('\u{30F4}'),
                _ => None,
            }
        }
        '\u{309A}' => {
            // Handakuten: Ha-row only → +2
            match base_code {
                0x30CF | 0x30D2 | 0x30D5 | 0x30D8 | 0x30DB => char::from_u32(base_code + 2),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Mathematical Alphanumeric Symbols range table: (`block_start`, `block_end`, `base_char`).
/// Each block maps 26 characters to a-z.
#[rustfmt::skip]
static MATH_ALPHA_RANGES: &[(u32, u32, u8)] = &[
    (0x1D400, 0x1D419, b'a'), // Bold uppercase A-Z
    (0x1D41A, 0x1D433, b'a'), // Bold lowercase a-z
    (0x1D434, 0x1D44D, b'a'), // Italic uppercase A-Z
    (0x1D44E, 0x1D467, b'a'), // Italic lowercase a-z
    (0x1D468, 0x1D481, b'a'), // Bold Italic uppercase A-Z
    (0x1D482, 0x1D49B, b'a'), // Bold Italic lowercase a-z
    (0x1D49C, 0x1D4B5, b'a'), // Script uppercase A-Z
    (0x1D4B6, 0x1D4CF, b'a'), // Script lowercase a-z
    (0x1D4D0, 0x1D4E9, b'a'), // Bold Script uppercase A-Z
    (0x1D4EA, 0x1D503, b'a'), // Bold Script lowercase a-z
    (0x1D504, 0x1D51D, b'a'), // Fraktur uppercase A-Z
    (0x1D51E, 0x1D537, b'a'), // Fraktur lowercase a-z
    (0x1D538, 0x1D551, b'a'), // Double-struck uppercase A-Z
    (0x1D552, 0x1D56B, b'a'), // Double-struck lowercase a-z
    (0x1D56C, 0x1D585, b'a'), // Bold Fraktur uppercase A-Z
    (0x1D586, 0x1D59F, b'a'), // Bold Fraktur lowercase a-z
    (0x1D5A0, 0x1D5B9, b'a'), // Sans-serif uppercase A-Z
    (0x1D5BA, 0x1D5D3, b'a'), // Sans-serif lowercase a-z
    (0x1D5D4, 0x1D5ED, b'a'), // Sans-serif Bold uppercase A-Z
    (0x1D5EE, 0x1D607, b'a'), // Sans-serif Bold lowercase a-z
    (0x1D608, 0x1D621, b'a'), // Sans-serif Italic uppercase A-Z
    (0x1D622, 0x1D63B, b'a'), // Sans-serif Italic lowercase a-z
    (0x1D63C, 0x1D655, b'a'), // Sans-serif Bold Italic uppercase A-Z
    (0x1D656, 0x1D66F, b'a'), // Sans-serif Bold Italic lowercase a-z
    (0x1D670, 0x1D689, b'a'), // Monospace uppercase A-Z
    (0x1D68A, 0x1D6A3, b'a'), // Monospace lowercase a-z
];

/// Map Mathematical Alphanumeric Symbols (U+1D400..U+1D7FF) to ASCII.
///
/// Covers bold, italic, bold-italic, script, bold-script, fraktur,
/// bold-fraktur, double-struck, sans-serif, sans-serif bold,
/// sans-serif italic, sans-serif bold-italic, and monospace variants
/// of Latin A-Z/a-z.
#[must_use]
pub(crate) fn math_alphanumeric_to_ascii(ch: char) -> Option<char> {
    let code = ch as u32;

    if !(0x1D400..=0x1D7FF).contains(&code) {
        return None;
    }

    for &(start, end, base) in MATH_ALPHA_RANGES {
        if code >= start && code <= end {
            let offset = (code - start) % 26;
            return char::from_u32(u32::from(base) + offset);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_a_maps_to_latin() {
        assert_eq!(confusable_to_latin('\u{0430}'), Some('a'));
        assert_eq!(confusable_to_latin('\u{0440}'), Some('p'));
    }

    #[test]
    fn greek_omicron_maps_to_latin() {
        assert_eq!(confusable_to_latin('\u{03BF}'), Some('o'));
    }

    #[test]
    fn enclosed_lowercase() {
        // ⓐ = U+24D0 → 'a'
        assert_eq!(special_unicode_to_ascii('\u{24D0}'), Some('a'));
        // ⓩ = U+24E9 → 'z'
        assert_eq!(special_unicode_to_ascii('\u{24E9}'), Some('z'));
    }

    #[test]
    fn superscript_letters() {
        assert_eq!(special_unicode_to_ascii('\u{02E2}'), Some('s'));
        assert_eq!(special_unicode_to_ascii('\u{02B8}'), Some('y'));
        assert_eq!(special_unicode_to_ascii('\u{1D57}'), Some('t'));
    }

    #[test]
    fn halfwidth_kana_basic() {
        // ｼ (FF7C) → シ (30B7)
        assert_eq!(halfwidth_kana_to_fullwidth('\u{FF7C}'), Some('\u{30B7}'));
        // ﾝ (FF9D) → ン (30F3)
        assert_eq!(halfwidth_kana_to_fullwidth('\u{FF9D}'), Some('\u{30F3}'));
    }

    #[test]
    fn voicing_composition() {
        // カ + dakuten → ガ
        assert_eq!(compose_voicing('\u{30AB}', '\u{3099}'), Some('\u{30AC}'));
        // ハ + handakuten → パ
        assert_eq!(compose_voicing('\u{30CF}', '\u{309A}'), Some('\u{30D1}'));
    }

    #[test]
    fn non_confusable_returns_none() {
        assert_eq!(confusable_to_latin('a'), None);
        assert_eq!(confusable_to_latin('z'), None);
    }

    #[test]
    fn math_bold_uppercase_a() {
        // 𝐀 (U+1D400) → 'a'
        assert_eq!(math_alphanumeric_to_ascii('\u{1D400}'), Some('a'));
    }

    #[test]
    fn math_bold_lowercase_z() {
        // 𝐳 (U+1D433) → 'z'
        assert_eq!(math_alphanumeric_to_ascii('\u{1D433}'), Some('z'));
    }

    #[test]
    fn math_italic_lowercase() {
        // 𝑠 (U+1D460) → 's'  (offset 18 in italic lowercase block 1D44E..1D467)
        assert_eq!(math_alphanumeric_to_ascii('\u{1D460}'), Some('s'));
    }

    #[test]
    fn math_sans_serif_bold() {
        // 𝗔 (U+1D5D4) → 'a'
        assert_eq!(math_alphanumeric_to_ascii('\u{1D5D4}'), Some('a'));
    }

    #[test]
    fn math_monospace_lowercase_a() {
        // 𝚊 (U+1D68A) → 'a'
        assert_eq!(math_alphanumeric_to_ascii('\u{1D68A}'), Some('a'));
    }

    #[test]
    fn math_outside_range_returns_none() {
        assert_eq!(math_alphanumeric_to_ascii('a'), None);
        assert_eq!(math_alphanumeric_to_ascii('Z'), None);
    }
}
