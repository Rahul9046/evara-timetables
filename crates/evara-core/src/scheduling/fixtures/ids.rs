//! Deterministic, sortable identifiers for fixture generation.
//!
//! Built on [`Rng`](crate::scheduling::rng::Rng), which moved down into the shipped domain in
//! M6 because the solver needs the same generator. This half stays here: a fixture mints
//! identifiers and nothing in the product does, so an identifier generator in `evara-core`'s
//! shipped surface would be a tool looking for a user.

use uuid::{Builder, Uuid};

use crate::scheduling::rng::Rng;

/// A source of deterministic, sortable, UUIDv7-shaped identifiers.
///
/// # Why these are version 7 and not version 4
///
/// `CLAUDE.md`: *"IDs: UUIDv7, stored as `TEXT`, stable across export and import. Never
/// renumber."* A fixture whose identifiers were v4 would be a fixture whose rows could not
/// be inserted into a project without being rewritten, and the sortability a v7 gives — the
/// timestamp is the high 48 bits — is what makes a generated fixture's output order stable
/// and readable.
///
/// # Why the timestamp is synthetic
///
/// A real v7 reads the clock, and a fixture that read the clock would produce different
/// identifiers on every run, which would break every determinism assertion M2 exists to
/// make. So the timestamp starts from a fixed epoch offset derived from the seed and
/// advances by one millisecond per identifier. The result is a well-formed v7 — correct
/// version and variant bits, monotonically increasing — that is a pure function of the seed.
///
/// The fixed base is deliberately in the past and is not a real event; it is a synthetic
/// number, like every other value in a fixture (`CLAUDE.md` rule 3).
#[derive(Debug, Clone)]
pub struct IdGenerator {
    rng: Rng,
    millis: u64,
}

/// Unix milliseconds the synthetic timestamps start from: 2020-01-01T00:00:00Z.
///
/// Any fixed value would do. A round one in the past makes a generated identifier obviously
/// synthetic to anyone who decodes it.
const SYNTHETIC_EPOCH_MS: u64 = 1_577_836_800_000;

impl IdGenerator {
    /// Starts a generator from a seed.
    ///
    /// The seed perturbs both the random bits and the starting millisecond, so two seeds
    /// produce entirely disjoint identifier sets rather than the same identifiers in a
    /// different order.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            // Keep the offset small enough that the timestamps stay in the recent past and
            // remain plausible as v7 values. 2^20 ms is about twelve days.
            millis: SYNTHETIC_EPOCH_MS + (seed & 0xF_FFFF),
        }
    }

    /// The next identifier.
    ///
    /// Monotonically increasing in its timestamp, so identifiers sort in creation order the
    /// way real UUIDv7s do.
    ///
    /// Named `next_id` rather than `next` so it is not mistaken for — or shadowed by —
    /// [`Iterator::next`]: this generator is infinite and fallible in neither direction, and
    /// implementing `Iterator` would invite `.take(n).collect()` on something whose draw
    /// count is part of a fixture's determinism contract.
    pub fn next_id(&mut self) -> Uuid {
        let mut random = [0u8; 10];
        let first = self.rng.next_u64().to_be_bytes();
        let second = self.rng.next_u64().to_be_bytes();
        random[..8].copy_from_slice(&first);
        random[8..].copy_from_slice(&second[..2]);

        let id = Builder::from_unix_timestamp_millis(self.millis, &random).into_uuid();
        self.millis += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use uuid::Version;

    use super::IdGenerator;

    #[test]
    fn generated_identifiers_are_well_formed_version_seven() {
        // CLAUDE.md requires UUIDv7. A fixture whose ids were a different version could not
        // be inserted into a project without being rewritten.
        let mut ids = IdGenerator::new(5);
        for _ in 0..64 {
            let id = ids.next_id();
            assert_eq!(id.get_version(), Some(Version::SortRand));
            assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
            assert_ne!(id, uuid::Uuid::nil());
        }
    }

    #[test]
    fn generated_identifiers_are_unique_and_sorted_by_creation_order() {
        let mut ids = IdGenerator::new(5);
        let generated: Vec<uuid::Uuid> = (0..512).map(|_| ids.next_id()).collect();

        let distinct: BTreeSet<uuid::Uuid> = generated.iter().copied().collect();
        assert_eq!(distinct.len(), generated.len(), "an identifier repeated");

        let mut sorted = generated.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted, generated,
            "v7 identifiers sort in creation order; the synthetic timestamps must too"
        );
    }

    #[test]
    fn the_same_seed_produces_the_same_identifiers() {
        let a: Vec<uuid::Uuid> = {
            let mut ids = IdGenerator::new(99);
            (0..16).map(|_| ids.next_id()).collect()
        };
        let b: Vec<uuid::Uuid> = {
            let mut ids = IdGenerator::new(99);
            (0..16).map(|_| ids.next_id()).collect()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_produce_disjoint_identifier_sets() {
        let a: BTreeSet<uuid::Uuid> = {
            let mut ids = IdGenerator::new(1);
            (0..64).map(|_| ids.next_id()).collect()
        };
        let b: BTreeSet<uuid::Uuid> = {
            let mut ids = IdGenerator::new(2);
            (0..64).map(|_| ids.next_id()).collect()
        };
        assert!(
            a.is_disjoint(&b),
            "two seeds must not mint the same identifiers"
        );
    }
}
