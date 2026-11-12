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
//! file name is legitimate. What is refused here is narrower: a symlink **at the destination
//! itself**, and the `file:` prefix that the **bundled SQLite would read as a URI** — finding
//! F-2, recorded in `evara_db::transfer::paths`. The staged payload's own path is Evara's and
//! is asserted by that module; this is the destination half.
//!
//! A link in an *ancestor* is accepted, because staging is a lexical sibling and so resolves
//! through the same chain into the same real directory — macOS reaches `/var` and `/tmp`
//! through links, and refusing those refused the platform's own temporary directory.
//! [`require_safe_destination`] carries the full argument.

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
/// 3. **No symlink at the leaf.** `symlink_metadata` on the destination itself, which does
///    not follow it. Renaming *onto* a link replaces the link and leaves its target alone, so
///    a Replace would move the link aside and keep that as the user's preserved project —
///    their real file untouched behind a name that now points at something else. That is the
///    preservation failure D11 exists to prevent, so it is refused rather than worked around.
/// 4. **The deepest existing ancestor must resolve to a directory.** `canonicalize` resolves
///    every link in the chain, so the destination is known to be somewhere real.
///
/// # Why an ancestor link is allowed, and why that is not a weakening
///
/// Because [`crate::staging::Staging::open`] builds the staging directory as a **lexical
/// sibling** — `destination.parent().join(…)` — so staging and destination traverse the
/// *identical* ancestor chain. Whatever those ancestors resolve to, both paths land in one
/// real directory on one volume, which is the whole of what the atomicity claim needs. An
/// ancestor link cannot separate them.
///
/// Refusing one, by contrast, refuses ordinary systems: macOS reaches `/var` and `/tmp`
/// through links to `/private/…`, so every path under the platform's own temporary directory
/// was refused — 54 tests on macOS, and any real export beneath such a folder. The rule's
/// intent was right and its scope was wrong.
///
/// # TOCTOU
///
/// The gap between this check and the rename cannot be closed by a check, so what bounds it
/// is that **no code path here ever follows a link to write**: the destination is only ever
/// renamed onto or renamed away, and `rename` does not follow a final-component link. A link
/// swapped into place after this returns is therefore replaced, never written through, so the
/// window cannot redirect a write outside the chosen directory. An attacker able to swap an
/// *ancestor* directory for a link already has write access to the folder the user picked;
/// that is not an escalation this function could prevent, and §6.2 states it rather than
/// claiming otherwise.
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

    // The leaf only, and without following it. An ancestor link is fine — see the header for
    // why — but a link *at* the destination is the preservation failure described there.
    if let Ok(metadata) = path.symlink_metadata()
        && metadata.file_type().is_symlink()
    {
        return Err(TransferError::DestinationUnsafe {
            name,
            reason: "the file it names is a link rather than a campus file",
        });
    }

    // And the destination is somewhere real. `canonicalize` resolves every link in the chain,
    // which is what turns macOS's `/var` into `/private/var` rather than into a refusal.
    //
    // The *deepest existing* ancestor rather than the immediate parent, because a destination
    // in a folder that does not exist yet is legitimate: `Staging::open` calls
    // `create_dir_all`, so import can create it. Requiring the parent to exist would refuse
    // that, which is a behaviour change this fix has no reason to make.
    let anchor = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .find(|ancestor| ancestor.exists())
        .ok_or_else(|| TransferError::DestinationUnsafe {
            name: file_name_of(path),
            reason: "no folder on that path exists",
        })?;

    let resolved = anchor
        .canonicalize()
        .map_err(|_| TransferError::DestinationUnsafe {
            name: file_name_of(path),
            reason: "its folder cannot be resolved",
        })?;

    if !resolved.is_dir() {
        return Err(TransferError::DestinationUnsafe {
            name: file_name_of(path),
            reason: "the folder it names is not a folder",
        });
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

    // The defect this module was fixed for: macOS reaches `/var` and `/tmp` through links to
    // `/private/…`, so a destination under the platform's own temporary directory has a link
    // in its ancestor chain. Refusing that refused 54 tests and every real export beneath
    // such a folder.
    //
    // `cfg(unix)` because creating a link on Windows needs a privilege CI does not grant, and
    // a test that silently skipped would assert nothing while looking green. macOS is the
    // platform the defect was on and the platform this proves the fix on.
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_ancestor_chain_is_accepted_as_macos_var_requires() {
        let dir = TempDir::new().expect("temp dir");
        let real = dir.path().join("private").join("schools");
        std::fs::create_dir_all(&real).expect("create the real directory");

        // `link` -> `private`, exactly `/var` -> `/private/var`.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("private"), &link).expect("create the link");

        // Reached *through* the link, which is what a macOS temp path looks like.
        let destination = link.join("schools").join("Northgate.evaraproj");
        require_safe_destination(&destination)
            .expect("a link in the ancestor chain is how macOS reaches its own temp directory");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_destination_itself_is_still_refused() {
        let dir = TempDir::new().expect("temp dir");
        let elsewhere = dir.path().join("somebody-elses.evaraproj");
        std::fs::write(&elsewhere, b"not ours").expect("write");

        // The leaf is the link. Renaming onto it would replace the link and leave the real
        // file alone, so a Replace would preserve the link and not the user's project.
        let destination = dir.path().join("Northgate.evaraproj");
        std::os::unix::fs::symlink(&elsewhere, &destination).expect("create the link");

        match require_safe_destination(&destination) {
            Err(TransferError::DestinationUnsafe { reason, .. }) => {
                assert_eq!(
                    reason,
                    "the file it names is a link rather than a campus file"
                );
            }
            other => panic!("a link at the destination must still be refused, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_chain_does_not_separate_staging_from_the_destination() {
        // The reason an ancestor link is safe, asserted rather than argued: staging is a
        // lexical sibling, so both paths resolve into one real directory. If that ever stopped
        // being true the rename would cross a volume and D11's atomicity claim would be void.
        let dir = TempDir::new().expect("temp dir");
        let real = dir.path().join("private");
        std::fs::create_dir_all(&real).expect("create");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("link");

        let destination = link.join("Northgate.evaraproj");
        let staging_sibling = destination
            .parent()
            .expect("a destination has a parent")
            .join("staging-01");
        std::fs::create_dir_all(&staging_sibling).expect("create staging");

        assert_eq!(
            staging_sibling
                .canonicalize()
                .expect("resolve staging")
                .parent()
                .expect("staging has a parent"),
            destination
                .parent()
                .expect("destination has a parent")
                .canonicalize()
                .expect("resolve the destination's folder"),
            "staging and the destination must resolve into one real directory"
        );
    }

    #[test]
    fn a_destination_whose_folder_does_not_exist_is_still_accepted() {
        // Import creates it with `create_dir_all`, so refusing here would be a behaviour
        // change the symlink fix has no reason to make.
        let dir = TempDir::new().expect("temp dir");
        require_safe_destination(&dir.path().join("not-yet").join("Northgate.evaraproj"))
            .expect("a folder import would create is not an unsafe destination");
    }

    #[test]
    fn a_destination_under_the_platform_temporary_directory_is_accepted() {
        // The end-to-end shape of the defect, with no link built by hand: on macOS this path
        // runs through `/var` -> `/private/var` and this assertion is what failed 54 times.
        let dir = TempDir::new().expect("temp dir");
        require_safe_destination(&dir.path().join("Northgate.evaraproj"))
            .expect("the platform's own temporary directory must be a usable destination");
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
