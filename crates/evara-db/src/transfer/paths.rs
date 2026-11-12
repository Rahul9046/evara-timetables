//! The path form a database may be opened by, and why clearing a flag is not the control.
//!
//! # Finding F-2: `SQLITE_OPEN_URI` is not a switch this build owns
//!
//! The architecture record said URI filenames were disabled by leaving
//! `SQLITE_OPEN_URI` out of the open flags. That is not what the flag does here. The
//! bundled engine is compiled with **`-DSQLITE_USE_URI`**
//! (`libsqlite3-sys-0.35.0/build.rs:140`), and that compile-time option makes URI
//! interpretation the **global default**: `sqlite3_open_v2` parses a filename beginning
//! `file:` as a URI whether or not the caller asked for it. The flag can turn URI handling
//! *on* for a build that defaults off; it cannot turn it off for a build that defaults on.
//!
//! `t91_a_uri_filename_is_interpreted_without_the_open_uri_flag` proves it against the
//! bundled engine rather than asserting it from the build flag, because the whole point of
//! F-2 is that the record believed a control it had not tested.
//!
//! # So what is the control
//!
//! Two things, neither of them a flag:
//!
//! 1. **Every path Evara opens an untrusted payload by is one Evara chose.** A staging
//!    directory Evara created, with a UUIDv7 file name Evara minted. No byte of archive
//!    content contributes to any path — that is `D6`, and it is the reason the question is
//!    closed before this module is reached.
//! 2. **The path form is asserted anyway**, by [`require_plain_database_path`], at the one
//!    place a payload is opened. Belt and braces by design: the assertion costs a string
//!    scan, and it is what turns a future refactor that *did* let a name through into a
//!    refusal rather than a URI with `vfs=` and `mode=rwc` in it.
//!
//! # What the forms mean, so the list is reviewable rather than superstitious
//!
//! | Form | What SQLite would do with it |
//! | --- | --- |
//! | `file:…` | parse the whole string as a URI: `?` begins query parameters, `#` a fragment, `%XX` is decoded |
//! | `?…` anywhere | inside a URI, `mode=rwc` re-grants write, `vfs=` selects a VFS, `immutable=1` declares the file cannot change |
//! | `#…` | truncates the path at the fragment, so the file opened is not the file named |
//! | `%XX` | decodes to anything, `%2F`, `%5C` and `%00` included — a separator or a terminator smuggled past a component walk |
//! | a NUL byte | terminates the C string early, so the engine opens a prefix of the path |
//! | a `..` component | escapes the directory Evara chose, which is the only thing making point 1 true |
//!
//! Nothing here is applied to a **destination** the user picked in a native dialog: `#` is a
//! legal character in an NTFS file name and a school may well have one. This is the rule for
//! the paths Evara mints, and asserting it there costs a user nothing.

use std::path::Path;

use crate::error::{DbError, Result};
use crate::file_name_of;

/// Why a path may not be used to open a database.
///
/// Named individually rather than collapsed into a boolean so a refusal says which form was
/// found, and so the test that drives every one of them reads as a list of decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathHazard {
    /// Nothing at all.
    Empty,
    /// Begins `file:`, which the bundled engine parses as a URI.
    UriScheme,
    /// Holds a `?`, which separates URI query parameters.
    QuerySeparator,
    /// Holds a `#`, which begins a URI fragment.
    Fragment,
    /// Holds a `%`, which introduces a percent-encoded byte.
    PercentEncoded,
    /// Holds a NUL, which terminates the C string the engine receives.
    InteriorNul,
    /// Holds a `..` component, so it does not stay inside the directory Evara chose.
    ParentComponent,
    /// Is not valid UTF-8, so its form cannot be reasoned about at all.
    NotUnicode,
}

impl PathHazard {
    /// A short phrase naming the form, for a refusal message.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "an empty path",
            Self::UriScheme => "a `file:` URI rather than a path",
            Self::QuerySeparator => "a `?` query separator",
            Self::Fragment => "a `#` fragment",
            Self::PercentEncoded => "a `%` escape",
            Self::InteriorNul => "an interior NUL",
            Self::ParentComponent => "a `..` component",
            Self::NotUnicode => "bytes that are not valid Unicode",
        }
    }
}

/// The first hazard in `path`, or `None` if it is a plain file path.
///
/// Pure, so every form can be driven from a string in a test without creating a file.
#[must_use]
pub fn classify_database_path(path: &Path) -> Option<PathHazard> {
    let Some(text) = path.to_str() else {
        return Some(PathHazard::NotUnicode);
    };
    if text.is_empty() {
        return Some(PathHazard::Empty);
    }

    // Checked on the whole string rather than per component: SQLite reads the filename it is
    // handed, and a `file:` appearing after a separator is not a scheme, while one at the
    // front is. Ordered so the most explanatory finding is the one reported.
    if text.len() >= 5 && text[..5].eq_ignore_ascii_case("file:") {
        return Some(PathHazard::UriScheme);
    }
    if text.contains('\0') {
        return Some(PathHazard::InteriorNul);
    }
    if text.contains('?') {
        return Some(PathHazard::QuerySeparator);
    }
    if text.contains('#') {
        return Some(PathHazard::Fragment);
    }
    if text.contains('%') {
        return Some(PathHazard::PercentEncoded);
    }
    // `Component::ParentDir` rather than a substring search for `..`, so a file legitimately
    // named `report..evaraproj` is not refused while `../x` is.
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Some(PathHazard::ParentComponent);
    }
    None
}

/// Refuses a path SQLite would read as anything other than a plain file name.
///
/// Called at the untrusted-payload boundary and before a staged payload is migrated. See
/// this module's header for why this, and not an open flag, is the control.
///
/// # Errors
///
/// [`DbError::DestinationPathUnsafe`], naming the file and the form found.
pub fn require_plain_database_path(path: &Path) -> Result<()> {
    match classify_database_path(path) {
        None => Ok(()),
        Some(hazard) => Err(DbError::DestinationPathUnsafe {
            name: if matches!(hazard, PathHazard::NotUnicode | PathHazard::Empty) {
                String::from("that file")
            } else {
                file_name_of(path)
            },
            reason: hazard.as_str(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PathHazard, classify_database_path, require_plain_database_path};

    #[test]
    fn a_plain_path_has_no_hazard() {
        assert_eq!(
            classify_database_path(Path::new("/schools/north.evaraproj")),
            None
        );
        assert!(require_plain_database_path(Path::new("/schools/north.evaraproj")).is_ok());
    }

    #[test]
    fn every_uri_form_is_named() {
        // The forms F-2 asks for, each with the reason it is dangerous rather than merely
        // unusual. Driven as strings so no file has to exist.
        for (path, expected) in [
            ("file:/schools/north.evaraproj", PathHazard::UriScheme),
            ("FILE:north.evaraproj", PathHazard::UriScheme),
            ("/s/north.evaraproj?mode=rwc", PathHazard::QuerySeparator),
            (
                "/s/north.evaraproj?vfs=unix-none",
                PathHazard::QuerySeparator,
            ),
            ("/s/north.evaraproj#frag", PathHazard::Fragment),
            ("/s/north%2fevil.evaraproj", PathHazard::PercentEncoded),
            ("/s/north%5Cevil.evaraproj", PathHazard::PercentEncoded),
            ("/s/north%00.evaraproj", PathHazard::PercentEncoded),
            ("/s/../escape.evaraproj", PathHazard::ParentComponent),
            ("..", PathHazard::ParentComponent),
            ("", PathHazard::Empty),
        ] {
            assert_eq!(
                classify_database_path(Path::new(path)),
                Some(expected),
                "{path} must be classified as {expected:?}"
            );
            assert!(
                require_plain_database_path(Path::new(path)).is_err(),
                "{path} must be refused"
            );
        }
    }

    #[test]
    fn a_nul_is_found_before_a_query_separator() {
        // Order matters only for the message, but it is asserted so a reordering is a
        // deliberate change rather than a surprise in a bug report.
        assert_eq!(
            classify_database_path(Path::new("/s/a\0b?c")),
            Some(PathHazard::InteriorNul)
        );
    }

    #[test]
    fn a_double_dot_inside_a_name_is_not_a_parent_component() {
        // The reason this walks components instead of searching for the substring.
        assert_eq!(
            classify_database_path(Path::new("/s/report..evaraproj")),
            None
        );
    }
}
