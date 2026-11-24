//! Suggesting a filename, and naming a worksheet Excel will accept.
//!
//! # Two different character sets, two different functions
//!
//! A filename and a worksheet name are refused for different reasons by different software,
//! and collapsing them into one sanitiser would get both wrong:
//!
//! | | Filename | Worksheet name |
//! | --- | --- | --- |
//! | Refused characters | `< > : " / \ | ? *` and the control range | `* ? : [ ] \ /` |
//! | Length | bounded so a path stays usable | **31 UTF-16 code units**, refused above |
//! | Reserved words | `CON`, `PRN`, `AUX`, `NUL`, `COM1`…, `LPT1`… | none |
//! | Edge characters | no trailing dot or space | no leading or trailing apostrophe |
//! | Uniqueness | the filesystem's problem, and not ours (§8.2) | **Excel refuses a reuse** |
//!
//! The worksheet rules are **VERIFIED** against the vendored writer:
//! `rust_xlsxwriter-0.102.0/src/utility.rs:1112-1138` refuses a name that is empty, longer
//! than 31 UTF-16 code units, contains any of `* ? : [ ] \ /`, or starts or ends with an
//! apostrophe; `Workbook` additionally refuses a reused name with `XlsxError::SheetnameReused`.
//!
//! # Why there is so little filename work here
//!
//! Because bulk output is **one document** — one workbook, one CSV, one printed document — the
//! user names that one file in a native dialog, and Evara derives no filenames (§8.2, §10.2).
//! That removes at a stroke the Windows reserved-name class, the duplicate-name class and path
//! traversal through an entity name. What remains is the *suggested* name in the dialogue's
//! `defaultPath`, which a user may accept, so it needs the one guard below.
//!
//! These functions prepare **suggestions**. They write no file, open no directory and know no
//! path: judging a destination and claiming it is `evara-project`'s and the command layer's
//! (§10.1), and this crate has no path to judge.

use std::collections::BTreeSet;

use super::OutputError;

/// How many UTF-16 code units a worksheet name may be.
///
/// Sixteen bits per unit is the unit Excel counts in, which is why truncating by bytes or by
/// characters is wrong: a CJK name is one code unit per character where an emoji is two, and a
/// byte count is three per CJK character. VERIFIED against `validate_sheetname`.
pub const SHEET_NAME_LIMIT: usize = 31;

/// How long a suggested file stem may be.
///
/// Not a filesystem limit — a bound, so that a timetable somebody called a sentence does not
/// produce a path that fails at some later join. Short enough to leave room for an extension,
/// a deduplicating suffix and a folder a user chose.
pub const FILE_STEM_LIMIT: usize = 120;

/// What a stem falls back to when nothing of the original survives.
pub const FILE_STEM_FALLBACK: &str = "timetable";

/// The characters Windows refuses in a filename.
const FORBIDDEN_IN_FILENAME: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// The characters Excel refuses in a worksheet name.
const FORBIDDEN_IN_SHEET_NAME: [char; 7] = ['*', '?', ':', '[', ']', '\\', '/'];

/// The device names Windows reserves, whatever extension follows them.
///
/// Case-insensitive, and checked against the stem as well as the whole string: `CON.csv` is
/// the console device, not a file called `CON` with a `.csv` extension.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A filename stem a user can accept without the dialogue refusing it.
///
/// No extension: the caller appends one, because the extension is the format's and this
/// function does not know the format.
///
/// The steps, in order, each of which can empty the string and is followed by the fallback:
///
/// 1. every character in `< > : " / \ | ? *`, and every control character, becomes a space;
/// 2. runs of whitespace collapse to one space, and the ends are trimmed;
/// 3. trailing dots and spaces are trimmed, because Windows silently drops them and a name
///    that silently changes is a name the user did not choose;
/// 4. the result is bounded to [`FILE_STEM_LIMIT`] **characters**, on a character boundary;
/// 5. a reserved device name — the whole string, or the part before the first dot, compared
///    case-insensitively — becomes [`FILE_STEM_FALLBACK`];
/// 6. an empty result becomes [`FILE_STEM_FALLBACK`].
///
/// # Examples
///
/// A `text` block rather than a compiled doctest, for the reason `guard::defuse_formula`'s
/// gives. Pinned as assertions in `the_documented_stem_examples_hold`.
///
/// ```text
/// safe_file_stem("Spring 2027 — Draft 3") == "Spring 2027 — Draft 3"
/// safe_file_stem("A/B: timetable?")       == "A B timetable"
/// safe_file_stem("CON")                   == "timetable"
/// safe_file_stem("con.csv")               == "timetable"
/// safe_file_stem("Reconnaissance")        == "Reconnaissance"
/// ```
#[must_use]
pub fn safe_file_stem(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| {
            if FORBIDDEN_IN_FILENAME.contains(&c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();

    let collapsed = collapse_whitespace(&replaced);
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let bounded = truncate_chars(trimmed, FILE_STEM_LIMIT);
    // Truncation can expose a trailing dot or space that was interior before it.
    let bounded = bounded
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();

    if bounded.is_empty() || is_reserved_device_name(&bounded) {
        return FILE_STEM_FALLBACK.to_owned();
    }
    bounded
}

/// Whether a stem names a Windows device, whatever follows it.
///
/// The whole string and the part before the first dot are both checked, which is what makes
/// `CON.csv` caught and `Reconnaissance` and `TERMCON` allowed: a reserved word as a prefix of
/// a longer word, or as a suffix, is an ordinary name.
#[must_use]
pub fn is_reserved_device_name(stem: &str) -> bool {
    let candidates = [stem, stem.split('.').next().unwrap_or(stem)];
    candidates.iter().any(|candidate| {
        let candidate = candidate.trim();
        RESERVED_DEVICE_NAMES
            .iter()
            .any(|reserved| candidate.eq_ignore_ascii_case(reserved))
    })
}

/// A worksheet name `rust_xlsxwriter` will accept, before deduplication.
///
/// `fallback` is used when nothing of `raw` survives — a name of only forbidden characters is
/// ordinary enough in a school's data that failing the export for it would be wrong. Pass
/// something that identifies the entity, e.g. `"Teacher 12"`; it is sanitised and truncated
/// the same way, and a `fallback` that is itself empty yields `"Sheet"`.
///
/// Rules 1 to 5 of §6.2, in order. Deduplication is [`dedupe_sheet_names`], because it is a
/// property of the *set* and not of one name.
///
/// # Examples
///
/// Pinned as assertions in `the_documented_sheet_name_examples_hold`.
///
/// ```text
/// safe_sheet_name("9 Mathematics B", "Section 1") == "9 Mathematics B"
/// safe_sheet_name("Y9/Maths[B]", "Section 1")     == "Y9 Maths B"
/// safe_sheet_name("'quoted'", "Section 1")        == "quoted"
/// safe_sheet_name("///", "Section 1")             == "Section 1"
/// ```
#[must_use]
pub fn safe_sheet_name(raw: &str, fallback: &str) -> String {
    let cleaned = clean_sheet_name(raw);
    if !cleaned.is_empty() {
        return cleaned;
    }
    let cleaned_fallback = clean_sheet_name(fallback);
    if cleaned_fallback.is_empty() {
        "Sheet".to_owned()
    } else {
        cleaned_fallback
    }
}

/// Rules 1 to 4: forbidden characters out, apostrophes off the ends, bounded to the limit.
fn clean_sheet_name(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| {
            if FORBIDDEN_IN_SHEET_NAME.contains(&c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = collapse_whitespace(&replaced);
    // Rule 3. An apostrophe is only refused at the ends, so an interior one survives.
    let unquoted = collapsed.trim_matches('\'').trim();
    // Rule 4, counted in UTF-16 and cut on a character boundary.
    let bounded = truncate_utf16(unquoted, SHEET_NAME_LIMIT);
    bounded.trim_matches('\'').trim().to_owned()
}

/// A set of worksheet names, unique in Excel's eyes.
///
/// Rules 6 and 7 of §6.2. Excel compares worksheet names **case-insensitively**, so `Okafor`
/// and `OKAFOR` collide and are deduplicated; two teachers genuinely called the same thing is
/// ordinary in a large school, and `SheetnameReused` would fail the whole export for it.
///
/// A collision takes ` (2)`, ` (3)`, … and the base is re-truncated so the suffix fits inside
/// [`SHEET_NAME_LIMIT`]. The input order is preserved, so a sheet's position does not depend on
/// its name.
///
/// # Errors
///
/// [`OutputError::SheetNames`] when a name cannot be made unique at all — which needs a
/// thousand collisions on one name, and is reported naming the clash rather than written as a
/// workbook with an arbitrary name in it.
pub fn dedupe_sheet_names(names: &[String]) -> Result<Vec<String>, OutputError> {
    /// How many suffixes to try before giving up.
    const ATTEMPTS: usize = 1_000;

    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let key = name.to_lowercase();
        if taken.insert(key) {
            out.push(name.clone());
            continue;
        }
        let mut resolved = None;
        for n in 2..=ATTEMPTS {
            let suffix = format!(" ({n})");
            let room = SHEET_NAME_LIMIT.saturating_sub(utf16_len(&suffix));
            let candidate = format!("{}{suffix}", truncate_utf16(name, room).trim_end());
            if taken.insert(candidate.to_lowercase()) {
                resolved = Some(candidate);
                break;
            }
        }
        match resolved {
            Some(candidate) => out.push(candidate),
            None => {
                return Err(OutputError::SheetNames {
                    detail: format!(
                        "\"{name}\" collides with more than {ATTEMPTS} worksheets already named \
                         the same thing"
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// Whether a name is one `rust_xlsxwriter` would accept on its own.
///
/// The writer's own four rules, asked here so a test can prove the output of
/// [`safe_sheet_name`] satisfies them without a workbook.
#[must_use]
pub fn is_acceptable_sheet_name(name: &str) -> bool {
    !name.is_empty()
        && utf16_len(name) <= SHEET_NAME_LIMIT
        && !name.chars().any(|c| FORBIDDEN_IN_SHEET_NAME.contains(&c))
        && !name.starts_with('\'')
        && !name.ends_with('\'')
}

/// Runs of whitespace collapsed to one space, ends trimmed.
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many UTF-16 code units a string needs.
#[must_use]
pub fn utf16_len(value: &str) -> usize {
    value.chars().map(char::len_utf16).sum()
}

/// The longest prefix that fits in `limit` UTF-16 code units, cut on a character boundary.
///
/// Never splits a character, which is the whole point: a byte-wise cut of a CJK name is
/// invalid UTF-8, and a code-unit-wise cut of an emoji is half a surrogate pair.
fn truncate_utf16(value: &str, limit: usize) -> &str {
    let mut used = 0_usize;
    for (at, c) in value.char_indices() {
        let width = c.len_utf16();
        if used + width > limit {
            return &value[..at];
        }
        used += width;
    }
    value
}

/// The longest prefix of `limit` characters, cut on a character boundary.
fn truncate_chars(value: &str, limit: usize) -> &str {
    match value.char_indices().nth(limit) {
        Some((at, _)) => &value[..at],
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------ T-17, file stems

    #[test]
    fn the_documented_stem_examples_hold() {
        assert_eq!(
            safe_file_stem("Spring 2027 — Draft 3"),
            "Spring 2027 — Draft 3"
        );
        assert_eq!(safe_file_stem("A/B: timetable?"), "A B timetable");
        assert_eq!(safe_file_stem("CON"), "timetable");
        assert_eq!(safe_file_stem("con.csv"), "timetable");
        assert_eq!(safe_file_stem("Reconnaissance"), "Reconnaissance");
    }

    #[test]
    fn the_documented_sheet_name_examples_hold() {
        assert_eq!(
            safe_sheet_name("9 Mathematics B", "Section 1"),
            "9 Mathematics B"
        );
        assert_eq!(safe_sheet_name("Y9/Maths[B]", "Section 1"), "Y9 Maths B");
        assert_eq!(safe_sheet_name("'quoted'", "Section 1"), "quoted");
        assert_eq!(safe_sheet_name("///", "Section 1"), "Section 1");
    }

    #[test]
    fn an_ordinary_name_survives_whole() {
        for name in [
            "Spring 2027 Draft 3",
            "A. Okafor",
            "9 Mathematics B",
            "Northgate — Term 1",
            "さくら先生",
            "مدرسة النور",
        ] {
            assert_eq!(safe_file_stem(name), name, "{name:?}");
        }
    }

    #[test]
    fn every_forbidden_filename_character_becomes_a_space() {
        assert_eq!(
            safe_file_stem(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a b c d e f g h i j"
        );
    }

    #[test]
    fn control_characters_become_spaces_and_collapse() {
        assert_eq!(safe_file_stem("Draft\t3\nfinal\u{0}"), "Draft 3 final");
    }

    #[test]
    fn a_trailing_dot_or_space_is_trimmed() {
        // Windows drops these silently, and a name that silently changes is not the name the
        // user chose.
        assert_eq!(safe_file_stem("Draft 3."), "Draft 3");
        assert_eq!(safe_file_stem("Draft 3   "), "Draft 3");
        assert_eq!(safe_file_stem("Draft 3 ..."), "Draft 3");
        assert_eq!(safe_file_stem("  .Draft 3."), "Draft 3");
    }

    #[test]
    fn an_interior_dot_survives() {
        assert_eq!(safe_file_stem("Draft 3.1 final"), "Draft 3.1 final");
    }

    #[test]
    fn every_windows_device_name_falls_back() {
        for reserved in RESERVED_DEVICE_NAMES {
            assert_eq!(safe_file_stem(reserved), FILE_STEM_FALLBACK, "{reserved}");
            assert_eq!(
                safe_file_stem(&reserved.to_lowercase()),
                FILE_STEM_FALLBACK,
                "{reserved} lower-cased"
            );
            // The stem is what Windows looks at, so an extension does not rescue it.
            assert_eq!(
                safe_file_stem(&format!("{reserved}.csv")),
                FILE_STEM_FALLBACK,
                "{reserved}.csv"
            );
        }
    }

    #[test]
    fn a_reserved_word_inside_a_longer_name_is_allowed() {
        // The case a sloppier check breaks: these are ordinary words and ordinary names.
        for allowed in [
            "Reconnaissance",
            "CONCERT",
            "Falcon",
            "TERMCON",
            "Auxiliary",
            "NULL hypothesis",
            "COM10",
            "LPT10",
            "Nulla",
        ] {
            assert_eq!(safe_file_stem(allowed), allowed, "{allowed:?}");
        }
    }

    #[test]
    fn an_entirely_invalid_name_falls_back() {
        for nothing in ["", "   ", "///", r#"<>:"|?*"#, "...", "\u{0}\u{1}\u{2}"] {
            assert_eq!(safe_file_stem(nothing), FILE_STEM_FALLBACK, "{nothing:?}");
        }
    }

    #[test]
    fn a_long_name_is_bounded_on_a_character_boundary() {
        let long = "x".repeat(400);
        let stem = safe_file_stem(&long);
        assert_eq!(stem.chars().count(), FILE_STEM_LIMIT);

        // Four hundred CJK characters: bounded by characters, and still valid UTF-8.
        let cjk = "中".repeat(400);
        let stem = safe_file_stem(&cjk);
        assert_eq!(stem.chars().count(), FILE_STEM_LIMIT);
        assert!(stem.chars().all(|c| c == '中'));

        // An emoji is one character and several bytes; neither is split.
        let emoji = "\u{1f3eb}".repeat(400);
        let stem = safe_file_stem(&emoji);
        assert_eq!(stem.chars().count(), FILE_STEM_LIMIT);
    }

    #[test]
    fn a_stem_never_carries_a_character_a_dialogue_would_refuse() {
        for raw in [
            r#"A/B\C:D"E<F>G|H?I*J"#,
            "CON",
            "   ",
            "Draft 3...",
            &"y".repeat(500),
        ] {
            let stem = safe_file_stem(raw);
            assert!(!stem.is_empty(), "{raw:?}");
            assert!(
                !stem.chars().any(|c| FORBIDDEN_IN_FILENAME.contains(&c)),
                "{stem:?}"
            );
            assert!(!stem.chars().any(char::is_control), "{stem:?}");
            assert!(!stem.ends_with('.') && !stem.ends_with(' '), "{stem:?}");
            assert!(!is_reserved_device_name(&stem), "{stem:?}");
            assert!(stem.chars().count() <= FILE_STEM_LIMIT, "{stem:?}");
        }
    }

    // ------------------------------------------------------------------ T-16, sheet names

    #[test]
    fn a_short_name_is_a_worksheet_name_already() {
        assert_eq!(safe_sheet_name("A. Okafor", "Teacher 1"), "A. Okafor");
        assert!(is_acceptable_sheet_name("A. Okafor"));
    }

    #[test]
    fn every_forbidden_worksheet_character_is_replaced() {
        for forbidden in FORBIDDEN_IN_SHEET_NAME {
            let raw = format!("Maths{forbidden}B");
            let name = safe_sheet_name(&raw, "Section 1");
            assert!(
                !name.chars().any(|c| c == forbidden),
                "{forbidden:?} survived in {name:?}"
            );
            assert!(is_acceptable_sheet_name(&name), "{name:?}");
        }
        assert_eq!(safe_sheet_name("Y9/Maths[B]", "Section 1"), "Y9 Maths B");
    }

    #[test]
    fn a_leading_or_trailing_apostrophe_is_trimmed_and_an_interior_one_is_not() {
        assert_eq!(safe_sheet_name("'Okafor", "Teacher 1"), "Okafor");
        assert_eq!(safe_sheet_name("Okafor'", "Teacher 1"), "Okafor");
        assert_eq!(safe_sheet_name("''Okafor''", "Teacher 1"), "Okafor");
        assert_eq!(safe_sheet_name("O'Brien", "Teacher 1"), "O'Brien");
        assert!(is_acceptable_sheet_name("O'Brien"));
    }

    #[test]
    fn a_name_of_only_forbidden_characters_uses_the_fallback() {
        assert_eq!(safe_sheet_name("///", "Teacher 12"), "Teacher 12");
        assert_eq!(safe_sheet_name("'''", "Teacher 12"), "Teacher 12");
        assert_eq!(safe_sheet_name("", "Teacher 12"), "Teacher 12");
        assert_eq!(safe_sheet_name("   ", "Teacher 12"), "Teacher 12");
        // And a fallback that is itself empty still yields something Excel accepts.
        assert_eq!(safe_sheet_name("///", ""), "Sheet");
        assert!(is_acceptable_sheet_name(&safe_sheet_name("///", "")));
    }

    #[test]
    fn a_long_name_is_truncated_to_thirty_one_utf16_units() {
        let long = "Mathematics for the Upper School, Section B";
        let name = safe_sheet_name(long, "Section 1");
        assert_eq!(utf16_len(&name), SHEET_NAME_LIMIT);
        assert!(long.starts_with(name.as_str()));
        assert!(is_acceptable_sheet_name(&name));
    }

    #[test]
    fn a_cjk_name_truncates_by_code_unit_and_never_mid_codepoint() {
        // One UTF-16 unit per character, three bytes per character: a byte-wise cut would be
        // invalid UTF-8 and would panic on the slice.
        let cjk = "中".repeat(40);
        let name = safe_sheet_name(&cjk, "Section 1");
        assert_eq!(utf16_len(&name), SHEET_NAME_LIMIT);
        assert_eq!(name.chars().count(), SHEET_NAME_LIMIT);
        assert!(name.chars().all(|c| c == '中'));
    }

    #[test]
    fn an_emoji_name_truncates_without_splitting_a_surrogate_pair() {
        // Two UTF-16 units per emoji, so fifteen fit in thirty-one and the thirty-first unit
        // is deliberately left unused rather than filled with half a character.
        let emoji = "\u{1f3eb}".repeat(40);
        let name = safe_sheet_name(&emoji, "Section 1");
        assert_eq!(name.chars().count(), 15);
        assert_eq!(utf16_len(&name), 30);
        assert!(is_acceptable_sheet_name(&name));
    }

    #[test]
    fn three_identical_names_become_one_two_and_three() {
        let names = vec![
            "Okafor".to_owned(),
            "Okafor".to_owned(),
            "Okafor".to_owned(),
        ];
        assert_eq!(
            dedupe_sheet_names(&names).expect("three collisions resolve"),
            vec!["Okafor", "Okafor (2)", "Okafor (3)"]
        );
    }

    #[test]
    fn a_collision_is_case_insensitive_because_excel_is() {
        let names = vec![
            "Okafor".to_owned(),
            "OKAFOR".to_owned(),
            "okafor".to_owned(),
        ];
        let out = dedupe_sheet_names(&names).expect("resolves");
        assert_eq!(out, vec!["Okafor", "OKAFOR (2)", "okafor (3)"]);
        let mut lowered: Vec<String> = out.iter().map(|n| n.to_lowercase()).collect();
        lowered.sort();
        lowered.dedup();
        assert_eq!(lowered.len(), 3, "Excel would refuse a reuse");
    }

    #[test]
    fn deduplication_keeps_the_input_order() {
        let names = vec![
            "B".to_owned(),
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
        ];
        assert_eq!(
            dedupe_sheet_names(&names).expect("resolves"),
            vec!["B", "A", "B (2)", "C"]
        );
    }

    #[test]
    fn a_suffix_fits_inside_the_limit_by_shortening_the_base() {
        let at_limit = "A".repeat(SHEET_NAME_LIMIT);
        let names = vec![at_limit.clone(), at_limit];
        let out = dedupe_sheet_names(&names).expect("resolves");
        for name in &out {
            assert!(utf16_len(name) <= SHEET_NAME_LIMIT, "{name:?}");
            assert!(is_acceptable_sheet_name(name), "{name:?}");
        }
        assert!(out[1].ends_with(" (2)"));
        assert_ne!(out[0], out[1]);
    }

    #[test]
    fn nothing_deduplication_produces_is_refused_by_the_writers_rules() {
        // The composite property: whatever goes in, what comes out is acceptable and unique.
        let cjk = "中".repeat(40);
        let raw = [
            "Mathematics for the Upper School, Section B",
            "Mathematics for the Upper School, Section C",
            "'quoted'",
            "///",
            cjk.as_str(),
            "Okafor",
            "OKAFOR",
        ];
        let safe: Vec<String> = raw
            .iter()
            .enumerate()
            .map(|(n, name)| safe_sheet_name(name, &format!("Sheet {n}")))
            .collect();
        let out = dedupe_sheet_names(&safe).expect("resolves");
        assert_eq!(out.len(), raw.len(), "every entity gets a sheet");
        for name in &out {
            assert!(is_acceptable_sheet_name(name), "{name:?}");
        }
        let mut keys: Vec<String> = out.iter().map(|n| n.to_lowercase()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), out.len(), "{out:?} must be unique to Excel");
    }

    #[test]
    fn two_names_that_truncate_to_the_same_thing_still_differ() {
        // The case that makes truncation and deduplication interact: both inputs shorten to
        // the same thirty-one units, so only the suffix separates them.
        let a = safe_sheet_name("Mathematics for the Upper School, Section B", "a");
        let b = safe_sheet_name("Mathematics for the Upper School, Section C", "b");
        assert_eq!(a, b, "they truncate to the same prefix");
        let out = dedupe_sheet_names(&[a, b]).expect("resolves");
        assert_ne!(out[0], out[1]);
        assert!(out.iter().all(|n| is_acceptable_sheet_name(n)));
    }
}
