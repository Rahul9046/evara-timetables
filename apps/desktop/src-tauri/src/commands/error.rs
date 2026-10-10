//! The error contract for every School Setup command.
//!
//! Same shape as [`super::project::ProjectError`], for the same reason: the interface
//! branches on `kind` rather than reading English out of a message. A setup screen has to
//! react differently to "that code is already taken" (highlight the field), "this campus
//! still has buildings" (explain what is in the way) and "the grid moved while you were
//! looking at it" (re-read and ask again), and it cannot do that by pattern-matching
//! prose.
//!
//! # Wording lives here, once
//!
//! Every variant carries `message`, already fit to show someone. The alternative —
//! assembling sentences in React from an error code — ends with the same rule worded three
//! different ways on three screens.
//!
//! # Nothing from the engine crosses this boundary
//!
//! `DbError::Internal` and `DbError::Migration` carry SQLite detail in their source
//! chain, so their wording is **replaced** rather than forwarded. The Phase 1B tests that
//! pin this down for the project lifecycle have counterparts here.

use evara_db::DbError;
use serde::Serialize;

/// A School Setup failure, in a form the interface can act on.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum SetupError {
    /// A command needing an open project ran without one.
    ///
    /// Reachable without a bug: a screen can still be mounted when the project closes.
    NoProjectOpen {
        /// Message to show.
        message: String,
    },

    /// The record is not there — deleted in another window, or already gone.
    NotFound {
        /// What kind of record was expected, for the message.
        entity: String,
        /// Message to show.
        message: String,
    },

    /// A delete was refused because other records still depend on this one.
    ///
    /// The `ON DELETE RESTRICT` case. The message says what to do about it, because
    /// "this campus is still in use" on its own leaves the user with no next step.
    StillReferenced {
        /// What could not be deleted.
        entity: String,
        /// Message to show.
        message: String,
    },

    /// A uniqueness rule was broken.
    ///
    /// Carries `field` so the form can mark the offending input rather than showing a
    /// banner and leaving the user to guess which box is wrong.
    Duplicate {
        /// What kind of record collided.
        entity: String,
        /// Which field collided: `code`, `name`, `label`, `position` or `date`.
        field: String,
        /// Message to show.
        message: String,
    },

    /// A value failed a rule the project enforces.
    Invalid {
        /// Message to show.
        message: String,
    },

    /// The grid changed between the preview and the confirmation.
    ///
    /// Distinct from [`SetupError::Invalid`] because the interface's response is
    /// specific: re-read the plan and ask the user again. Nothing was written.
    ReviewRequired {
        /// Message to show.
        message: String,
    },

    /// The project file holds a value that cannot be read.
    ///
    /// Separated from [`SetupError::Failed`] because the advice differs: this is damage,
    /// not a transient problem, and retrying will not help.
    CorruptProject {
        /// Message to show.
        message: String,
    },

    /// Anything else.
    Failed {
        /// Message to show.
        message: String,
    },
}

impl From<DbError> for SetupError {
    fn from(error: DbError) -> Self {
        let message = error.to_string();
        match error {
            DbError::NoProjectOpen => Self::NoProjectOpen {
                message: "No project is open. Open or create a project first.".to_owned(),
            },
            DbError::NotFound { entity } => Self::NotFound {
                entity: entity.to_owned(),
                message: format!(
                    "That {entity} no longer exists. It may have been deleted already."
                ),
            },
            DbError::StillReferenced { entity } => Self::StillReferenced {
                entity: entity.to_owned(),
                message: format!(
                    "This {entity} cannot be deleted while other records still depend on it. \
                     Remove or reassign those records first, then try again."
                ),
            },
            DbError::DuplicateValue { entity, field } => Self::Duplicate {
                entity: entity.to_owned(),
                field: field.to_owned(),
                message: format!("Another {entity} already uses that {field}."),
            },
            DbError::Invalid { message } => Self::Invalid { message },
            DbError::ReviewRequired { .. } => Self::ReviewRequired { message },
            DbError::CorruptIdentifier { .. } | DbError::InvalidMetadata { .. } => {
                Self::CorruptProject {
                    message: "This project contains a damaged value and cannot be read safely. \
                              Restore a backup if you have one."
                        .to_owned(),
                }
            }
            // These carry engine detail — SQL text, file paths — in their source chain,
            // so the wording is replaced rather than forwarded.
            DbError::Internal(_) | DbError::Migration(_) | DbError::Open { .. } => Self::Failed {
                message: "The project could not be updated. It may be damaged or in use by \
                          another program."
                    .to_owned(),
            },
            // `InvalidIdentifier` is a programming error in this crate's own SQL, not
            // something a user can cause or fix; it must not surface as advice.
            DbError::InvalidIdentifier { .. } => Self::Failed {
                message: "Evara could not complete that change. Please report this.".to_owned(),
            },
            _ => Self::Failed { message },
        }
    }
}

/// Result type every setup command returns.
pub type SetupResult<T> = Result<T, SetupError>;

#[cfg(test)]
mod tests {
    use super::SetupError;
    use evara_db::DbError;

    fn json(error: DbError) -> serde_json::Value {
        serde_json::to_value(SetupError::from(error)).expect("serialise")
    }

    fn kind_of(error: DbError) -> String {
        json(error)["kind"].as_str().expect("tagged").to_owned()
    }

    #[test]
    fn every_state_the_interface_must_distinguish_has_its_own_kind() {
        assert_eq!(kind_of(DbError::NoProjectOpen), "noProjectOpen");
        assert_eq!(kind_of(DbError::NotFound { entity: "campus" }), "notFound");
        assert_eq!(
            kind_of(DbError::StillReferenced { entity: "campus" }),
            "stillReferenced"
        );
        assert_eq!(
            kind_of(DbError::DuplicateValue {
                entity: "campus",
                field: "code"
            }),
            "duplicate"
        );
        assert_eq!(
            kind_of(DbError::Invalid {
                message: "no".to_owned()
            }),
            "invalid"
        );
        assert_eq!(
            kind_of(DbError::ReviewRequired {
                reviewed: "aaaa".to_owned(),
                current: "bbbb".to_owned()
            }),
            "reviewRequired"
        );
        assert_eq!(
            kind_of(DbError::CorruptIdentifier {
                column: "campus_id".to_owned(),
                value: "not-a-uuid".to_owned()
            }),
            "corruptProject"
        );
        assert_eq!(
            kind_of(DbError::internal("near \"SELCT\"")),
            "failed",
            "an engine failure is not a user-actionable state"
        );
    }

    #[test]
    fn a_restrict_failure_tells_the_user_what_to_do_about_it() {
        let value = json(DbError::StillReferenced { entity: "campus" });
        let message = value["message"].as_str().expect("message");

        assert_eq!(value["entity"], "campus");
        assert!(
            message.contains("Remove or reassign"),
            "a refusal with no next step is not an explanation: {message}"
        );
    }

    #[test]
    fn a_duplicate_names_the_field_so_the_form_can_mark_it() {
        let value = json(DbError::DuplicateValue {
            entity: "cycle day",
            field: "position",
        });
        assert_eq!(value["field"], "position");
        assert_eq!(value["entity"], "cycle day");
    }

    #[test]
    fn a_stale_plan_is_not_reported_as_a_bad_value() {
        let value = json(DbError::ReviewRequired {
            reviewed: "1111111111111111".to_owned(),
            current: "2222222222222222".to_owned(),
        });
        let message = value["message"].as_str().expect("message");

        assert_eq!(value["kind"], "reviewRequired");
        assert!(
            message.contains("nothing was applied"),
            "the user must be told the grid is untouched: {message}"
        );
        assert!(
            !message.contains("1111111111111111"),
            "the fingerprint is a diagnostic, not something to show: {message}"
        );
    }

    /// Built without `rusqlite`, which is the point: this crate must not need the SQLite
    /// driver even to construct an error in a test. See the Phase 1B counterpart.
    #[test]
    fn engine_detail_and_paths_never_reach_the_interface() {
        let value = json(DbError::Open {
            name: "secret school.evaraproj".to_owned(),
            source: evara_db::InternalError::from_message(
                "unable to open database file: /home/someone/Private/secret school.evaraproj",
            ),
        });
        let message = value["message"].as_str().expect("message");

        assert_eq!(value["kind"], "failed");
        assert!(
            !message.contains("secret school"),
            "leaked a name: {message}"
        );
        assert!(!message.contains("/home/"), "leaked a path: {message}");
        assert!(
            !message.contains("database file"),
            "leaked engine detail: {message}"
        );
    }

    #[test]
    fn a_sql_mistake_of_our_own_is_not_dressed_up_as_user_advice() {
        let value = json(DbError::InvalidIdentifier {
            value: "room; DROP TABLE school".to_owned(),
        });
        let message = value["message"].as_str().expect("message");

        assert_eq!(value["kind"], "failed");
        assert!(
            !message.contains("DROP TABLE"),
            "echoing the offending identifier back is not useful: {message}"
        );
    }
}
