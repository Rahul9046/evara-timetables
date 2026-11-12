//! The three entry names, and the policy that makes a path from an archive unexpressible.
//!
//! # ADR 0055: read by name, never extracted
//!
//! Every traversal vulnerability in every language has the same shape: iterate the archive's
//! directory, sanitise each name, write it out. The sanitising is where the bugs are —
//! `..%2f`, a drive letter, a backslash on a platform that treats it as a separator, a
//! Unicode form that normalises into a separator, a name that is fine until the filesystem
//! decomposes it.
//!
//! This reader does not do that. It opens **exactly three entries by exact name** and
//! streams the one large one into a file whose path *Evara* chose. No byte of archive content
//! ever contributes to a path, so traversal is not defended against — it is unexpressible.
//! `zip`'s own `extract()`, where all three of the RUSTSEC advisories Stage P closed live, is
//! never called.
//!
//! # So why is there a name policy at all
//!
//! Because an archive that *contains* a traversal attempt is evidence, and the right response
//! to evidence is to stop. [`classify`] is applied to every name in the central directory,
//! and an unsafe one refuses the whole archive rather than being skipped: an archive built to
//! escape a directory is not one to read the rest of.
//!
//! Unknown-but-**safe** names are the opposite case. They are ignored and reported — ADR
//! 0004's forward tolerance — so a `formatVersion 1` reader meets a future entry without
//! refusing, and T-36 tests that rather than assuming it.

use crate::error::TransferError;
use crate::limits::MAX_ENTRY_NAME_BYTES;

/// The manifest: what the archive says about itself.
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// The digest record: one digest per entry whose integrity is covered.
pub const CHECKSUMS_ENTRY: &str = "checksums.json";

/// The payload: a verified SQLite snapshot of the project.
pub const PAYLOAD_ENTRY: &str = "payload/project.evaraproj";

/// The three entries `formatVersion 1` writes, in write order.
///
/// The manifest first, because it is parsed before anything else is trusted and a reader
/// should meet it at the start of the file rather than after a payload.
pub const REQUIRED_ENTRIES: &[&str] = &[MANIFEST_ENTRY, CHECKSUMS_ENTRY, PAYLOAD_ENTRY];

/// Why an entry name is not one Evara will have in an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameHazard {
    /// Empty.
    Empty,
    /// Longer than [`MAX_ENTRY_NAME_BYTES`].
    TooLong,
    /// Rooted: begins `/` or `\`.
    Absolute,
    /// Drive-qualified, as `C:` or `\\server\share`.
    DriveQualified,
    /// Holds a `..` component.
    ParentComponent,
    /// Holds a backslash, which is a separator on one of the two platforms.
    Backslash,
    /// Holds a NUL, or any other control character.
    ControlCharacter,
    /// Not valid UTF-8 in its stored form.
    NotUnicode,
    /// Ends in `/`, so it is a directory entry.
    Directory,
}

impl NameHazard {
    /// A short phrase for a refusal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "it is empty",
            Self::TooLong => "it is longer than Evara will read",
            Self::Absolute => "it is an absolute path",
            Self::DriveQualified => "it names a drive or a network share",
            Self::ParentComponent => "it contains `..`",
            Self::Backslash => "it contains a backslash",
            Self::ControlCharacter => "it contains a control character",
            Self::NotUnicode => "it is not valid text",
            Self::Directory => "it is a directory",
        }
    }
}

/// The first hazard in `name`, or `None` if it is a safe relative file name.
///
/// Pure, and applied to the **raw stored bytes** rather than to anything a reader has already
/// interpreted: the point is to judge what the archive actually says, not what a path type
/// made of it.
#[must_use]
pub fn classify(name: &[u8]) -> Option<NameHazard> {
    if name.is_empty() {
        return Some(NameHazard::Empty);
    }
    if name.len() > MAX_ENTRY_NAME_BYTES {
        return Some(NameHazard::TooLong);
    }
    let Ok(text) = std::str::from_utf8(name) else {
        return Some(NameHazard::NotUnicode);
    };

    // Control characters first, NUL included. A name carrying one has already told us the
    // archive was not written by anything reasonable, and reporting a later finding would
    // describe the symptom rather than the cause.
    if text.chars().any(char::is_control) {
        return Some(NameHazard::ControlCharacter);
    }
    if text.contains('\\') {
        return Some(NameHazard::Backslash);
    }
    if text.starts_with('/') {
        return Some(NameHazard::Absolute);
    }
    // `C:` and `\\share`. The second is already caught by the backslash rule; the first is
    // not, and a reader on a platform where `C:x` is drive-relative would resolve it.
    if text.len() >= 2 && text.as_bytes()[1] == b':' {
        return Some(NameHazard::DriveQualified);
    }
    // Split on `/` only. A name is `/`-separated by the format, so this is the format's own
    // notion of a component and not the host's.
    if text.split('/').any(|segment| segment == "..") {
        return Some(NameHazard::ParentComponent);
    }
    if text.ends_with('/') {
        return Some(NameHazard::Directory);
    }
    None
}

/// Refuses the archive if any name in it is unsafe.
///
/// # Errors
///
/// [`TransferError::UnsafeEntryName`], naming the entry and which rule it broke.
pub fn require_safe(name: &[u8]) -> crate::error::Result<()> {
    match classify(name) {
        None => Ok(()),
        Some(hazard) => Err(TransferError::UnsafeEntryName {
            entry: String::from_utf8_lossy(name).into_owned(),
            reason: hazard.as_str(),
        }),
    }
}

/// Whether a safe name is one `formatVersion 1` knows.
///
/// A safe name that is not one of the three is ignored and reported, never read.
#[must_use]
pub fn is_known(name: &str) -> bool {
    REQUIRED_ENTRIES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{MAX_ENTRY_NAME_BYTES, NameHazard, classify, is_known, require_safe};

    #[test]
    fn the_three_names_are_safe_and_known() {
        for name in super::REQUIRED_ENTRIES {
            assert_eq!(classify(name.as_bytes()), None, "{name} must be safe");
            assert!(is_known(name));
        }
    }

    #[test]
    fn every_traversal_shape_refuses() {
        // T-31's list, plus the shapes a sanitising reader gets wrong. Each is judged from
        // the raw bytes, so none of them has to be constructed as a path first.
        for (name, expected) in [
            (b"../../evil".as_slice(), NameHazard::ParentComponent),
            (b"payload/../../evil", NameHazard::ParentComponent),
            (b"..", NameHazard::ParentComponent),
            (b"/etc/passwd", NameHazard::Absolute),
            (b"C:\\evil", NameHazard::Backslash),
            (b"C:/evil", NameHazard::DriveQualified),
            (b"a\\b", NameHazard::Backslash),
            (b"a\0b", NameHazard::ControlCharacter),
            (b"payload/", NameHazard::Directory),
            (b"", NameHazard::Empty),
            (b"\xff\xfe", NameHazard::NotUnicode),
        ] {
            assert_eq!(
                classify(name),
                Some(expected),
                "{} must be classified as {expected:?}",
                String::from_utf8_lossy(name)
            );
            assert!(require_safe(name).is_err());
        }
    }

    #[test]
    fn a_four_kilobyte_name_is_refused_before_it_is_parsed() {
        let long = vec![b'a'; MAX_ENTRY_NAME_BYTES + 1];
        assert_eq!(classify(&long), Some(NameHazard::TooLong));
        // And one byte under is fine, so the cap is a cap rather than an approximation.
        let just_fits = vec![b'a'; MAX_ENTRY_NAME_BYTES];
        assert_eq!(classify(&just_fits), None);
    }

    #[test]
    fn a_percent_encoded_separator_is_not_a_separator_here() {
        // `%2f` is refused by nothing, and must not be: this reader never turns a name into
        // a path, so `a%2fb` is simply an unknown-but-safe entry name that is ignored. The
        // test exists because a *sanitising* reader has to decode it, and that is the class
        // of bug ADR 0055 avoids by not having the code.
        assert_eq!(classify(b"a%2fb"), None);
        assert!(
            !is_known("a%2fb"),
            "and it is not an entry this build reads"
        );
    }

    #[test]
    fn an_unknown_but_safe_name_is_safe_and_unknown() {
        assert_eq!(classify(b"attachments/notes.txt"), None);
        assert!(!is_known("attachments/notes.txt"));
    }
}
