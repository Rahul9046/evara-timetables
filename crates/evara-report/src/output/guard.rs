//! Keeping a school's spreadsheet from running somebody else's text as a formula.
//!
//! # The exposure is real, not theoretical
//!
//! A teacher's display name, a room's name, a section's name and a timetable's name are all
//! user-controlled text. They reach a CSV that a school opens by double-clicking it in Excel,
//! and a cell whose text begins with `=` is a formula to Excel, Calc and Numbers alike. The
//! consequences run from a `#NAME?` where a name should be to a `=HYPERLINK` or a `=WEBSERVICE`
//! that a reader is invited to trust, and for a local-first application whose whole promise is
//! that nothing leaves the machine, writing a file that invites a network call is the wrong
//! kind of surprise.
//!
//! # One implementation, two renderers
//!
//! [`defuse_formula`] is called by the CSV renderer **and** by the workbook renderer. XLSX is
//! not exempt: `rust_xlsxwriter`'s `write_string` does not evaluate anything, but a cell whose
//! stored text begins with `=` is still read as a formula by the reader that opens it, and a
//! school's copy of Excel is the reader. A second copy of this function in either renderer is
//! the duplication `CLAUDE.md` §2 names.
//!
//! # Escaping and defusing are different jobs
//!
//! Quoting a comma, a double quote or a newline is **`csv`'s** job and this function must not
//! do it: a value quoted twice is a value with quotes in it. This function changes the
//! *meaning* a spreadsheet gives a value; `csv` changes how the value survives a line-based
//! file. Keeping them apart is what makes each testable on its own.
//!
//! # Nothing benign is altered
//!
//! `defuse_formula` returns [`Cow::Borrowed`] for every value it leaves alone, so a document
//! of ordinary names allocates nothing here. A leading space, a leading digit, a name
//! containing an `=` anywhere but the first character, and any non-ASCII text are all
//! untouched — a function that prefixed an apostrophe to `José` or to `Block B - Room 2`
//! would be corrupting data to look careful.

use std::borrow::Cow;

/// The first characters a spreadsheet reads as the start of a formula.
///
/// `=` and `@` begin a formula or a function reference. `+` and `-` begin one in Excel's
/// Lotus-compatible parsing, which is still live. A tab and a carriage return are here because
/// a reader strips leading control characters before deciding, so `"\t=1+1"` is `"=1+1"` by the
/// time it is parsed — the case a check on `=` alone misses.
///
/// `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §6.1. The list is **closed**: a character added to it
/// changes every output format at once, so it is changed deliberately or not at all.
pub const DANGEROUS_PREFIXES: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// The character a defused value is prefixed with.
///
/// A single apostrophe, which every major spreadsheet reads as *"what follows is text"* and
/// does not display. Not a space and not a zero-width character: both would change the value a
/// reader sees or a consumer parses.
pub const TEXT_MARKER: char = '\'';

/// Whether a spreadsheet would read this value as the start of a formula.
#[must_use]
pub fn is_dangerous(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|first| DANGEROUS_PREFIXES.contains(&first))
}

/// One cell's text, safe to write into a spreadsheet.
///
/// Prefixes a single apostrophe when — and only when — the first character is one of
/// [`DANGEROUS_PREFIXES`]. Everything else is returned borrowed and unchanged.
///
/// # Examples
///
/// A `text` block rather than a compiled doctest, as every code example in this workspace is:
/// `tauri-build` writes an 82-byte `msvcrt.lib` into `evara-desktop`'s `OUT_DIR`, rustdoc puts
/// that directory on a doctest's link path, and it shadows the real `msvcrt` — so a compiled
/// doctest anywhere in the workspace fails to link on Windows. The three lines below are
/// pinned as assertions in `the_documented_examples_hold` instead, which cannot rot.
///
/// ```text
/// defuse_formula("=1+1")         == "'=1+1"          // a formula is defused
/// defuse_formula("Mathematics B") -> Cow::Borrowed    // untouched, and not reallocated
/// defuse_formula("Room = 12")    == "Room = 12"      // an `=` that is not first is not one
/// ```
#[must_use]
pub fn defuse_formula(value: &str) -> Cow<'_, str> {
    if is_dangerous(value) {
        let mut out = String::with_capacity(value.len() + 1);
        out.push(TEXT_MARKER);
        out.push_str(value);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_examples_hold() {
        // The three lines of `defuse_formula`'s `# Examples` block, as assertions — because
        // this workspace cannot run a compiled doctest (see that block for why).
        assert_eq!(defuse_formula("=1+1"), "'=1+1");
        assert!(matches!(
            defuse_formula("Mathematics B"),
            Cow::Borrowed("Mathematics B")
        ));
        assert_eq!(defuse_formula("Room = 12"), "Room = 12");
    }

    #[test]
    fn every_dangerous_prefix_is_defused() {
        for prefix in DANGEROUS_PREFIXES {
            let value = format!("{prefix}cmd|' /C calc'!A0");
            let defused = defuse_formula(&value);
            assert!(
                defused.starts_with('\''),
                "{prefix:?} must be defused, got {defused:?}"
            );
            assert_eq!(
                &defused[1..],
                value,
                "the value itself is unchanged behind the marker"
            );
        }
    }

    #[test]
    fn the_four_classic_formula_starts_are_covered() {
        // The set every spreadsheet-injection advisory names, asserted by name rather than
        // only through the loop above, so a shortened constant is a red test.
        assert_eq!(defuse_formula("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(defuse_formula("+1"), "'+1");
        assert_eq!(defuse_formula("-1"), "'-1");
        assert_eq!(defuse_formula("@SUM(A1)"), "'@SUM(A1)");
    }

    #[test]
    fn a_leading_tab_or_carriage_return_is_defused() {
        // A reader strips these before parsing, so `\t=1+1` is `=1+1` by the time it decides.
        assert_eq!(defuse_formula("\t=1+1"), "'\t=1+1");
        assert_eq!(defuse_formula("\r=1+1"), "'\r=1+1");
        assert_eq!(defuse_formula("\r\n=1+1"), "'\r\n=1+1");
    }

    #[test]
    fn a_hyphenated_name_is_still_defused_and_still_readable() {
        // A real case, and the reason the marker is an apostrophe rather than a removal: a
        // teacher called "-Smith" keeps their name.
        let defused = defuse_formula("-Okonkwo");
        assert_eq!(defused, "'-Okonkwo");
        assert_eq!(&defused[1..], "-Okonkwo");
    }

    #[test]
    fn a_negative_number_is_defused_as_text_rather_than_lost() {
        // A penalty is negative-going, and a penalty column written as text is a penalty a
        // reader can still read. Defusing is never deletion.
        assert_eq!(defuse_formula("-1000"), "'-1000");
    }

    #[test]
    fn benign_values_are_borrowed_unchanged() {
        for benign in [
            "Mathematics B",
            "9 Mathematics B",
            "Room 12",
            "Block B - Room 2",
            "Room = 12",
            "a+b",
            "08:00",
            "2026-11-24",
            "0",
            "124",
            "00124",
            " =not first",
            "\n=not first either",
            "#NAME?",
            "'already marked",
        ] {
            let defused = defuse_formula(benign);
            assert!(
                matches!(defused, Cow::Borrowed(_)),
                "{benign:?} is benign and must not be reallocated"
            );
            assert_eq!(defused, benign);
        }
    }

    #[test]
    fn international_text_is_untouched() {
        // A function that prefixed an apostrophe to a name with a diacritic, or to CJK or
        // Arabic text, would be corrupting data to look careful.
        for name in [
            "José Álvarez",
            "Zoë Müller",
            "北門中學",
            "さくら先生",
            "مدرسة النور",
            "Αθηνά",
            "Ростов",
            "Teacher \u{1f3eb}",
        ] {
            let defused = defuse_formula(name);
            assert!(matches!(defused, Cow::Borrowed(_)), "{name:?}");
            assert_eq!(defused, name);
        }
    }

    #[test]
    fn an_empty_value_is_left_empty() {
        // A blank cell must stay blank: a lone apostrophe would turn "no value" into a value.
        let defused = defuse_formula("");
        assert!(matches!(defused, Cow::Borrowed("")));
        assert_eq!(defused, "");
    }

    #[test]
    fn defusing_twice_adds_one_marker_and_then_stops() {
        // The marker itself is not a dangerous prefix, so the function is idempotent — which
        // matters because a renderer must be free to call it without tracking whether it has.
        let once = defuse_formula("=1+1").into_owned();
        let twice = defuse_formula(&once).into_owned();
        assert_eq!(once, "'=1+1");
        assert_eq!(twice, once);
    }

    #[test]
    fn is_dangerous_agrees_with_what_is_defused() {
        for value in ["=1", "+1", "-1", "@1", "\t1", "\r1", "a", "", " =1", "1"] {
            assert_eq!(
                is_dangerous(value),
                defuse_formula(value) != Cow::Borrowed(value),
                "{value:?}"
            );
        }
    }
}
