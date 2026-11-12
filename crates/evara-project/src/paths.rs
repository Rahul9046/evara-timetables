//! Paths a transfer will write to, and the one rename primitive it writes through.
//!
//! # A destination is only ever written by a rename
//!
//! That sentence is the guarantee D11 relies on, and this module is where it is implemented.
//! A single rename within one directory on one volume is atomic for other observers: either
//! the old name or the new name resolves, never a half-written file. Staging is therefore
//! always a **sibling** of the destination, so no rename crosses a volume — a cross-volume
//! rename is a copy, and it fails at the worst possible moment.
//!
//! What is deliberately **not** claimed is durability. `std::fs` cannot `fsync` a directory
//! handle on Windows, so after a power loss the filesystem may present either side of a
//! rename. The claim is weaker and sufficient: the set of reachable states is
//! `{old project intact}` or `{new project in place, old project kept beside it}` — never a
//! partially replaced destination.
//!
//! # The bounded retry, and why it is bounded
//!
//! Both platforms can hold a newly written file open briefly — an antivirus scanner, a search
//! indexer. So every rename gets five attempts a hundred milliseconds apart, and then a named
//! refusal. **Not an unbounded loop**: a permanently locked destination is a condition the
//! user has to be told about, and a transfer that waited for ever would present as a hang.
//!
//! # The path forms refused, and what they have to do with SQLite
//!
//! A destination comes from a native file dialog, so it is the user's own path and `#` in a
//! file name is legitimate. What is refused here is narrower: a symlink anywhere in the
//! chain, and the `file:` prefix that the **bundled SQLite would read as a URI** — finding
//! F-2, recorded in `evara_db::transfer::paths`. The staged payload's own path is Evara's and
//! is asserted by that module; this is the destination half.

use std::path::{Component, Path};

use crate::error::{Result, TransferError};

/// How many times a rename is retried before it is reported.
const RENAME_ATTEMPTS: u32 = 5;

/// How long between attempts.
const RENAME_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Refuses a destination Evara will not write to.
///
/// Three rules, each with a reason:
///
/// 1. **No `file:` prefix.** The bundled engine is compiled with `-DSQLITE_USE_URI`, so URI
///    interpretation is the global default and `SQLITE_OPEN_URI` cannot subtract it — F-2.
///    A destination SQLite would read as `file:…?mode=ro` is refused here rather than opened
///    later with parameters nobody chose.
/// 2. **No `..` component.** A destination is a path the user picked, and staging is created
///    as its sibling; a `..` makes "sibling" mean something a reader cannot predict.
/// 3. **No symlink in the chain.** `symlink_metadata` on the destination and every ancestor
///    that exists. A rename onto a symlink writes wherever the link points, which is outside
///    the directory the user chose and outside the volume the atomicity claim is about.
///
/// # Errors
///
/// [`TransferError::DestinationUnsafe`], naming the file and the rule.
pub fn require_safe_destination(path: &Path) -> Result<()> {
    let name = file_name_of(path);

    if let Some(text) = path.to_str()
        && text.len() >= 5
        && text[..5].eq_ignore_ascii_case("file:")
    {
        return Err(TransferError::DestinationUnsafe {
            name,
            reason: "it is a URI rather than a path",
        });
    }

    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(TransferError::DestinationUnsafe {
            name,
            reason: "it contains `..`",
        });
    }

    // The destination itself, then every ancestor. A link *anywhere* in the chain moves where
    // the write lands, so checking only the leaf would miss the common arrangement — a linked
    // `Documents` folder.
    let mut current = Some(path);
    while let Some(candidate) = current {
        if let Ok(metadata) = candidate.symlink_metadata()
            && metadata.file_type().is_symlink()
        {
            return Err(TransferError::DestinationUnsafe {
                name,
                reason: "part of that path is a link",
            });
        }
        current = candidate
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
    }

    Ok(())
}

/// Renames `from` to `to`, retrying a bounded number of times.
///
/// # Errors
///
/// - [`TransferError::RenameBlocked`] after the last attempt, naming the file.
/// - [`TransferError::Io`] for a failure retrying cannot help with.
pub fn rename_with_retry(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                // Only a sharing violation or a permission refusal is worth retrying. A
                // missing source or a cross-device rename will not become true by waiting,
                // and retrying one would turn an immediate, explicable failure into a
                // half-second hang followed by the wrong message.
                if !worth_retrying(&error) {
                    return Err(TransferError::Io {
                        action: "put the campus file in place",
                        source: error,
                    });
                }
                last = Some(error);
                if attempt + 1 < RENAME_ATTEMPTS {
                    std::thread::sleep(RENAME_BACKOFF);
                }
            }
        }
    }

    // The last error is dropped in favour of the named refusal, because what a user needs
    // here is the remedy — close the other program — and not an OS error number.
    let _ = last;
    Err(TransferError::RenameBlocked {
        name: file_name_of(to),
    })
}

/// Whether waiting could plausibly make this rename succeed.
fn worth_retrying(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
    ) || error.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// Windows' sharing violation, which `io::ErrorKind` does not name.
///
/// The error a rename gets when another process holds the file open without
/// `FILE_SHARE_DELETE` — which is how SQLite opens a database, so it is the error the
/// platform asymmetry in D11's guarantees section is about. Named by number because
/// `ErrorKind` has no variant for it, and the constant is harmless on macOS where no error
/// carries it.
const ERROR_SHARING_VIOLATION: i32 = 32;

/// File name of a path, for a message that must not carry the whole path.
pub(crate) fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("the selected file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::{rename_with_retry, require_safe_destination};
    use crate::error::TransferError;

    #[test]
    fn an_ordinary_destination_is_accepted() {
        let dir = TempDir::new().expect("temp dir");
        require_safe_destination(&dir.path().join("Northgate.evaraproj"))
            .expect("a plain path in a real directory is fine");
    }

    #[test]
    fn a_uri_destination_is_refused() {
        // F-2: the bundled engine would read this as a URI whatever open flags are passed.
        match require_safe_destination(Path::new("file:/schools/north.evaraproj")) {
            Err(TransferError::DestinationUnsafe { reason, .. }) => {
                assert_eq!(reason, "it is a URI rather than a path");
            }
            other => panic!("expected DestinationUnsafe, got {other:?}"),
        }
    }

    #[test]
    fn a_parent_component_is_refused() {
        match require_safe_destination(Path::new("/schools/../north.evaraproj")) {
            Err(TransferError::DestinationUnsafe { reason, .. }) => {
                assert_eq!(reason, "it contains `..`");
            }
            other => panic!("expected DestinationUnsafe, got {other:?}"),
        }
    }

    #[test]
    fn a_rename_into_place_succeeds() {
        let dir = TempDir::new().expect("temp dir");
        let from = dir.path().join("staged");
        let to = dir.path().join("final.evaraproj");
        std::fs::write(&from, b"bytes").expect("write");
        rename_with_retry(&from, &to).expect("rename");
        assert!(to.exists() && !from.exists());
    }

    #[test]
    fn a_missing_source_is_reported_immediately_rather_than_retried() {
        let dir = TempDir::new().expect("temp dir");
        let at = std::time::Instant::now();
        let error = rename_with_retry(
            &dir.path().join("not-there"),
            &dir.path().join("final.evaraproj"),
        )
        .expect_err("a missing source cannot be renamed");

        match error {
            TransferError::Io { action, .. } => {
                assert_eq!(action, "put the campus file in place");
            }
            other => panic!("expected Io, got {other:?}"),
        }
        // Four backoffs would be 400 ms. The assertion is loose because it is about not
        // sleeping at all, not about how fast the filesystem is.
        assert!(
            at.elapsed() < std::time::Duration::from_millis(300),
            "a failure retrying cannot fix must not be retried"
        );
    }
}
