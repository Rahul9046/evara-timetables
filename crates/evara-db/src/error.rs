//! Typed errors for every persistence failure.
//!
//! Callers must be able to distinguish "this project is from a newer Evara" from "the
//! disk is broken" without matching on strings, because the two need completely
//! different messages in the interface.
//!
//! # The driver does not cross this boundary
//!
//! No variant exposes a `rusqlite` or `refinery` type. Underlying failures are wrapped in
//! an opaque [`InternalError`], so a consumer of this crate never needs the SQLite driver
//! on its dependency list — not even to construct an error in a test. The source chain is
//! preserved for logging and debugging; it is simply not part of the public type.
//!
//! Nor do messages carry filesystem paths. A failure names the file, never where it lives.

use std::fmt;

/// Result alias used throughout this crate.
pub type Result<T> = std::result::Result<T, DbError>;

/// An underlying failure, kept opaque.
///
/// Wraps whatever actually went wrong — a SQLite error, a migration failure — without
/// naming its type in the public API. `Display` gives the detail for a log; the
/// user-facing wording lives on [`DbError`].
pub struct InternalError(Box<dyn std::error::Error + Send + Sync + 'static>);

impl InternalError {
    /// Wraps a concrete error. Crate-internal, so driver types cannot escape.
    pub(crate) fn wrap(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(source))
    }

    /// Builds one from a message, where there is no underlying error to wrap.
    ///
    /// Also how a consumer synthesises a failure in a test without reaching for the
    /// driver.
    #[must_use]
    pub fn from_message(message: impl Into<String>) -> Self {
        Self(message.into().into())
    }
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InternalError({})", self.0)
    }
}

impl std::error::Error for InternalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// A malformed stored identifier, on its way out through the driver.
///
/// Row decoding runs inside a `rusqlite::Result` closure, so the only way to carry a reason
/// out is a driver error. Decoding wraps this type in
/// [`rusqlite::Error::FromSqlConversionFailure`]; [`From<rusqlite::Error>`] recognises it
/// again and rebuilds [`DbError::CorruptIdentifier`]. Doing the recovery in the single
/// `From` impl is what stops corruption degrading to a generic [`DbError::Internal`] on
/// whichever read path happens to hit it first.
#[derive(Debug)]
pub(crate) struct CorruptId {
    /// Column the value was read from.
    pub(crate) column: String,
    /// The value as stored.
    pub(crate) value: String,
}

impl fmt::Display for CorruptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "column `{}` holds `{}`, which is not a UUID",
            self.column, self.value
        )
    }
}

impl std::error::Error for CorruptId {}

/// Everything that can go wrong opening, migrating or writing to a project.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// The project could not be opened.
    #[error("could not open {name}")]
    Open {
        /// File name only — never the full path.
        name: String,
        /// Underlying failure, opaque.
        #[source]
        source: InternalError,
    },

    /// A connection setting did not take effect.
    ///
    /// Treated as fatal rather than ignored: a connection without `foreign_keys = ON`
    /// silently accepts referential corruption, which is far worse than refusing to open.
    #[error("connection setting `{pragma}` is `{actual}` but must be `{expected}`")]
    PragmaRejected {
        /// Name of the pragma that disagreed.
        pragma: &'static str,
        /// What the architecture requires.
        expected: String,
        /// What SQLite actually reported.
        actual: String,
    },

    /// The project was written by a newer Evara than this build understands.
    ///
    /// Forward-only migrations mean there is no safe way to read it. Refusing is
    /// mandatory: a partial read would silently discard data the newer version added.
    #[error(
        "this project uses schema version {found}, but this build of Evara supports up to \
         version {supported}. Update Evara to open it."
    )]
    SchemaTooNew {
        /// Schema version recorded in the project.
        found: u32,
        /// Highest version this binary can apply.
        supported: u32,
    },

    /// Required project metadata is absent.
    #[error("project metadata key `{key}` is missing; the project may be corrupt")]
    MissingMetadata {
        /// The absent key.
        key: &'static str,
    },

    /// Project metadata exists but cannot be interpreted.
    #[error("project metadata key `{key}` holds an unreadable value `{value}`")]
    InvalidMetadata {
        /// The offending key.
        key: &'static str,
        /// The value as stored.
        value: String,
    },

    /// A stored identifier is not a UUID.
    ///
    /// Identifiers are written by this crate and never typed by anyone, so a malformed one
    /// means the file was damaged or edited outside Evara. There is no safe fallback: a nil
    /// identifier would merge unrelated rows, make foreign keys point at nothing and get
    /// written back as if it were real, so the read fails and nothing is interpreted.
    #[error("the project contains a damaged identifier in `{column}`; it may be corrupt")]
    CorruptIdentifier {
        /// Column the value was read from, where the driver could name it.
        column: String,
        /// The value as stored.
        value: String,
    },

    /// An identifier bound for interpolation into SQL was not a plain identifier.
    ///
    /// Trigger generation and the update helper build SQL by interpolating table and
    /// column names, which cannot be parameter-bound. Rejecting anything that is not a
    /// simple identifier is what keeps that safe.
    #[error(
        "`{value}` is not a valid SQL identifier (expected lowercase letters, digits and underscores)"
    )]
    InvalidIdentifier {
        /// The rejected text.
        value: String,
    },

    /// The file exists but is not an Evara project.
    ///
    /// Distinct from a corrupt project: this is "you picked the wrong file", which needs
    /// a completely different message.
    #[error("{name} is not an Evara project")]
    NotAnEvaraProject {
        /// File name only.
        name: String,
    },

    /// A project was expected at a path where no file exists.
    #[error("{name} could not be found; it may have been moved or deleted")]
    ProjectNotFound {
        /// File name only.
        name: String,
    },

    /// The path does not carry the project extension.
    #[error("Evara projects must use the {expected} extension")]
    WrongExtension {
        /// The extension Evara expects.
        expected: &'static str,
    },

    /// A project already exists where a new one was to be created.
    #[error("{name} already exists")]
    ProjectAlreadyExists {
        /// File name only.
        name: String,
    },

    /// No project is currently open.
    #[error("no project is open")]
    NoProjectOpen,

    /// This platform exposes no application data directory.
    #[error("could not locate an application data directory for Evara settings")]
    SettingsDirectoryUnavailable,

    /// Reading or writing the settings directory failed.
    #[error("could not {action}")]
    SettingsIo {
        /// What was being attempted.
        action: &'static str,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// A record was expected but is not there.
    #[error("no {entity} with that identifier")]
    NotFound {
        /// Entity kind, for the message.
        entity: &'static str,
    },

    /// A record cannot be removed because other records still refer to it.
    ///
    /// Carried separately from a raw constraint failure so the interface can say *what*
    /// is in the way rather than "database error".
    #[error("this {entity} is still in use and cannot be deleted")]
    StillReferenced {
        /// Entity kind being deleted.
        entity: &'static str,
    },

    /// A uniqueness rule was broken.
    #[error("another {entity} already uses that {field}")]
    DuplicateValue {
        /// Entity kind.
        entity: &'static str,
        /// Which field collided.
        field: &'static str,
    },

    /// A value failed a domain rule enforced by the schema.
    #[error("{message}")]
    Invalid {
        /// What is wrong, in words fit to show someone.
        message: String,
    },

    /// The project changed between a preview and the confirmation of it.
    ///
    /// Raised by the reviewed materialisation calls when the plan recomputed inside the
    /// write transaction no longer matches the plan the caller was shown. Separate from
    /// [`DbError::Invalid`] because the interface's response is specific: re-read the
    /// plan and ask again, rather than report a bad value.
    ///
    /// The fingerprints are opaque and are carried for diagnostics only; nothing should
    /// branch on their content.
    #[error(
        "the timetable grid changed since it was last previewed, so nothing was applied; \
         review the new plan and confirm again"
    )]
    ReviewRequired {
        /// Fingerprint the caller was shown.
        reviewed: String,
        /// Fingerprint computed at the moment of applying.
        current: String,
    },

    /// Schema migration failed. Nothing was applied.
    #[error("migrating the project failed; no changes were applied")]
    Migration(#[source] InternalError),

    /// Any other database failure.
    #[error("the project database reported an error")]
    Internal(#[source] InternalError),
}

impl DbError {
    /// True when the project cannot be opened because Evara itself is out of date.
    ///
    /// The interface uses this to offer "update Evara" rather than "the file is broken".
    #[must_use]
    pub const fn is_schema_too_new(&self) -> bool {
        matches!(self, Self::SchemaTooNew { .. })
    }

    /// Builds an opaque internal failure from a message.
    ///
    /// Available to consumers so a test can construct one without the SQLite driver.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(InternalError::from_message(message))
    }

    /// Wraps an open failure, keeping only the file name.
    pub(crate) fn opening(path: &std::path::Path, source: rusqlite::Error) -> Self {
        Self::Open {
            name: path.file_name().map_or_else(
                || String::from("the selected file"),
                |name| name.to_string_lossy().into_owned(),
            ),
            source: InternalError::wrap(source),
        }
    }
}

// Conversions live here rather than as `#[from]` on a public field, so the driver types
// stay out of the public API while `?` still works throughout the crate.
impl From<rusqlite::Error> for DbError {
    fn from(source: rusqlite::Error) -> Self {
        if let rusqlite::Error::FromSqlConversionFailure(_, _, ref cause) = source
            && let Some(corrupt) = cause.downcast_ref::<CorruptId>()
        {
            return Self::CorruptIdentifier {
                column: corrupt.column.clone(),
                value: corrupt.value.clone(),
            };
        }
        Self::Internal(InternalError::wrap(source))
    }
}

impl From<refinery::Error> for DbError {
    fn from(source: refinery::Error) -> Self {
        Self::Migration(InternalError::wrap(source))
    }
}

#[cfg(test)]
mod tests {
    use super::{DbError, InternalError};

    #[test]
    fn messages_never_carry_a_path() {
        let error = DbError::opening(
            std::path::Path::new("/home/someone/Private/secret school.evaraproj"),
            rusqlite::Error::InvalidQuery,
        );
        let message = error.to_string();
        assert!(message.contains("secret school.evaraproj"), "{message}");
        assert!(!message.contains("/home/"), "leaked a directory: {message}");
        assert!(
            !message.contains("Private"),
            "leaked a directory: {message}"
        );
    }

    #[test]
    fn internal_failures_describe_themselves_generically() {
        let error = DbError::from(rusqlite::Error::InvalidQuery);
        assert_eq!(error.to_string(), "the project database reported an error");
    }

    #[test]
    fn an_internal_error_can_be_built_without_the_driver() {
        // This is what lets `evara-desktop` test error mapping with no rusqlite
        // dependency of its own.
        let error = DbError::Open {
            name: "school.evaraproj".to_owned(),
            source: InternalError::from_message("disk on fire"),
        };
        assert_eq!(error.to_string(), "could not open school.evaraproj");
    }
}
