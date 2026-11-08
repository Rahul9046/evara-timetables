//! Cancellation: one trait, asked between steps, owned by nobody in particular.
//!
//! # Why this lives in `evara-core`
//!
//! It began in `evara-solver`, where the only long-running operation was a solve. M10 adds
//! a second — copying a project, which for a large school is the one place a user will
//! reach for a Cancel button outside generation — and the copy is taken by `evara-db`,
//! which **may not depend on `evara-solver`** (`ARCHITECTURE.md` §3 rule 4: `evara-db`
//! knows SQL and depends on `evara-core` alone).
//!
//! So two crates have to agree on the vocabulary, which is exactly `ARCHITECTURE.md` §3
//! rule 1's criterion for a type belonging here. `evara-solver` re-exports both names, so
//! every existing `use evara_solver::Cancel` keeps compiling and there is still only one
//! trait in the workspace.
//!
//! # Why the clock is on the caller's side
//!
//! Nothing in this module reads a clock, and deliberately: `CLAUDE.md` forbids an
//! `Instant::now()` inside the solver's decisions, because every execution limit is a count
//! so that the same seed stops in the same place on every machine. A wall clock reaches an
//! engine only *through* this trait, as the caller's own decision to stop — which is why
//! `Termination::Cancelled` is the one outcome this workspace documents as not
//! reproducible.
//!
//! The same reasoning carries to a snapshot, for a different reason: a copy's progress
//! depends on page count, disk and platform, so a time limit inside it would make the
//! engine's behaviour depend on the machine. The caller decides; the engine only asks.

/// Whether the caller wants the operation to stop.
///
/// Asked between discrete steps — construction steps in a solve, page batches in a
/// snapshot — never mid-statement.
pub trait Cancel {
    /// Whether to stop now.
    ///
    /// Called often, so it must be cheap. An implementation that read a file or took a lock
    /// would make the operation's speed depend on something other than the work.
    fn is_cancelled(&self) -> bool;
}

/// A cancellation that never fires.
///
/// What the headless CLI and every test pass. Named rather than anonymous so a caller that
/// genuinely wants no cancellation says so, instead of an `Option<&dyn Cancel>` that reads as
/// an oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NeverCancel;

impl Cancel for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Blanket implementation so a `&C` is accepted wherever a `C` is.
///
/// Without it a caller holding a `&NeverCancel` has to re-borrow at every call site. The
/// trait is read-only, so this cannot change behaviour.
impl<C: Cancel + ?Sized> Cancel for &C {
    fn is_cancelled(&self) -> bool {
        (**self).is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cancel, NeverCancel};

    /// A cancellation that fires once it is told to, for tests that need one.
    struct Switch(std::cell::Cell<bool>);

    impl Cancel for Switch {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }

    #[test]
    fn never_cancel_never_does() {
        assert!(!NeverCancel.is_cancelled());
        // Through the trait object too, which is the shape callers actually pass.
        let as_object: &dyn Cancel = &NeverCancel;
        assert!(!as_object.is_cancelled());
    }

    #[test]
    fn a_reference_is_a_cancellation_too() {
        fn ask(cancel: impl Cancel) -> bool {
            cancel.is_cancelled()
        }
        let switch = Switch(std::cell::Cell::new(false));
        assert!(!ask(&switch));
        switch.0.set(true);
        assert!(ask(&switch), "the blanket impl must observe the change");
    }

    #[test]
    fn a_trait_object_is_usable() {
        // The shape `Database::snapshot_into` takes, asserted so a later change to the
        // trait cannot quietly make it un-object-safe.
        let cancel: &dyn Cancel = &NeverCancel;
        assert!(!cancel.is_cancelled());
    }
}
