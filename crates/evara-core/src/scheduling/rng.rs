//! Deterministic randomness: one generator, shared by the fixture catalogue and the solver.
//!
//! # Why not a dependency
//!
//! The workspace has no `rand`, and adding one for this would be a poor trade. Two
//! properties are needed and neither is cryptographic: the same seed must produce the same
//! bytes **on every platform and every toolchain version**, and the algorithm must be stable
//! forever, because a generator whose output changed under a dependency bump would both
//! invalidate every committed solver baseline and silently change the timetable a stored
//! `solve_run.seed` reproduces.
//!
//! A general-purpose RNG crate gives neither guarantee by contract — `rand`'s generators are
//! explicitly allowed to change their output across major versions. Twenty lines of
//! SplitMix64, written down here, gives both. It is also one fewer licence to review under
//! `CLAUDE.md` rule 4.
//!
//! # Why it is here rather than under `fixtures`
//!
//! It was fixture infrastructure until M6, which needs the same generator in the **shipped**
//! product: `SOLVER.md` §5 makes a seeded solve's reproducibility non-negotiable, and
//! `evara-solver` cannot reach a module behind the `fixtures` feature the desktop binary does
//! not enable. Copying twenty lines into the solver would have given two generators that
//! could drift apart and two seeds that meant different things, so the one implementation
//! moved down instead. [`fixtures::ids`](super::fixtures::ids) still owns the identifier
//! generator built on it, which is fixture-only and stays there.
//!
//! # SplitMix64
//!
//! The finaliser from Steele, Lea and Flood's *Fast Splittable Pseudorandom Number
//! Generators* (2014), as used for `java.util.SplittableRandom` and as the standard seeder
//! for the xoshiro family. Public-domain algorithm, no attribution encumbrance, and it
//! passes the usual statistical batteries at the quality a synthetic school and a candidate
//! ordering need. It is **not** suitable for anything security-bearing, and nothing here is.
//!
//! # Determinism is the whole point
//!
//! Nothing in this module reads a clock, an environment variable or an address. `CLAUDE.md`
//! forbids `Instant::now()` and unseeded randomness in the solver's decisions, and a
//! generator whose output depended on the time would make the reproducibility guarantee
//! untestable — a baseline would drift under a fixture that was supposed to be fixed.

/// A deterministic pseudo-random number generator.
///
/// Reproducible across platforms and stable across toolchain versions, because the algorithm
/// is written here rather than depended upon. See this module's header.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Starts a generator from a seed.
    ///
    /// Every seed is valid, including zero: the finaliser's first step adds an odd constant,
    /// so a zero state does not degenerate the way a bare xorshift would.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        // SplitMix64: advance by the golden-ratio odd constant, then finalise with two
        // xor-shift-multiply rounds.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound`, or `0` when `bound` is zero.
    ///
    /// Uses the multiply-shift reduction rather than a modulo, which avoids the modulo bias
    /// without a rejection loop — and a rejection loop would make the number of RNG draws
    /// depend on the values drawn, which is a subtle way to lose reproducibility when a
    /// caller's bounds change.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        let draw = u128::from(self.next_u64());
        usize::try_from((draw * bound as u128) >> 64).unwrap_or(0)
    }

    /// A number in `low..=high`, or `low` when the range is empty or reversed.
    pub fn between(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        let span = usize::try_from(high - low + 1).unwrap_or(1);
        low + i64::try_from(self.below(span)).unwrap_or(0)
    }

    /// Whether an event with probability `numerator / denominator` happens.
    ///
    /// Expressed as a ratio rather than a float so that the decision is exact integer
    /// arithmetic and cannot vary with floating-point rounding between platforms.
    pub fn chance(&mut self, numerator: usize, denominator: usize) -> bool {
        denominator > 0 && self.below(denominator) < numerator
    }

    /// Picks one of `options`, or `None` when it is empty.
    pub fn pick<'a, T>(&mut self, options: &'a [T]) -> Option<&'a T> {
        options.get(self.below(options.len()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::Rng;

    #[test]
    fn the_same_seed_produces_the_same_sequence() {
        let mut first = Rng::new(42);
        let mut second = Rng::new(42);
        let a: Vec<u64> = (0..32).map(|_| first.next_u64()).collect();
        let b: Vec<u64> = (0..32).map(|_| second.next_u64()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge_immediately() {
        // A generator that agreed on its first draws would make two "different" fixtures
        // share their first entities, which is the opposite of distinguishable.
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn a_zero_seed_does_not_degenerate() {
        let mut rng = Rng::new(0);
        let draws: BTreeSet<u64> = (0..16).map(|_| rng.next_u64()).collect();
        assert_eq!(draws.len(), 16, "no repeats in the first sixteen draws");
        assert!(!draws.contains(&0));
    }

    #[test]
    fn bounded_draws_stay_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0..1_000 {
            assert!(rng.below(5) < 5);
        }
        assert_eq!(rng.below(0), 0, "a zero bound is not a panic");
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn bounded_draws_cover_their_range() {
        // Not a statistical test — just enough to catch a reduction that always returns 0,
        // which would make every "random" choice in a fixture the same choice.
        let mut rng = Rng::new(11);
        let seen: BTreeSet<usize> = (0..400).map(|_| rng.below(4)).collect();
        assert_eq!(seen, BTreeSet::from([0, 1, 2, 3]));
    }

    #[test]
    fn inclusive_ranges_include_both_ends() {
        let mut rng = Rng::new(13);
        let seen: BTreeSet<i64> = (0..400).map(|_| rng.between(10, 12)).collect();
        assert_eq!(seen, BTreeSet::from([10, 11, 12]));
        assert_eq!(rng.between(5, 5), 5);
        assert_eq!(rng.between(5, 1), 5, "a reversed range is not a panic");
    }

    #[test]
    fn a_certain_chance_always_happens_and_an_impossible_one_never_does() {
        let mut rng = Rng::new(17);
        for _ in 0..100 {
            assert!(rng.chance(4, 4));
            assert!(!rng.chance(0, 4));
            assert!(!rng.chance(1, 0), "a zero denominator is not a panic");
        }
    }

    #[test]
    fn picking_from_an_empty_slice_is_none() {
        let mut rng = Rng::new(19);
        let empty: [u8; 0] = [];
        assert_eq!(rng.pick(&empty), None);
        assert_eq!(rng.pick(&[9u8]), Some(&9));
    }
}
