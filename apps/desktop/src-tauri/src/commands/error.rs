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
// `rename_all_fields` as well as `rename_all`: the variant names are the `kind` tags and
// the field names cross as JSON keys, and `CLAUDE.md` requires camelCase on the wire for
// both. Without the second, `AlreadySaved`'s `timetable_id` would reach the interface in
// snake_case — the one shape a generated type cannot warn about, because it would be
// generated wrong too.
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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

    /// The record named belongs to a different school.
    ///
    /// Distinct from [`SetupError::NotFound`] on purpose: the record exists, and saying it
    /// does not would send the user looking for something that is there. Distinct from
    /// [`SetupError::Failed`] because it is not a failure — it is a boundary working, and
    /// the next step is to switch school rather than to retry.
    ///
    /// Reachable without a bug: a confirmation left open while the active school changed.
    WrongSchool {
        /// What kind of record it was.
        entity: String,
        /// Message to show.
        message: String,
    },

    /// Somebody else changed the record after this window read it.
    ///
    /// The optimistic-concurrency refusal, new in Stage B. **Nothing was written**, which
    /// is the first thing the message says, because the one unacceptable outcome is a
    /// screen that looks saved and is not.
    ///
    /// Its own kind rather than [`SetupError::Invalid`] because the response is specific
    /// and is not an apology: show the user what the record says now, keep what they
    /// typed, and let them decide. Distinct from [`SetupError::NotFound`] because the
    /// record is still there — telling somebody it has gone would send them looking for
    /// something that exists. Distinct from [`SetupError::ReviewRequired`], which is the
    /// grid's version of the same idea and is about a recomputed plan rather than one
    /// row's revision.
    ///
    /// The revisions are carried for diagnostics and for a test to assert. Nothing should
    /// branch on the gap between them: one behind and twelve behind want the same answer.
    Conflict {
        /// What kind of record it was.
        entity: String,
        /// Revision this window was holding.
        expected_rev: i64,
        /// Revision the record is actually at.
        actual_rev: i64,
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

    /// A scheduling input moved between generating a timetable and saving it.
    ///
    /// Its own kind, and **not** [`SetupError::ReviewRequired`], because the two remedies
    /// are different and only one of them is available here. `ReviewRequired` means *look at
    /// the recomputed plan and confirm it*; this means **generate again** — there is no
    /// plan to re-read, because the arrangement was built from inputs that have changed.
    ///
    /// The refusal is correct and the message has to make that obvious.
    /// [TIMETABLE-PERSISTENCE.md](../../../../docs/TIMETABLE-PERSISTENCE.md) §4: nothing
    /// referential breaks when an availability rule moves after generation, so nothing else
    /// would catch it, and the result would be *"a plausible, wrong timetable"*. A screen
    /// that showed a raw error rather than offering the remedy would make a working safety
    /// check look like a bug — which §13 item 4 names as M8's obligation to avoid.
    InputsChanged {
        /// Message to show.
        message: String,
    },

    /// The result of a generation is not a timetable, so there is nothing to save.
    ///
    /// Two cases, and the message says which, because the user's next step differs:
    ///
    /// - **the inputs were refused** — nothing was attempted, and the remedy is the
    ///   validation findings;
    /// - **the engine's own final check failed** — the reference scorer rejected what the
    ///   constructor built. That is a **defect in Evara**, is reported as one, and is
    ///   deliberately not saveable. `TIMETABLE-PERSISTENCE.md` §9 and `generate`'s own
    ///   documentation both refuse to let it be returned as a success.
    NotSaveable {
        /// Message to show.
        message: String,
    },

    /// A write was attempted against a project opened for inspection.
    ///
    /// Separate from [`SetupError::Failed`] because it is a boundary working rather than a
    /// failure: the file was opened read-only on purpose, and the next step is to open it
    /// normally rather than to retry.
    ReadOnlyProject {
        /// Message to show.
        message: String,
    },

    /// This generation has already been saved.
    ///
    /// Carries the timetable so the interface can offer to **open it** rather than reporting
    /// an error for something that already succeeded. The case a second click reaches, and
    /// the reason the session's save is claimed by a compare-and-swap rather than guarded by
    /// a flag.
    AlreadySaved {
        /// The timetable that was written.
        timetable_id: String,
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
                message: "No campus is open. Open or create one first.".to_owned(),
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
            DbError::Conflict {
                entity,
                expected,
                actual,
            } => Self::Conflict {
                entity: entity.to_owned(),
                expected_rev: expected,
                actual_rev: actual,
                // Worded once, here, because every screen's answer is the same and three
                // screens phrasing it three ways would be three different promises about
                // whether anything was saved.
                message: format!(
                    "This {entity} has changed since you opened it, so nothing was saved. \
                     Reload the latest version before saving."
                ),
            },
            DbError::DuplicateValue { entity, field } => Self::Duplicate {
                entity: entity.to_owned(),
                field: field.to_owned(),
                message: format!("Another {entity} already uses that {field}."),
            },
            DbError::WrongSchool { entity } => Self::WrongSchool {
                entity: entity.to_owned(),
                message: format!(
                    "That {entity} belongs to a different school, so nothing was changed.                      Switch to the school it belongs to and try again."
                ),
            },
            DbError::Invalid { message } => Self::Invalid { message },
            DbError::ReviewRequired { .. } => Self::ReviewRequired { message },
            // The remedy is *generate again*, and it is named rather than implied. See the
            // variant's own documentation for why this is not `ReviewRequired`.
            DbError::InputsChanged { .. } => Self::InputsChanged {
                message: "A scheduling input changed since this timetable was generated, so \
                          saving it could store a wrong timetable. Nothing was saved. \
                          Generate again."
                    .to_owned(),
            },
            // `reason` is the engine's own, and the two cases need different things said
            // about them, so it is forwarded rather than replaced.
            DbError::NotSaveable { reason } => Self::NotSaveable {
                message: format!("Nothing was saved: {reason}."),
            },
            DbError::ReadOnlyProject => Self::ReadOnlyProject {
                message: "This campus file was opened for inspection only, so nothing can \
                          be saved to it."
                    .to_owned(),
            },
            DbError::CorruptIdentifier { .. } | DbError::InvalidMetadata { .. } => {
                Self::CorruptProject {
                    message: "This campus contains a damaged value and cannot be read \
                              safely. Restore a backup if you have one."
                        .to_owned(),
                }
            }
            // These carry engine detail — SQL text, file paths — in their source chain,
            // so the wording is replaced rather than forwarded.
            DbError::Internal(_) | DbError::Migration(_) | DbError::Open { .. } => Self::Failed {
                message: "This campus could not be updated. It may be damaged or in use by \
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
        let value = json(DbError::WrongSchool { entity: "teacher" });
        assert_eq!(value["kind"], "wrongSchool");
        assert_eq!(value["entity"], "teacher");
        let message = value["message"].as_str().expect("a message");
        assert!(
            message.contains("different school") && message.contains("nothing was changed"),
            "a wrong-school refusal must say the boundary held and that nothing changed:              {message}"
        );

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
