//! Detecting cloud-synchronised folders.
//!
//! WAL-mode SQLite inside OneDrive, Dropbox, iCloud Drive or Google Drive is a known
//! corruption source: the sync client copies the `-wal` and `-shm` sidecar files
//! independently of the main database, and can restore them out of step with each other.
//! Users will put project files there anyway, because that is where their documents live.
//!
//! This module is **detection only**. It is a pure function over a path, with no I/O and
//! no interface: the warning itself belongs with project opening in Phase 1B, and
//! `*.evara` export remains the supported way to move a project between machines.

use std::path::Path;

/// A cloud storage provider recognised from a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyncedFolder {
    /// Microsoft OneDrive, including OneDrive for Business.
    OneDrive,
    /// Dropbox.
    Dropbox,
    /// Apple iCloud Drive.
    ICloud,
    /// Google Drive, including Drive for desktop.
    GoogleDrive,
    /// Box Drive.
    Box,
    /// Nextcloud or ownCloud.
    Nextcloud,
    /// Proton Drive.
    ProtonDrive,
}

impl SyncedFolder {
    /// Provider name for display.
    #[must_use]
    pub const fn provider(self) -> &'static str {
        match self {
            Self::OneDrive => "OneDrive",
            Self::Dropbox => "Dropbox",
            Self::ICloud => "iCloud Drive",
            Self::GoogleDrive => "Google Drive",
            Self::Box => "Box",
            Self::Nextcloud => "Nextcloud",
            Self::ProtonDrive => "Proton Drive",
        }
    }
}

/// Path fragments that identify each provider, lowercased.
///
/// Matched against whole path components so that a folder merely *named* "my onedrive
/// backup" does not trigger a false positive, while `OneDrive - Contoso Ltd` does.
const MARKERS: &[(&str, SyncedFolder)] = &[
    ("onedrive", SyncedFolder::OneDrive),
    ("dropbox", SyncedFolder::Dropbox),
    ("icloud drive", SyncedFolder::ICloud),
    ("com~apple~clouddocs", SyncedFolder::ICloud),
    ("mobile documents", SyncedFolder::ICloud),
    ("google drive", SyncedFolder::GoogleDrive),
    ("googledrive", SyncedFolder::GoogleDrive),
    ("my drive", SyncedFolder::GoogleDrive),
    ("box", SyncedFolder::Box),
    ("nextcloud", SyncedFolder::Nextcloud),
    ("owncloud", SyncedFolder::Nextcloud),
    ("proton drive", SyncedFolder::ProtonDrive),
    ("protondrive", SyncedFolder::ProtonDrive),
];

/// Reports the cloud provider a path appears to live under, if any.
///
/// Heuristic by nature — there is no reliable cross-platform way to ask whether a
/// directory is synchronised. It is therefore only ever used to *warn*, never to refuse
/// to open a project: a false positive must not stop someone using their own file.
#[must_use]
pub fn detect(path: &Path) -> Option<SyncedFolder> {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy().to_lowercase();

        for (marker, provider) in MARKERS {
            let matched = match *provider {
                // "OneDrive - Contoso" and "Dropbox (Personal)" are real folder names, so
                // a prefix match is right for providers that append an account label.
                SyncedFolder::OneDrive | SyncedFolder::Dropbox => {
                    name == *marker
                        || name.starts_with(&format!("{marker} "))
                        || name.starts_with(&format!("{marker}-"))
                }
                // Everything else must match the component exactly. "Box" especially:
                // sandbox, toolbox and boxes are ordinary directory names.
                _ => name == *marker,
            };
            if matched {
                return Some(*provider);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{SyncedFolder, detect};
    use std::path::Path;

    #[test]
    fn detects_common_providers() {
        let cases: &[(&str, SyncedFolder)] = &[
            (r"C:\Users\sam\OneDrive\school.evdb", SyncedFolder::OneDrive),
            (
                r"C:\Users\sam\OneDrive - Contoso Ltd\school.evdb",
                SyncedFolder::OneDrive,
            ),
            ("/Users/sam/Dropbox/school.evdb", SyncedFolder::Dropbox),
            (
                "/Users/sam/Dropbox (Personal)/school.evdb",
                SyncedFolder::Dropbox,
            ),
            (
                "/Users/sam/Library/Mobile Documents/school.evdb",
                SyncedFolder::ICloud,
            ),
            (
                "/Users/sam/Google Drive/My Drive/school.evdb",
                SyncedFolder::GoogleDrive,
            ),
            ("/home/sam/Nextcloud/school.evdb", SyncedFolder::Nextcloud),
        ];
        for (path, expected) in cases {
            assert_eq!(detect(Path::new(path)), Some(*expected), "path: {path}");
        }
    }

    #[test]
    fn ignores_ordinary_paths() {
        for path in [
            r"C:\Users\sam\Documents\school.evdb",
            "/Users/sam/Projects/school.evdb",
            "/var/tmp/school.evdb",
        ] {
            assert_eq!(detect(Path::new(path)), None, "path: {path}");
        }
    }

    #[test]
    fn does_not_fire_on_similarly_named_folders() {
        // A false positive warns a user away from a perfectly safe location, so the
        // matcher must not treat any folder containing these words as synchronised.
        for path in [
            "/home/sam/sandbox/school.evdb",
            "/home/sam/toolbox/school.evdb",
            "/home/sam/my-dropbox-backups/school.evdb",
            "/home/sam/boxes/school.evdb",
            r"C:\work\onedrive_exports\school.evdb",
        ] {
            assert_eq!(detect(Path::new(path)), None, "path: {path}");
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            detect(Path::new("/Users/sam/DROPBOX/school.evdb")),
            Some(SyncedFolder::Dropbox)
        );
    }

    #[test]
    fn provider_names_are_displayable() {
        assert_eq!(SyncedFolder::OneDrive.provider(), "OneDrive");
        assert_eq!(SyncedFolder::ICloud.provider(), "iCloud Drive");
    }
}
