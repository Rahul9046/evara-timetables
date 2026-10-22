//! The combined pre-solve report: what the loader found, what integrity found, and what M3
//! found.
//!
//! # Three stages, one report
//!
//! `PRE-SOLVE-VALIDATION.md` §15 sketches the call sequence; this is it, named once so no
//! caller assembles its own version:
//!
//! ```text
//! load ──► Problem::integrity ──► validate ──► combine
//!  M4            M2                  M3          here
//! ```
//!
//! # Why integrity is reported separately and still blocks
//!
//! An [`IntegrityError`] is a **loader or fixture bug**, not a data state — its own
//! documentation says so, and ADR 0036 §4 refused to report one as a blocking diagnostic
//! because *"the remedy is not a field the user can fill in — it is a code fix"*. A user told
//! to "correct this section's bell-schedule resolution" has been handed somebody else's
//! defect.
//!
//! So it keeps its own channel. But it must still stop a solve, because a problem with a
//! dangling index is not a problem anybody should search over: [`AssemblyReport::may_solve`]
//! is `false` whenever integrity is non-empty, whatever the diagnostics say. A caller that
//! read only the severities would otherwise be told a broken problem is ready.

use std::fmt::Write as _;

use evara_core::scheduling::{
    Diagnostic, DiagnosticReport, IntegrityError, Severity, ValidationSummary, validate,
};

use super::{Assembly, AssemblyStats, Completeness, SchedulingSelection};

/// Everything that is wrong with one school's scheduling inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyReport {
    /// Which school, cycle and term this is about.
    pub selection: SchedulingSelection,
    /// What the school is called, so a printed report can name it.
    pub school: String,
    /// Ways the compiled problem refers to something it does not contain.
    ///
    /// Empty for a sound load. Non-empty means a defect in this loader, and blocks a solve
    /// without being shown to the user as something to fix.
    pub integrity: Vec<IntegrityError>,
    /// The loader's findings and the validator's, merged.
    pub findings: DiagnosticReport,
    /// The counts a Generate screen shows above the findings.
    ///
    /// Computed over the **merged** report, so a loader-owned blocking finding is counted in
    /// `blocking` and `may_solve` reflects it.
    pub summary: ValidationSummary,
    /// Whether every stored restriction in scope is being enforced.
    pub completeness: Completeness,
    /// What was worth counting but is not a finding.
    pub stats: AssemblyStats,
}

impl AssemblyReport {
    /// Checks integrity, runs M3, and merges the two sets of findings.
    #[must_use]
    pub fn of(assembly: &Assembly) -> Self {
        let integrity = assembly.integrity();
        let findings = merge(&assembly.findings, &validate(&assembly.problem));
        let summary = ValidationSummary::of(&assembly.problem, &findings);
        Self {
            selection: assembly.selection,
            school: assembly.problem.context.school.name.clone(),
            integrity,
            findings,
            summary,
            completeness: assembly.completeness,
            stats: assembly.stats,
        }
    }

    /// Whether a solve may proceed.
    ///
    /// **The question a caller asks**, rather than one it derives from severities — the rule
    /// `PRE-SOLVE-VALIDATION.md` §15 states for M8 and which applies identically here. Two
    /// things make it `false`: a blocking finding, and a non-empty integrity result.
    ///
    /// A warning or an informational finding never blocks on its own. §3.2 reserves a
    /// refusal for an input whose remedy is a field the user can fill in, and a warning is by
    /// definition not one.
    ///
    /// `true` is **not** a claim that a timetable exists. Nothing in Evara can make that
    /// claim: there is no solver, and `evara_core::lessons`' rule binds this type as it binds
    /// the modules behind it — never report that a demand *can* be scheduled.
    #[must_use]
    pub fn may_solve(&self) -> bool {
        self.integrity.is_empty() && self.summary.may_solve()
    }

    /// Whether anything at all was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.integrity.is_empty() && self.summary.is_clean() && self.completeness.is_complete()
    }

    /// The findings of one severity, in report order.
    pub fn of_severity(&self, severity: Severity) -> impl Iterator<Item = &Diagnostic> + '_ {
        self.findings.of_severity(severity)
    }

    /// A one-line summary of what was assembled and what blocks it.
    ///
    /// Deliberately never says a timetable is feasible. The wording matches `evara validate`'s
    /// so the fixture and project paths describe their results identically.
    #[must_use]
    pub fn headline(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{}: {} blocking, {} warning, {} informational",
            self.school, self.summary.blocking, self.summary.warnings, self.summary.informational
        );
        if !self.integrity.is_empty() {
            let _ = write!(
                out,
                "; {} internal inconsistenc{}",
                self.integrity.len(),
                if self.integrity.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
        }
        if let Completeness::RestrictionsNotEnforced { rules } = self.completeness {
            let _ = write!(out, "; {rules} stored restriction(s) not enforced");
        }
        out
    }
}

/// Merges the loader's findings with the validator's.
///
/// # By severity first, then by stage
///
/// Not a concatenation, and the reason is a property the merged report has to keep.
/// `PRE-SOLVE-VALIDATION.md` §4 orders a report so that severity is non-decreasing down it,
/// and M3 has a test asserting exactly that. Appending the loader's findings — which span all
/// three severities — to either end would break it, and a reader would meet an informational
/// note above a blocking refusal.
///
/// So the two streams are interleaved per severity, loader first. Within a severity that puts
/// causes before symptoms, which is §4's own justification for its five stages: a stored row
/// that should not exist is upstream of anything a compiled problem can say about itself.
///
/// # Nothing is deduplicated, and nothing needs to be
///
/// The two producers' code sets are disjoint by design — ADR 0037 §1 — so no finding can
/// arrive twice with two wordings. A test asserts the disjointness over the whole
/// `DiagnosticCode` enum rather than trusting it, because the cheapest way for this to go
/// wrong is for a later milestone to implement a loader code in the validator too.
///
/// Distinct findings sharing a code are kept: `PRE-SOLVE-VALIDATION.md` §5 is explicit that
/// distinct actionable problems are never suppressed for sharing one.
fn merge(loader: &[Diagnostic], validator: &DiagnosticReport) -> DiagnosticReport {
    let mut merged = DiagnosticReport::empty();
    for severity in [Severity::Blocking, Severity::Warning, Severity::Info] {
        for finding in loader.iter().filter(|f| f.severity() == severity) {
            merged.push(finding.clone());
        }
        for finding in validator.of_severity(severity) {
            merged.push(finding.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use evara_core::scheduling::{
        Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef, Severity,
    };
    use uuid::Uuid;

    use super::merge;
    use crate::scheduling::LOADER_DIAGNOSTICS;

    fn finding(code: DiagnosticCode, label: &str) -> Diagnostic {
        Diagnostic::new(
            code,
            EntityRef::new(EntityKind::School, Uuid::from_u128(1), label),
            "something",
        )
    }

    #[test]
    fn severity_is_non_decreasing_down_the_merged_report() {
        // The property M3 asserts over its own report, which a concatenation would lose.
        let loader = vec![
            finding(DiagnosticCode::AvailabilityEntityKindNotScheduled, "info"),
            finding(DiagnosticCode::TeacherNotEligibleForCampus, "blocking"),
            finding(DiagnosticCode::OrphanAvailabilityReference, "warning"),
        ];
        let mut validator = DiagnosticReport::empty();
        validator.push(finding(DiagnosticCode::NoGridMaterialised, "v-blocking"));
        validator.push(finding(DiagnosticCode::SectionWithoutTeacher, "v-warning"));
        validator.push(finding(DiagnosticCode::DeclaredPeriodsDisagree, "v-info"));

        let merged = merge(&loader, &validator);
        let severities: Vec<Severity> = merged.findings.iter().map(Diagnostic::severity).collect();
        let mut sorted = severities.clone();
        sorted.sort_unstable();
        assert_eq!(
            severities, sorted,
            "severity must not decrease down a report"
        );
    }

    #[test]
    fn a_loader_finding_precedes_a_validator_finding_of_the_same_severity() {
        // Causes before symptoms: a stored row that should not exist is upstream of
        // anything the compiled problem can say about itself.
        let loader = vec![finding(
            DiagnosticCode::TeacherNotEligibleForCampus,
            "loader",
        )];
        let mut validator = DiagnosticReport::empty();
        validator.push(finding(DiagnosticCode::NoGridMaterialised, "validator"));

        let merged = merge(&loader, &validator);
        let labels: Vec<&str> = merged
            .findings
            .iter()
            .map(|f| f.subject.label.as_str())
            .collect();
        assert_eq!(labels, ["loader", "validator"]);
    }

    #[test]
    fn merging_keeps_every_finding() {
        let loader = vec![finding(DiagnosticCode::OrphanAvailabilityReference, "a")];
        let mut validator = DiagnosticReport::empty();
        validator.push(finding(DiagnosticCode::SectionWithoutTeacher, "b"));
        validator.push(finding(DiagnosticCode::SectionWithoutTeacher, "c"));

        // Two findings sharing a code are two actionable problems, and §5 forbids
        // suppressing either.
        assert_eq!(merge(&loader, &validator).findings.len(), 3);
    }

    #[test]
    fn merging_nothing_with_nothing_is_empty() {
        assert_eq!(
            merge(&[], &DiagnosticReport::empty()),
            DiagnosticReport::empty()
        );
    }

    #[test]
    fn the_loader_claims_only_the_codes_adr_0037_gave_it() {
        // The ownership contract, asserted rather than trusted.
        //
        // **Four since M5, not three.** `group-overlap-contradiction` is the addition, and
        // ADR 0037's own table is why it belongs here rather than being a fourth producer:
        // the code was assigned to M5 because *"a `Problem` carries a member count, so M3
        // could not detect it even if it owned the rule"*, and M5's rule runs where the
        // member identities are — in this loader, at relation-compile time. A finding about
        // stored rows is the loader's, which is precisely ADR 0037's criterion.
        assert_eq!(LOADER_DIAGNOSTICS.len(), 4);
        for code in LOADER_DIAGNOSTICS {
            assert!(
                matches!(
                    code,
                    DiagnosticCode::TeacherNotEligibleForCampus
                        | DiagnosticCode::OrphanAvailabilityReference
                        | DiagnosticCode::AvailabilityEntityKindNotScheduled
                        | DiagnosticCode::GroupOverlapContradiction
                ),
                "{} is not a code the loader owns",
                code.key()
            );
        }
    }
}
