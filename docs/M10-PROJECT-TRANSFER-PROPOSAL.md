# M10 — Project Transfer · Architecture proposal

- Status: **Revision 2 — APPROVED FOR IMPLEMENTATION**, 2026-10-29, subject to the three
  mandatory safeguards in [§0.2](#02-mandatory-safeguards-from-the-second-review). Approvals
  **A1–A5 granted**; **A6 and A7 deferred**.
- Stage progress: **Stage P complete** (PR #35), **Stage 1 complete** (PR #37),
  **Stage 2 complete** — the `.evara` container, its reader, the replacement protocol and its
  recovery, with T-60, T-83 and §7.2's inflation figure **measured**
  ([§C.10](#c10-stage-2-what-was-built-measured-and-corrected)). Stages 3–6 not started: no
  CLI command, no Tauri command and no screen.
- Phase: [ROADMAP.md](ROADMAP.md) Phase 9. Closes **M1-17** and **M1-18**.
- Branch: `docs/m10-project-transfer-plan`, from `origin/develop` at `0faff47` — the merge of
  PR #35 (Stage P), verified present: `Cargo.toml` carries `calamine 0.37` and
  `rust_xlsxwriter 0.102`, and `Cargo.lock` resolves exactly one `zip`, at `9.1.3`, with
  `8.6.0` absent from the tree.
- Audited against `CLAUDE.md`, [ARCHITECTURE.md](ARCHITECTURE.md),
  [PROJECT-FORMAT.md](PROJECT-FORMAT.md), [DATA-MODEL.md](DATA-MODEL.md),
  [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md),
  [OPEN-DECISIONS.md](OPEN-DECISIONS.md), ADRs 0001–0052, and the code in `crates/evara-db`,
  `crates/evara-import`, `crates/evara-project`, `crates/evara-cli`,
  `apps/desktop/src-tauri`.
- Revision 2 adds twenty-two **measured or code-proven facts**; 2.1 and 2.2 bring the log to
  **thirty-two**, one of which (E-04) corrects a revision 2 entry that was wrong; 2.3 brings it
  to **thirty-eight**, one of which (E-33) corrects a *control the record claimed and had not
  tested*. Every one is listed with its evidence in
  [Part C §C.2](#c2-evidence-log--what-is-proven-and-what-still-needs-an-experiment),
  and nothing below is asserted without being marked proven, inferred, or pending.

**Revision history**

| Rev | Date | Change |
| --- | --- | --- |
| 1 | 2026-10-28 | First architecture proposal. Seven findings returned by review. |
| 2 | 2026-10-29 | All seven findings resolved (Part C). **Approved for implementation**: A1–A5 granted, A6–A7 deferred, three mandatory safeguards attached ([§0.2](#02-mandatory-safeguards-from-the-second-review)). D11's three known-wrong passages annotated as superseded in place. |
| 2.1 | 2026-11-01 | **Documentation only — no design change.** Stage P recorded as complete (PR #35, `0faff47`): §C.8 rewritten as delivered, all three RUSTSEC advisories named and resolved, E-23…E-26 added, finding 6's last pending experiment closed, D12 and §1.4.9 corrected to say `evara-project` has no `zip` dependency *yet*, §8 and the appendix risks updated, and S3 extended to state recovery across the individual sidecar moves explicitly. **No decision was reopened and no superseded D11 text was reinstated.** |
| 2.3 | 2026-11-12 | **Stage 2 folded in.** One **correction to a security control**: finding **F-2** — `SQLITE_OPEN_URI` was never a control in this build, because `-DSQLITE_USE_URI` makes URI interpretation the global default; §C.5, D6 and §6.1 now describe the control that exists (the path form, plus D6's "no archive byte reaches any path"), and **E-33** proves the flag's irrelevance against the engine. Two **defects found by composing Stage 1's primitives** and fixed: `looks_like_a_project` **created a database in a zero-length destination** and **deleted a stray `-wal`**, so *observing* the disk was mutating it — the thing S2's reconciliation assumes it does not (**E-34**). One **design fact recorded rather than assumed**: Deflate's own 1 032:1 ceiling makes the decompression-ratio cap unreachable for the two methods `formatVersion 1` admits, so what bounds a bomb is the byte cap (**E-35**). **T-60, T-83 and §7.2's inflation figure are measured** — §C.10 — and the §7.2 compaction question is **answered: no pass is added**. Four departures from the sketch, each recorded in §C.10. **No decision was reopened**, S1/S2/S3 stand exactly as §0.2 specifies them, and no superseded D11 text was reinstated. |
| 2.2 | 2026-11-02 | **Stage 1 folded in** (PR #37, `cc025e2`…`ff695f0`). One **correction** — E-04 was wrong, and was cited as confirming shipped code (§C.2, §C.6, and `backup.rs` on the branch). One **deviation accepted** — D8's two snapshot mechanisms became one, with the free-page cost stated (§C.6, D8, §7.2). One fact the record did not state — **a read-only open creates WAL sidecars** (§6.1). Six additions, **E-27…E-32**. The discharged experiments moved out of the pending list, and **T-60 and T-83 moved to Stage 2**: neither can be measured without a project with real edit history, and Stage 2's round-trip fixtures are the first to build one. `hooks` removed from the `rusqlite` feature list. **No other decision was reopened** — S1, S2 and S3 stand exactly as §0.2 specifies them, and no superseded D11 text was reinstated. Three later corrections, from the branch's own review commits (`ff695f0`, `2b30105`), which landed after this revision was first drafted: the invariant sweep is **T-88's and is 128 combinations**, not T-87's and not 192 (§9); D11's journal block now carries the **uniqueness index** and the `CHECK`s the migration actually ships; and D11 states what happens when a journal row is **damaged or written by a newer build**. |

> **Reading order for an implementer.** [§0.1](#01-approval-status-of-a1a7) is the
> authoritative approval status and [§0.2](#02-mandatory-safeguards-from-the-second-review) the
> authoritative replacement specification. Where [D11](#d11--replacement-is-journalled-non-destructive-and-recoverable)
> and §0.2 disagree, **§0.2 wins** — D11's superseded passages are kept, annotated, so the
> known-wrong text cannot be implemented by accident, and they are marked `⚠ S1`, `⚠ S2` and
> `⚠ S3` at the point of error.

---

## 0. Approved decisions — settled, not reopened

| # | Decision | Where it is built |
| --- | --- | --- |
| 1 | `.evara` is a ZIP carrying a **verified SQLite snapshot**, superseding ADR 0004's JSON-section container | [D1](#d1--the-payload-is-a-verified-sqlite-snapshot), ADR 0053 |
| 2 | The **complete `change_log`** is preserved | [D4](#d4--the-change-log-travels-complete) |
| 3 | **All** evaluations, violations, repair lineage, manual placements and locks are preserved | [D5](#d5--every-evaluation-finding-lock-and-lineage-edge-travels) |
| 4 | Container versioning and SQLite schema versioning stay **independent** | [D2](#d2--two-version-axes-one-refusal-each), ADR 0054 |
| 5 | Import is **offline, bounded, staged and validated** | [D6](#d6--nothing-is-ever-extracted-to-a-path-taken-from-the-archive), [D7](#d7--import-stages-verifies-then-renames), [§6](#6-security-and-integrity-guarantees) |
| 6 | The **ZIP security upgrade precedes** the archive implementation | [Stage P](#8-implementation-stages-and-dependencies) — **done, PR #35**, [§C.8](#c8-finding-6--dependency-upgrade-feasibility-resolved-and-delivered) |

These are fixed. Revision 2 reopens none of them, and found no technical blocker to any.
**Decision 6 is now discharged rather than planned**: the upgrade landed on `develop` as
PR #35, all three advisories are out of the tree, and `cargo deny check` reports *advisories
ok, bans ok, licenses ok, sources ok*.

**One approved decision gained a proven constraint rather than a blocker.** Decision 5's
staged validation cannot rely on `Database::open_for_inspection` to produce the snapshot:
`VACUUM INTO` is **refused** under `PRAGMA query_only = ON` (proven, E-01). Two fallbacks
were tested and both work (E-02, E-03), so the decision stands with a named mechanism per
path — [D8](#d8--export-never-mutates-the-source-and-the-lock-is-held-for-one-statement).

## 0.1 Approval status of A1–A7

Granted 2026-10-29 at the second architecture review.

| # | Decision | Status |
| --- | --- | --- |
| **A1** | `Cancel` moves to `evara-core`, re-exported by `evara-solver` | ✅ **Delivered — Stage 1.** Every existing import compiles unchanged, and one test satisfies both paths with a single impl, so there is provably one trait rather than two that look alike |
| **A2** | `evara_site_id` marked `SQLITE_INNOCUOUS`; `trusted_schema = OFF` through phase V1 | ✅ **Delivered — Stage 1** (E-31). The flag is additive as predicted, and the test that matters turns `trusted_schema` **off** before asserting all three triggers fire — with it on, the bundled default, the flag changes nothing observable and a test would pass either way |
| **A3** | The replaced copy is **never** deleted by Evara | ✅ **Approved** — Stage 2 |
| **A4** | One **settings** migration, `V2__pending_replacement.sql` | ✅ **Delivered — Stage 1.** No project migration; the one-way-door cost of advancing the settings chain is written in the migration file itself, not only in the PR |
| **A5** | `backup.rs`'s 0-byte leftover fixed inside Stage 1 | ✅ **Delivered — Stage 1.** The target is removed on the `VACUUM INTO` failure path too, so seventeen failed upgrades no longer exhaust the sixteen backup names. E-04's correction is why that mattered more than it looked (§C.6) |
| **A6** | A `--without-history` export option | ⏸ **Deferred, and now decided against for M10 with a number rather than a gap.** T-60 measured the feed at **26.2%** of a realistic project's occupied pages (E-36): real, not dominating, and it deflates with everything else. §C.10 says why that does not force the option |
| **A7** | Scoped per-school export, and merge-into-existing | ⏸ **Deferred.** Neither in M10. OPEN-DECISIONS D38 |

A6 and A7 being deferred changes nothing already specified: M10 carries the complete feed and
the whole campus file, which is what every section below already describes.

## 0.2 Mandatory safeguards from the second review

Three safeguards were made **conditions of approval**. All three are defects in **D11 as
revision 2 wrote it** — two of them would lose a user's project — so each is recorded here
with the finding, why the current text is wrong, and the resolution to build.

> **None of these is optional, and none is closed.** Stage 2's replacement support is **not
> complete** until all three are implemented *and* covered by the tests named below. D11's
> affected steps are annotated with `⚠ S1` / `⚠ S2` / `⚠ S3` pointing back here, so the
> known-wrong text cannot be implemented as written by accident.

### S1 — `create_new(true)` contradicts adopting a zero-length destination

**The finding.** D11 P1 says a destination that is *"absent, or present and zero length"* is
claimed with `OpenOptions::new().write(true).create_new(true)`. `create_new` **fails if the
path exists at all**, zero length included. The two halves of that sentence cannot both hold,
so the atomic claim and the zero-length case are mutually exclusive as specified.

**Why it matters beyond the contradiction.** The claim exists to close a TOCTOU window:
`fs::rename` **replaces** an existing destination on both platforms, so without a claim a file
created between the check and the rename is silently destroyed. An implementer resolving the
contradiction the easy way — drop `create_new`, just check and rename — reopens exactly that
window.

**Resolution.** Keep `create_new(true)` as the only claim, and narrow the Create path to a
destination that does **not exist**:

| Destination | Mode | Behaviour |
| --- | --- | --- |
| absent | **Create** | `create_new(true)` succeeds and holds the name. The final rename replaces our own placeholder |
| exists, zero length, **named by a journal row** | recovery | it is our own abandoned claim — §0.2 S2's reconciliation owns it, not the import |
| exists, zero length, **no journal row** | **occupied** | requires explicit `OnConflict::Replace`, and the Replace path keeps it as a `.replaced-` copy, so even an unknown empty file is not destroyed |
| exists, non-empty | **occupied** | `ProjectAlreadyExists`, or Replace |

This **deliberately diverges from `Database::create`**, which treats a zero-length file as
creatable. The two are different acts: `Database::create` is making a project where the user
just said to, and an import adopting an unknown empty file would be writing over something
nobody has looked at. The divergence is stated rather than reconciled.

A non-replacing rename was considered and rejected: `renameat2(RENAME_NOREPLACE)` /
`renamex_np(RENAME_EXCL)` / `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` all need FFI,
and `unsafe_code` is forbidden workspace-wide. `fs::hard_link` fails atomically when the
target exists and would work on NTFS and APFS, but not on exFAT — a USB stick would break it,
so it is not the primitive to depend on.

**Tests:** T-84, T-85.

### S2 — Recovery must reconcile the journal against the filesystem, not trust it

**The finding.** D11's recovery table maps a journal phase straight to an action, which
assumes the phase describes the disk. It does not. **The journal row is committed *before* the
rename it authorises**, so a crash in that window leaves the phase ahead of the filesystem.

**The data-loss case, concretely.** Journal says `PLACED`, but the crash landed between that
commit and the rename. On disk: the destination is **absent**, the staged file is still in
staging, and the old project is in `staging/aside/`. D11's table says *"`PLACED` → finish P4
step 3 — move the aside set to the keep-name."* Following it moves the user's only project to
a `.replaced-` name and leaves **no project at the destination at all**.

**Resolution.** The journal records **intent** and names the paths; the **filesystem is the
truth**. Recovery probes four facts — does the destination exist and is it a readable project;
does `staging/staged.evaraproj` exist; does the aside set exist; does `keep_name` exist — and
acts on the observed combination:

| destination | staged | aside | Action | Direction |
| --- | --- | --- | --- | --- |
| absent | present | absent | delete staging; delete a zero-length claim if one is ours | roll back |
| absent | present | present | **move aside → destination**, then delete staging | roll back |
| absent | absent | present | **move aside → destination** | roll back |
| present, is a project | absent | present | move aside → `keep_name` | complete |
| present, is a project | absent | absent | delete the journal row | already finished |
| present, is a project | present | present | ambiguous | **report, act on nothing** |
| present, **not** a project | any | present | **move aside → destination** after setting the bad file aside | roll back |

**One invariant dominates the table and is asserted in code, not just documented: the old file
set is released only once a destination that `looks_like_a_project` has been observed.** Every
combination that does not satisfy that rolls back or reports. An ambiguous state is reported
and never guessed — guessing is how the `PLACED` case above loses a project.

**Tests:** T-86, T-87, T-88 — one per journal-commit-to-rename window, by killing the process
in each.

### S3 — A main file must never be paired with another database's sidecars

**The finding.** D11 says the aside move *"takes the whole set, together"*. There is **no
atomic multi-file move** on either platform. A crash between moving `X.evaraproj` and moving
`X.evaraproj-wal` leaves the old `-wal` at the destination's name; P4 step 2 then renames the
**new** project in beside it. A new main file with a foreign `-wal` is a database that **opens
cleanly and is wrong** — the exact failure `backup.rs`'s header describes.

**Resolution: reduce each side to one file before moving it, so there is no set to move.**

1. The destination is not open — P0 refuses if it is. Open it, `PRAGMA wal_checkpoint(TRUNCATE)`,
   close. A clean close removes `-wal` and `-shm`; verify they are gone and refuse
   (`RenameBlocked`) if they cannot be removed, **before anything has moved**.
2. The old project is now **one file**. Move it with a single rename.
3. The staged project is closed after migration, so it is one file too.
4. **Before** renaming the new main into place, assert that no `-wal`, `-shm` or `-journal`
   exists at the destination's name. Refuse if one does, rather than renaming beside it.

Step 4 is the safeguard proper: it makes the bad pairing unrepresentable instead of merely
unlikely, and it holds even if a future change reintroduces a multi-file move.

**Recovery must cover the individual moves, not only the main file.** Reducing each side to one
file removes the multi-file move from the *forward* path; it does not remove the states a crash
can leave *while* that reduction is happening, nor the ones an older build or a foreign tool
may already have left on disk. So S2's reconciliation takes a **fifth** probe alongside its
four: **which of `-wal`, `-shm` and `-journal` exist at the destination's name, and which exist
beside the aside set and the staged file.** It is read per sidecar, never as "the set", because
each one moves on its own.

Two rules follow, and they are asserted in code rather than left to the phase table:

- **A main file with any sidecar present is not a project `looks_like_a_project` may bless.**
  Under S2's dominating invariant — *the old file set is released only once a destination that
  `looks_like_a_project` has been observed* — such a destination therefore reads as **"present,
  not a project"**, which rolls back rather than completing. This is deliberate: a new main file
  beside a foreign `-wal` is the one corruption that opens cleanly, so the only safe reading of
  an unexplained sidecar is that the destination is not trustworthy yet.
- **Recovery never deletes a sidecar to make a destination look clean, and never pairs one
  across sides.** A sidecar beside the aside set belongs to the aside set and travels with it
  or is checkpointed away by reopening *that* database; a sidecar at the destination's name
  whose main file has already moved is an orphan, and it is **reported** (`RenameBlocked`) and
  acted on by nothing. Deleting a `-wal` is discarding committed transactions that the main
  file does not yet contain — the data-loss move dressed up as cleanup.

**Tests:** T-89, T-90 — interrupt at each individual sidecar step, and assert that **no
reachable state pairs a main file with a sidecar belonging to a different database**, and that
every interrupted sidecar state is reconciled or reported, never silently adopted.

---

# Part C — Architecture Review Resolution

Placed first because it is the answer to the review. The revised specification follows in
Parts A and B.

## C.1 Resolution register

| # | Finding | Status | Resolution | Section |
| --- | --- | --- | --- | --- |
| **1** | Fix checksum architecture | ✅ **Resolved** | Cycle was real and is removed. One root, one digest per fact, a fixed verification order, and an explicit corruption-vs-authenticity statement. | [§C.3](#c3-finding-1--checksum-architecture-resolved), [§3.3](#33-integrity-model), [§6.2](#62-what-is-not-guaranteed) |
| **2** | Strengthen destination replacement | ✅ **Resolved** | Non-destructive journalled protocol. The old project is *never* deleted, even on success. Crash recovery is deterministic from a journal in the settings database. Windows/macOS asymmetry stated, not papered over. | [§C.4](#c4-finding-2--destination-replacement-resolved), [D11](#d11--replacement-is-journalled-non-destructive-and-recoverable) |
| **3** | Harden untrusted SQLite handling | ✅ **Resolved** | Three-phase boundary with eight `DBCONFIG` flags and seven SQLite limits, all proven available in rusqlite 0.37. `sqlite_master` conformance is explicitly *not* claimed as a sandbox; two experiments show why it and `integrity_check` are each insufficient alone. | [§C.5](#c5-finding-3--untrusted-sqlite-handling-resolved), [§6.1](#61-the-three-phase-boundary) |
| **4** | Prove snapshot consistency | ✅ **Resolved, and now built** | `VACUUM INTO` under `query_only` is **refused** (E-01). Manifest facts read **from the snapshot**, not the source. Stage 1 re-proved E-01…E-08 against the bundled engine, **corrected E-04**, and settled on **one** mechanism — a stepped backup inside a pinned read transaction (E-27) — because `progress_handler` cannot take a borrowed `&dyn Cancel` (E-32). The cost is a payload that is not compacted (§7.2). Lock duration (T-83) is measured in Stage 2. | [§C.6](#c6-finding-4--snapshot-consistency-resolved-experimentally), [D8](#d8--export-never-mutates-the-source-and-the-lock-is-held-for-one-statement) |
| **5** | Duplicate project identity | ✅ **Resolved** | Identity always preserved. Ambiguity made visible in the recent list, warned before import, refused when the twin is open. Messages and five tests specified. | [§C.7](#c7-finding-5--duplicate-project-identity-resolved), [D13](#d13--identity-is-preserved-and-duplication-is-disclosed) |
| **6** | Confirm dependency upgrade feasibility | ✅ **Resolved, and now delivered** | Not merely feasible — **done**. `calamine 0.36.1 → 0.37.0`, `rust_xlsxwriter 0.99.1 → 0.102.0`, `zip 8.6.0 → 9.1.3` transitively with `8.6.0` out of the tree. **RUSTSEC-2026-0367, -0368 and -0369 all resolved**, with no `[advisories] ignore` and `deny.toml` untouched. Merged as **PR #35**, 2026-11-01. | [§C.8](#c8-finding-6--dependency-upgrade-feasibility-resolved-and-delivered), [Stage P](#8-implementation-stages-and-dependencies) |
| **7** | Reconcile API and command contracts | ✅ **Resolved** | Thirteen discrepancies found in revision 1 and fixed, including one that contradicted D8's own lock rule and one false claim about migrations. | [§C.9](#c9-finding-7--api-and-command-reconciliation-resolved), [§4](#4-public-rust-apis-and-tauri-command-contracts) |

| **F-2** | `SQLITE_OPEN_URI` claimed as a control | ✅ **Resolved — Stage 2** | The flag was never a control in this build: `-DSQLITE_USE_URI` makes URI interpretation the global default (E-33, proven against the engine). §C.5, D6 and §6.1 now describe what exists — D6's "no archive byte reaches any path", plus an assertion on the path form at every point a payload is opened. | [§C.10](#c10-stage-2-what-was-built-measured-and-corrected), [§C.5](#c5-finding-3--untrusted-sqlite-handling-resolved), [§6.1](#61-the-three-phase-boundary) |

**Deferred** (carried, with reasons, not resolved): a `--without-history` export option,
**still deferred, now with T-60's figure beside the decision rather than a gap** — 26.2% of a
realistic project's occupied pages, which is real and not dominating
([§C.10](#c10-stage-2-what-was-built-measured-and-corrected),
[§10 A6](#10-remaining-decisions-requiring-approval)); scoped per-school export and
merge-into-existing ([§10 A7](#10-remaining-decisions-requiring-approval)).

**Blocked**: nothing.

## C.2 Evidence log — what is proven, and what still needs an experiment

Experiments E-01…E-08 were run with the `sqlite3` CLI **3.50.6**. The bundled engine is
**3.50.2** (proven: `libsqlite3-sys-0.35.0/sqlite3/bindgen_bundled_version.rs` declares
`SQLITE_VERSION = "3.50.2"`). Same minor series, but **every one is re-proven as a Rust test
against the bundled engine in Stage 1** — that is what T-75…T-82 are for. Nothing in the
design depends on CLI-only behaviour.

| # | Claim | How established | Status |
| --- | --- | --- | --- |
| **E-01** | `VACUUM INTO` is refused under `PRAGMA query_only = ON` — *"attempt to write a readonly database (8)"* — **and leaves a 0-byte target file behind** | Experiment | **Proven** |
| **E-02** | The online backup API works from a `query_only` connection, including inside an explicit `BEGIN DEFERRED` read transaction, producing a complete copy | Experiment | **Proven** |
| **E-03** | `VACUUM INTO` works from a connection opened `SQLITE_OPEN_READ_ONLY` (file-level read-only, no `query_only` pragma) | Experiment | **Proven** |
| **E-04** | `VACUUM INTO`'s treatment of an existing target **depends on its content**: an existing *database* is refused (*"output file already exists"*), *non-database bytes* are refused differently (*"file is not a database"*), and a **zero-length file is adopted** and written into | Re-proven against the bundled engine — `transfer::tests_engine::t78_…` | **Corrected in 2.2.** Revision 2's wording was wrong, and it did **not** confirm `backup.rs`'s comment. See E-28 and §C.6 |
| **E-05** | `PRAGMA integrity_check` returns `ok` on a database carrying a hostile `AFTER INSERT` trigger and an extra table — so it is **not** a schema check | Experiment | **Proven** |
| **E-06** | `PRAGMA foreign_key_check` reports a violation that `integrity_check` still calls `ok` — so **both** are needed, answering different questions | Experiment | **Proven** |
| **E-07** | `PRAGMA writable_schema = ON` succeeds by default, which is what `SQLITE_DBCONFIG_DEFENSIVE` exists to block | Experiment | **Proven** |
| **E-08** | A failed `VACUUM INTO` to a non-existent directory leaves nothing behind — so E-01's leftover is specific to the refused-write case | Experiment | **Proven** |
| **E-09** | `rusqlite 0.37` exposes `FunctionFlags::SQLITE_INNOCUOUS` | `rusqlite-0.37.0/src/functions.rs:399` | **Proven** |
| **E-10** | `rusqlite 0.37` exposes `DEFENSIVE`, `WRITABLE_SCHEMA`, `TRUSTED_SCHEMA`, `ENABLE_VIEW`, `ENABLE_ATTACH_CREATE`, `ENABLE_ATTACH_WRITE`, `ENABLE_TRIGGER`, `ENABLE_FKEY` via `set_db_config` / `db_config` | `rusqlite-0.37.0/src/config.rs:19–131` | **Proven** |
| **E-11** | `rusqlite 0.37` exposes `Limit::SQLITE_LIMIT_ATTACHED` and ten other limits (`limits` feature) | `rusqlite-0.37.0/src/limits.rs:18–45` | **Proven** |
| **E-12** | `Connection::progress_handler` exists behind the `hooks` feature | `rusqlite-0.37.0/src/hooks/mod.rs:426` | **Proven** — and **not used**: E-32 is why, and the feature is not enabled (§7.4) |
| **E-13** | `Connection::backup`, `Backup::step(num_pages)`, `Backup::run_to_completion` exist behind the `backup` feature | `rusqlite-0.37.0/src/backup.rs:64,262,291` | **Proven** |
| **E-14** | The bundled build sets `-DSQLITE_ENABLE_LOAD_EXTENSION=1`, `-DSQLITE_USE_URI`, `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, `-DSQLITE_ENABLE_API_ARMOR`, and compiles in FTS3/FTS5/RTREE/STAT4/DBSTAT | `libsqlite3-sys-0.35.0/build.rs:124–143` | **Proven** |
| **E-15** | The bundled build sets **no** `-DSQLITE_TRUSTED_SCHEMA` flag, so `trusted_schema` defaults **ON** in Evara (the CLI shows `0` only because the shell turns it off) | `build.rs` absence + CLI reading | **Proven** |
| **E-16** | `calamine 0.37.0` requires `zip ^9.0`, MSRV 1.88, and still offers the `dates` feature | Live crates.io sparse index, fetched 2026-10-29 | **Proven** |
| **E-17** | `rust_xlsxwriter 0.100.0 / 0.101.0 / 0.102.0` require `zip ^9.0`, MSRV 1.88.0 | Live index, 2026-10-29 | **Proven** |
| **E-18** | `zip 9.1.3` is current and unyanked; `9.0.0–9.0.2` were yanked. `zip 9` adds non-optional `nix 0.31`, `windows-sys 0.61`, `typed-path`, `indexmap`, `memchr`, `getrandom 0.4`, `crc32fast` | Live index, 2026-10-29 | **Proven** |
| **E-19** | `evara-import`'s third-party surface is small: `calamine::{Reader, Xlsx, Data, Range}` and `rust_xlsxwriter::{Workbook, Format, FormatAlign, XlsxError}` | `grep` over `crates/evara-import/src/` | **Proven** |
| **E-20** | `recent_project.project_id` is a nullable column with **no** uniqueness constraint; the PK is `path_key`. Two entries can carry one `project_id` today | `settings-migrations/V1__settings.sql:21–27` | **Proven** |
| **E-21** | The settings database has its **own** refinery chain (`settings-migrations/`), separate from the project chain | `settings.rs:23–25` | **Proven** |
| **E-22** | `backup.rs` does **not** remove the target when `VACUUM INTO` itself fails — only when `verify` fails — and `choose_path` skips names that exist. Combined with E-01 this leaves a 0-byte file named like a backup | `backup.rs:78–96, 101–128` | **Proven** (pre-existing defect) |
| **E-23** | `cargo deny check licenses` **passes** with `zip 9`'s widened tree. `typed-path 0.13.0`, `nix 0.31.3` and the added build dependency `cfg_aliases 0.2.2` all satisfy `deny.toml`'s allow list, and **no exception was added** — `deny.toml` is byte-identical to its pre-upgrade self | `cargo deny check` on `0faff47`: *advisories ok, bans ok, licenses ok, sources ok* | **Proven** — this is the item revision 2 listed as pending |
| **E-24** | `develop`'s lockfile carried **three** ZIP advisories, not the two the triage note recorded: **RUSTSEC-2026-0367** (SUID/SGID bits preserved during extraction), **-0368** (`ZipArchive::extract` follows attacker-controlled symlinks out of the destination), **-0369** (`extract()` applies Unix permissions through symlinks without confining the target). All three are patched in `zip >= 9.0` and all three are now **absent** | `cargo deny check advisories` before and after PR #35 | **Proven** |
| **E-25** | Exactly **one** `zip` resolves in the tree, at `9.1.3`; `8.6.0` appears nowhere in `Cargo.lock`. So there is no dual-major `zip` for Stage 2 to link against | `Cargo.lock` on `0faff47` | **Proven** |
| **E-26** | The upgrade needed **no source change**: E-19's eight third-party types did not move, and `cargo test --workspace --all-features` passed 1 709 / 0, including the two `evara-import` round trips that write with `rust_xlsxwriter` (compressing through `zip`) and read back with `calamine`, one of which asserts `00124` survives as text | PR #35, verified on Windows | **Proven** |
| **E-27** | A stepped backup inside an explicit `BEGIN DEFERRED`, **whose first read pins the WAL snapshot**, does not observe a writer that commits on another connection during the copy. This is the fact the whole snapshot design rests on | `t77_a_stepped_backup_in_a_read_transaction_does_not_see_a_concurrent_write`, and again through the real API in `a_concurrent_write_does_not_reach_the_snapshot`, where the interleaving is driven from the progress callback rather than from a sleep | **Proven** (T-77) |
| **E-28** | `VACUUM INTO` adopts a zero-length target; refuses an existing database; refuses non-database bytes with a *different* message | `t78_vacuum_into_refuses_a_non_empty_target_but_adopts_a_zero_length_one` | **Proven** — supersedes E-04 as written |
| **E-29** | Opening a WAL-mode database with `SQLITE_OPEN_READ_ONLY` **creates `-wal` and `-shm` beside it**; the main file's own bytes are unchanged | `auditing_a_payload_leaves_its_bytes_identical` + `an_audited_payload_can_still_be_reduced_to_a_single_file` | **Proven** — §6.1 records what it costs |
| **E-30** | `PRAGMA wal_checkpoint(TRUNCATE)` cannot release the log while another connection holds the database, and the refusal arrives **before anything has moved** | `t90_a_checkpoint_refuses_while_the_database_is_still_open` | **Proven** — S3's refusal point, not a hypothetical |
| **E-31** | `evara_site_id` marked `SQLITE_INNOCUOUS` lets all three change-log triggers fire with `trusted_schema = OFF`; a connection **without** the function still cannot write a tracked row | five tests in `evara_db::tests_site_id_function` | **Proven** (A2 delivered) |
| **E-32** | `rusqlite 0.37`'s `progress_handler` requires `F: FnMut() -> bool + Send + 'static`, so a borrowed `&dyn Cancel` cannot drive it | `rusqlite-0.37.0/src/hooks/mod.rs:426–429` | **Proven** — the cause of §C.6's one-mechanism deviation |

| **E-33** | **`SQLITE_OPEN_URI` is not a control in this build.** The bundled engine carries `-DSQLITE_USE_URI`, so `sqlite3_open_v2` parses a filename beginning `file:` as a URI whether or not the flag is passed. Clearing the flag changes nothing observable | `t91_a_uri_filename_is_interpreted_without_the_open_uri_flag` | **Proven — and it corrects a control §C.5, D6 and §6.1 each claimed.** Finding F-2 |
| **E-34** | **Deciding "is this a project" by opening it mutates the disk.** `open_plain` runs `PRAGMA journal_mode = WAL`, which on a **zero-length** file creates a 4 096-byte database there; and opening a path beside a stray `-wal` makes SQLite recover the log and then **remove it** | `a_file_that_is_not_a_database_is_never_opened`, `observing_a_destination_changes_nothing_on_disk`, and the two `evara-project` tests that found it | **Proven — a Stage 1 defect, fixed in Stage 2.** The predicate now reads the sixteen-byte header first |
| **E-35** | **Deflate cannot express a decompression bomb.** Its maximum expansion is ~1 032:1, a property of the format, so the ratio cap at 2 000 is unreachable for the two methods `formatVersion 1` admits | `a_deflate_entry_cannot_declare_a_bomb_ratio` — 8 MiB of zeroes measured | **Proven.** What bounds a bomb is `MAX_PAYLOAD_BYTES` and the manifest's declared `byteLength`, through a limited reader |
| **E-36** | **The change feed is 26.2% of a realistic project's occupied pages** — 4 306 rows, 1 036 288 of 3 956 736 bytes, measured from `dbstat` | `t60_the_change_feed_share_of_a_payload` | **Measured — T-60 discharged.** [§C.10](#c10-stage-2-what-was-built-measured-and-corrected) |
| **E-37** | **A stepped backup of a realistic project holds the lock for ~46 ms** — 1 033 pages in 5 steps of 256, ~9 ms per step; ~50 ms with a writer committing 133 times during the copy | `t83_the_lock_duration_of_a_stepped_backup`, `t83_a_concurrent_writer_changes_the_duration_and_not_the_copy` | **Measured — T-83 discharged.** An order of magnitude inside the 500 ms budget, so D8's contingency is not needed |
| **E-38** | **A snapshot is the project's size page for page, and ~6.9% larger than a vacuumed copy would be**; the archive then saves 88.7% of it, because free pages are mostly zeroes | `the_snapshot_is_not_compacted_and_the_archive_absorbs_most_of_it` | **Measured — §7.2's figure discharged, and the compaction question answered: no pass is added** |

**Discharged in Stage 1.** These were the pending experiments, and they ran:

| Was pending | Outcome |
| --- | --- |
| E-01…E-08 against the **bundled** 3.50.2 engine through `rusqlite` | ✅ **Done** — T-75…T-82. E-01, E-02, E-03, E-05, E-06, E-07 and E-08 hold exactly as the CLI reported them. **E-04 does not**, and is corrected above |
| Whether a stepped backup inside a `BEGIN DEFERRED` holds the read snapshot for the whole copy under `rusqlite` | ✅ **Done — it does.** E-27, asserted twice |
| `cargo deny check licenses` with `zip 9`'s tree | ✅ Recorded as E-23 in revision 2.1 |

**Discharged in Stage 2.** The two figures 2.2 moved here, and the one §7.2 asked for:

| Was pending | Outcome |
| --- | --- |
| Workspace-lock duration for a **stepped backup** at the realistic upper end | ✅ **~46 ms** for 1 033 pages in 5 steps, ~9 ms per step; ~50 ms under a concurrent writer. E-37, and an order of magnitude inside the 500 ms budget |
| `change_log`'s share of the payload for a project with real edit history | ✅ **26.2%** of occupied pages. E-36. It decides A6, which stays deferred — §C.10 says why the figure does not force the option |
| The snapshot's free-page inflation (§7.2) | ✅ **+6.9%** over a vacuumed copy, of which the archive absorbs 88.7%. E-38, and **no compaction pass is added** |

**Still requiring an experiment**, and scheduled:

| Pending | Why it cannot be settled on paper | Stage |
| --- | --- | --- |
| Windows rename behaviour under a live antivirus scan | Environmental; needs CI plus a manual pass | 4 (T-70) |
| The **ten-times** project's figures (T-59 at 20 000 occurrences) | Needs a fixture an order larger than §7.3's realistic one, which is a build-time cost every run would pay. The realistic scale is measured (E-36…E-38); the ten-times row of §7.3 is not | 3, alongside T-41's generated matrix |

**Why T-60 and T-83 moved from Stage 1 to Stage 2.** Both need a project with **real edit
history and realistic size**, and Stage 1 builds no such fixture — its projects are a handful
of rows created inside a test. Measuring either against one would have produced a figure with
no relationship to the budget it is supposed to test, which is worse than an honest gap.
Stage 2's round-trip fixtures are the first in M10 to build a project at the §7.3 scale, so
that is where both belong. T-60 still decides **A6**, so deferring the measurement defers that
decision too — consistent, since A6 was already deferred out of M10 entirely.

**Settled since revision 2 was written:** *"whether `cargo deny check licenses` passes with
`zip 9`'s new transitive crates (`typed-path`, `nix`)"* was the one pending item in finding 6.
It is now **E-23 — proven, and it passes.** Nothing in the pending list belongs to Stage P any
more.

## C.3 Finding 1 — checksum architecture: resolved

**The circular dependency was real.** Revision 1 §3.3 said `checksums.json` *"covers every
other entry including `manifest.json`"* while the manifest carried the digest of
`checksums.json`. That is a cycle: neither file can be verified before the other, so in
practice neither was verified at all. Revision 1 also recorded the payload digest **twice**
(in `manifest.payload.sha256` and in `checksums.json`), which is two places for one fact.

**Resolved.** The integrity model is now a two-level tree with a single root and exactly one
digest per fact, specified in [§3.3](#33-integrity-model):

```
manifest.json  ──integrity.sha256──►  checksums.json  ──entries{}──►  payload/project.evaraproj
manifest.json  ──payload.byteLength──►  the bound for the payload read
```

- `manifest.json` is **not** listed in `checksums.json`, and `checksums.json` carries no
  digest of the manifest. The cycle is gone by construction.
- `payload.sha256` is **removed from the manifest**. The payload digest lives in
  `checksums.json` only.
- `payload.byteLength` **stays** in the manifest, because the payload read must be bounded
  before any digest is known.
- The verification order is fixed and each step gates the next ([§3.3](#33-integrity-model)).
- `integrity.algorithm` must be exactly `"sha256"`; any other value is a refusal, not a
  negotiation. An algorithm field that accepts alternatives is a downgrade surface, and with
  no signature there is nothing to negotiate.

**Corruption versus authenticity, stated plainly and in three places** (§3.3, §6.2, and R11
in OPEN-DECISIONS): the manifest is the deliberately unverified root. A self-describing
archive cannot verify its own root without a key, and ADR 0001 forbids the service that would
distribute one. **These digests detect corruption — truncation, bit rot, a bad copy, a
partially written archive. They do not establish authenticity: anyone can rewrite the manifest
and recompute every digest below it.** A `.evara` carries exactly the authority of the person
who sent it, and the interface says so at the point of import rather than in a document.

## C.4 Finding 2 — destination replacement: resolved

Revision 1 said "stage, then rename" and left the hard half unspecified. The replacement
protocol is now [D11](#d11--replacement-is-journalled-non-destructive-and-recoverable) in
full. The headline: **Evara never deletes the project it replaced, even on success.** The old
file set is renamed to `<stem>.replaced-<stamp>.evaraproj`, following `backup.rs`'s
established rule that nothing in Evara deletes a safety copy. "A failed import must not
destroy or partially replace the previous project" is therefore not a promise about error
handling — there is no code path that deletes it at all.

The filesystem guarantees actually relied on, and the ones deliberately **not** claimed, are
in [D11 §Guarantees](#guarantees-relied-on-and-guarantees-not-claimed). In summary:

- **Relied on:** a single intra-directory rename on one volume is atomic for other observers
  — either the old name or the new name resolves, never a half-written file. The destination
  is only ever written *by a rename*, never by a stream.
- **Not claimed:** crash-safe *durability*. Rust's std cannot `fsync` a directory handle on
  Windows, so after a power loss the filesystem may present either side of a rename and the
  journal row may itself be lost. What is claimed is weaker and sufficient: the set of
  reachable states is `{old project intact}` or `{new project in place, old project kept
  beside it}` — never a partially replaced destination.
- **The platform asymmetry is named**: on Windows a rename over a file another process holds
  open **fails** (SQLite opens without `FILE_SHARE_DELETE`); on macOS/APFS it **succeeds**
  and the holder keeps writing to an orphaned inode. Evara closes its own handle and cannot
  detect another process's. That residual risk is documented, and the kept `.replaced-`
  copy is what makes it survivable.

Also covered there: symlink and TOCTOU defence (ancestor `symlink_metadata` checks plus an
atomic `create_new` claim of a free destination), bounded rename retries for antivirus
interference, sidecar handling for `-wal` / `-shm` / `-journal` as one set, and journal-driven
recovery of abandoned staging at `Workspace::new`.

**One correction carried from revision 1**: the pre-flight free-space check is **removed**.
There is no free-space API in the dependency tree, `unsafe_code` is forbidden workspace-wide,
so the check would need a new dependency to do a thing the non-destructive protocol already
makes safe. `ENOSPC` becomes a named error with a remedy instead.

## C.5 Finding 3 — untrusted SQLite handling: resolved

**`sqlite_master` comparison is not treated as a sandbox, and the proposal now says so with
evidence.** Two experiments show why neither available check is sufficient alone:

- **E-05**: `integrity_check` returns `ok` on a database carrying a hostile trigger and an
  extra table. It is a *corruption* check and detects nothing about schema shape.
- **E-06**: `foreign_key_check` reports a violation that `integrity_check` calls `ok`. The
  two answer different questions and both are required.

Conformance catches the hostile trigger that E-05 misses; it does **not** bound page-content
risk, and a bug in SQLite's own b-tree parser is not mitigated by any of this. That residual
risk is stated in [§6.2](#62-what-is-not-guaranteed).

The boundary is now three phases ([§6.1](#61-the-three-phase-boundary)) with every control
proven available:

- **Eight `DBCONFIG` flags** (E-10): `DEFENSIVE` on (blocks `writable_schema`, which E-07
  proves is otherwise permitted, and blocks writes to virtual-table shadow tables);
  `WRITABLE_SCHEMA` off; `TRUSTED_SCHEMA` off; `ENABLE_VIEW` off (free — the schema has no
  views); `ENABLE_ATTACH_CREATE` off; `ENABLE_ATTACH_WRITE` off (both 3.49+, and the bundled
  engine is 3.50.2).
- **Seven limits** (E-11), including `SQLITE_LIMIT_ATTACHED = 0`, which forbids `ATTACH`
  outright and works independently of engine version.
- **The path form, not an open flag** — corrected in revision 2.3, finding **F-2**. The
  earlier text said `SQLITE_OPEN_URI` was *off* and treated that as the control. It is not a
  control this build has: E-14 proves the bundled build sets `-DSQLITE_USE_URI`, and that
  compile-time option makes URI interpretation the **global default** — `sqlite3_open_v2`
  parses a filename beginning `file:` as a URI whether or not the caller asked for it. The
  flag can turn URI handling *on* for a build that defaults off; it cannot turn it off for a
  build that defaults on. **E-33** proves it against the bundled engine rather than inferring
  it from the build flag, which is the point of F-2: the record believed a control it had not
  tested.

  What closes the question is two things, neither of them a flag. First,
  [D6](#d6--nothing-is-ever-extracted-to-a-path-taken-from-the-archive): every path a payload
  is opened by is one **Evara minted** — a staging directory Evara created, a UUIDv7 file name
  Evara chose — so no archive byte contributes to any path. Second, the form is asserted
  anyway, at the one place a payload is opened, by
  `evara_db::transfer::paths::require_plain_database_path`: a `file:` prefix, a `?`, a `#`, a
  `%` escape, an interior NUL or a `..` component is a refusal
  (`DbError::DestinationPathUnsafe`). Belt and braces by design — the assertion costs a string
  scan, and it is what turns a future refactor that *did* let a name through into a refusal
  rather than into a URI with `vfs=` and `mode=rwc` in it. The flag is still not requested,
  which is now recorded as a statement of intent rather than as a control.
- **Extension loading**: E-14 proves `-DSQLITE_ENABLE_LOAD_EXTENSION=1` is compiled in. The
  runtime gate defaults closed, rusqlite's `load_extension` Cargo feature is not enabled, and
  `load_extension()` is `SQLITE_DIRECTONLY` so it cannot be reached from a schema object.
  Three independent reasons — and T-72 asserts it rather than assuming it.
- **Verification strictly before any write**: conformance runs against
  `expected(declared schemaVersion)`, built in a temp file by `migrations::run_up_to`, on a
  `query_only` connection, touching **no domain table**. The `app_meta` identity read happens
  *after* conformance passes, which is the ordering revision 1 got wrong.

**`trusted_schema` and `evara_site_id()` — the compatibility question, answered.** E-15 proves
the bundled default is **ON**. Turning it off *would* break every change-log trigger, because
`evara_site_id` is registered with `SQLITE_UTF8 | SQLITE_DETERMINISTIC` and a non-innocuous
function cannot be called from a schema object when `trusted_schema` is off. So:

- **Phase V0 (hardened, read-only)** sets `trusted_schema = OFF`. Safe, because no change-log
  trigger can fire where nothing is written and no tracked table is queried. This is where the
  protection is actually needed, and it requires no change to the shared factory.
- **Phase V1 (migration)** leaves `trusted_schema` at the build default, because conformance
  has already proven the schema is exactly ours — there is no crafted function call left to
  guard against.
- **Optional extra hardening, needing approval** ([§10 A2](#10-remaining-decisions-requiring-approval)):
  mark `evara_site_id` `SQLITE_INNOCUOUS` (E-09 proves rusqlite exposes the flag) and keep
  `trusted_schema = OFF` through V1 as well. It is a one-line change, but it touches the
  connection factory every project open uses, so it is yours to approve rather than mine to
  slip in.

Malicious and malformed payload cases are T-29, T-30, T-68…T-74.

## C.6 Finding 4 — snapshot consistency: resolved experimentally

**The `VACUUM INTO` / `query_only` question is settled, not deferred.**

- **E-01: `VACUUM INTO` is refused under `PRAGMA query_only = ON`** — `attempt to write a
  readonly database (8)`. Revision 1 flagged this as an unverified risk; it is now a proven
  fact, and the design branches on it rather than hoping.
- **E-01 also proved a second thing**: the refused `VACUUM INTO` **created a 0-byte target
  file before failing**. E-08 shows this is specific to the refused-write case. E-22 shows
  `backup.rs` does not clean up on that path and `choose_path` then skips the name — so a
  pre-existing code path can leave a zero-byte file called `school.pre-v11.evaraproj`, which
  is precisely what that module's header says must never happen. **Fixed in Stage 1**
  ([§10 A5](#10-remaining-decisions-requiring-approval)), because M10 shares the primitive:
  the target is now removed on the `VACUUM INTO` failure path too.
- **E-04 was wrong, and the way it was wrong mattered.** Its provenance read *"confirms
  `backup.rs`'s comment"*, and that module's header duly asserted SQLite's refusal **was** the
  "do not overwrite an existing backup" rule, *"enforced by the engine rather than by a
  `try_exists` race"* — which invited a later reader to delete `choose_path`'s existence check
  as redundant. **E-28** proves the engine *adopts* a zero-length target. Combined with E-01,
  which leaves exactly such a file, the two describe a real sequence: a failed backup leaves a
  0-byte file, and the next `VACUUM INTO` aimed at that name writes into it rather than
  refusing. The header is corrected on the Stage 1 branch and `choose_path`'s check is
  load-bearing, not belt-and-braces.
- **E-02: the online backup API works from a `query_only` connection**, including inside an
  explicit `BEGIN DEFERRED` read transaction, producing a complete copy.
- **E-03: `VACUUM INTO` works from a `SQLITE_OPEN_READ_ONLY` connection.** File-level
  read-only does not block it; only the `query_only` pragma does.

**One mechanism, both paths** — amended in revision 2.2, after Stage 1 built it:

| Path | Mechanism | Consistency comes from |
| --- | --- | --- |
| Desktop export (live writable connection, under the workspace lock) | `BEGIN DEFERRED` + a **stepped** `Backup` | the pinned read snapshot (E-27) |
| Headless `evara export --project` (`open_for_inspection`, `query_only`) | the same (E-02, E-13) | the same |
| Named contingency, **not** needed | a second `SQLITE_OPEN_READ_ONLY` connection + `VACUUM INTO` (E-03) | `VACUUM INTO`'s read transaction |

**Why revision 2's two mechanisms became one.** `VACUUM INTO` is a single opaque statement, so
its only interrupt point is SQLite's progress handler — and **E-32** proves rusqlite requires
that closure to be `Send + 'static`, which a borrowed `&dyn Cancel` is not. Implementing the
split literally would therefore have shipped a **desktop export that cannot be cancelled**, on
the one path where a user is most likely to want to, while this proposal asks for consistency
*and* cancellation together. Two alternatives were weighed and rejected: widening the public
signature to `&(dyn Cancel + Sync)` and spawning a watcher thread to call `interrupt()` — a
thread and a poll interval bought to preserve a mechanism split that buys nothing else — and
shipping the uncancellable path, which would be a silent downgrade of what was approved.

**What that costs, stated rather than absorbed.** `VACUUM INTO` compacts as it copies and the
backup API does not: it reproduces the source page for page, free pages included. A snapshot of
a heavily edited project is therefore **larger** than a vacuumed one would be. It is still a
valid, complete, single-file database — the cost is bytes, and `.evara` compresses them. This
bears on [§7.2](#72-disk), which was written assuming a vacuumed payload, and §7.2 now says so.
**No second compaction pass is introduced**: snapshotting and then `VACUUM INTO`-ing our own
snapshot would work and needs no cancellation, but it costs a second full write of the file and
there is no figure yet to justify it. Stage 2 measures the real inflation first.

The contingency's one limitation is named, in case it is ever needed: a read-only connection
cannot recover a hot `-wal`, so a project left dirty by a crash would be refused with a message
telling the user to open it once in the application. That is the correct refusal rather than a
silent partial read.

**All manifest facts now come from the exact exported snapshot.** Revision 1 read them from
the live project under the lock, which meant the manifest could describe a state the snapshot
did not hold. Corrected in [D8](#d8--export-never-mutates-the-source-and-the-lock-is-held-for-one-statement):
under the lock, Evara does the snapshot plus **one** cheap agreement check (`backup.rs`'s own
`school` row-count comparison); the lock is released; and every fact in the manifest is then
read from the snapshot file through a hardened read-only connection.

**Source-lock duration and cancellation**, both defined in D8: the workspace mutex is held for
exactly the snapshot and one `COUNT(*)`. Budget ≤ 500 ms at the realistic upper end, to
be measured (T-83, **Stage 2** — §C.2 says why it moved) — and if it exceeds that, the remedy
is a second read-only connection (E-03), not a longer lock. **Cancellation is the backup's own
step boundary**, not `progress_handler`: `cancel` is asked before the first page and between
every batch of 256, which is what the one-mechanism decision above buys. A cancelled or failed
snapshot removes its partial file and any sidecar beside it — structurally, in `Snapshot`'s
`Drop`, rather than remembered at each call site, which is the same defect E-22 found in
`backup.rs`.

The proof tests T-75…T-82 re-established E-01…E-08 against the bundled 3.50.2 engine through
`rusqlite` in Stage 1, before any container code existed. **Seven of the eight claims held;
E-04 did not** (above). Their implemented numbering differs from the draft matrix in
[§9](#snapshot-mechanism--the-stage-1-proof-tests), which records the mapping.

## C.7 Finding 5 — duplicate project identity: resolved

Revision 1 did not mention this case at all. Two facts frame it:

- **E-20**: `recent_project.project_id` is nullable with no uniqueness constraint; the primary
  key is `path_key`. Two recent entries carrying one `project_id` are representable **today**,
  with nothing to notice it.
- `Workspace` holds `open: Option<Database>` — **one** project at a time. So one installation
  cannot have two copies of a project open simultaneously. Divergence happens sequentially, or
  across installations.

**Identity is preserved, always** — ADR 0004's rule, and regenerating would destroy round-trip
comparison and any future synchronisation. Evara cannot merge two copies, so the honest
remedy is **disclosure, not prevention**. [D13](#d13--identity-is-preserved-and-duplication-is-disclosed)
specifies four behaviours, the exact user-facing sentences, and the derived
`duplicateIdentity` flag that makes the ambiguity visible wherever the recent list appears —
not only at the moment of import. Tests T-63…T-67.

## C.8 Finding 6 — dependency upgrade feasibility: resolved, and delivered

> **Status: closed by delivery.** Revision 2 argued the upgrade path existed. It has since
> been taken: **PR #35, merged into `develop` on 2026-11-01 as `0faff47`.** This section is
> kept as the record of what was predicted and what was actually measured, because two of the
> predictions came out in the user's favour and one inherited triage figure was wrong against
> it.

### What landed

| Crate | Was | Now | `zip` requirement | Note |
| --- | --- | --- | --- | --- |
| `calamine` | 0.36.1 | **0.37.0** | `^9.0` (was `^8.6`) | first release requiring `zip ^9.0`; still offers `dates` (E-16) |
| `rust_xlsxwriter` | 0.99.1 | **0.102.0** | `^9.0` (was `^8.3`) | still usable with `default-features = false` (E-17) |
| `zip` | 8.6.0 (transitive) | **9.1.3** (transitive) | — | **`8.6.0` is gone from the tree** (E-25) |
| `typed-path` | 0.12.3 | **0.13.0** | — | transitive, through `zip 9` |
| `quick-xml` | 0.41.0 **and** 0.42.0 | **0.42.0** only | — | 0.41.0 **removed**, consolidating on the copy already present |
| `nix` | — | **0.31.3** added | — | MIT, Unix-gated, not compiled on Windows |
| `cfg_aliases` | — | **0.2.2** added | — | MIT, build dependency |

All MSRVs are below the workspace's 1.92. Both version bounds are written in the workspace
`Cargo.toml` as **security floors with the reason beside them**, so a contributor reading
`0.37` does not relax it to `0.36` to dodge an API change.

### The three advisories, all resolved

The triage note this proposal inherited recorded **two** advisories. `cargo deny check
advisories` against `develop`'s own lockfile reported **three** (E-24) — measured by running
the check rather than by reading the note:

| Advisory | What it is | Status |
| --- | --- | --- |
| **RUSTSEC-2026-0367** | SUID/SGID bits preserved during ZIP extraction | ✅ **Resolved** — patched in `zip >= 9.0` |
| **RUSTSEC-2026-0368** | `ZipArchive::extract` follows attacker-controlled symlinks out of the destination directory | ✅ **Resolved** — patched in `zip >= 9.0` |
| **RUSTSEC-2026-0369** | `extract()` applies Unix permissions through symlinks without confining the target | ✅ **Resolved** — patched in `zip >= 9.0` |

All three are **absent from the tree on `0faff47`**, and the fix was the upgrade itself:
**no `[advisories] ignore` was added, no check was relaxed, and `deny.toml` is untouched.**

Worth naming, because it is the argument for
[D6](#d6--nothing-is-ever-extracted-to-a-path-taken-from-the-archive) rather than a
coincidence: **all three advisories are in `extract()`** — the API that takes a path from the
archive and writes to it. D6 refuses to call it at all, reading entries by name into memory,
so even an unpatched `zip` would not have been reachable through Evara's own code. That is
what made §1.4.9's triage note tolerable while `evara-import` was the only consumer, and it is
also why the upgrade was still mandatory: an unreachable vulnerability is one `cargo deny`
fails CI on, and the floor has to hold **before** Stage 2 links a reader for real.

### How the three named risks actually came out

- **"0.x minor bumps are breaking under cargo semver."** True in principle; **no source change
  was needed in practice** (E-26). E-19's eight third-party types did not move, and the two
  `evara-import` round trips that cross all three upgraded crates still pass — including the
  one asserting `00124` comes back as text, which is the fidelity failure a parser change
  would break first.
- **"`zip 9` widens the tree, and `typed-path`'s and `nix`'s licences must be checked."**
  Checked, and they pass: **`cargo deny check` reports *advisories ok, bans ok, licenses ok,
  sources ok*** (E-23). This was the single pending experiment in finding 6 and it is now
  closed. `nix` is additionally Unix-gated, so it is not compiled on Windows at all.
- **"`deny.toml` sets `yanked = "deny"`, and `zip 9.0.0–9.0.2` were yanked."** Avoided by
  landing on **9.1.3**, which is current and unyanked (E-18).

### The six gates, and the one that did not pass

Five of the six are green: `cargo deny check` (all four sections — and note it is still **not**
part of `npm run check`, so it must be run deliberately), `npm run rust:fmt`,
`npm run rust:lint`, `cargo test -p evara-import` (38 tests), and `cargo test --workspace
--all-features` at 1 709 passed / 0 failed. CI ran on `windows-latest` and `macos-latest`.

**`npm run check` does not pass, and that was reported rather than worked around.** Its last
step is the frontend suite, where two jsdom tests time out against vitest's default 5 000 ms
under full-suite load — `Schools.test.tsx` *"creates a second one alongside the first"* and
`RepairTimetable.test.tsx` *"gives a moved input a different remedy from an edited baseline"*.
Both pass in isolation, the failing set moved between consecutive runs, and PR #35 changed
`Cargo.toml` and `Cargo.lock` and nothing else — `git diff` over `apps/desktop/src` and
`packages/` was empty, so the suite's inputs were byte-identical to `develop`'s. **Pre-existing
flakiness, not Stage P's.** No timeout was raised and no test was skipped to make it green.
It is a gap **carried into Stage 1, not closed** — Appendix risk 8.

### What Stage P deliberately did not do

It carried **no project-transfer code**: no `.evara` reader or writer, no manifest, no
snapshot, no staging, and **no direct `zip` dependency for `evara-project`**. That is Stage
2's, and `zip 9.1.3` is reached today only transitively, through `evara-import`'s two parents.
When Stage 2 adds the direct dependency it takes
`default-features = false, features = ["deflate-flate2"]`, routing compression through
`flate2`, which is already in the tree.

The fallback recorded in [D12](#d12--the-engine-is-in-evara-db-the-format-is-in-evara-project)
— write the three-entry container with `flate2` + `crc32fast` and add no dependency at all —
is **no longer needed for feasibility** and survives only as a contingency if `zip 9`'s
*reader* disappoints in Stage 2.

## C.9 Finding 7 — API and command reconciliation: resolved

Thirteen discrepancies in revision 1, each fixed. Two were substantive bugs, not cosmetics.

| # | Discrepancy in revision 1 | Fix |
| --- | --- | --- |
| 1 | **`evara_project::export(db: &Database, …)` held the `&Database` borrow for the whole archive write — contradicting D8's own "the lock is held for one statement"** | Split: `Database::snapshot_into` under the lock returns an owned `Snapshot`; `evara_project::seal(snapshot, …)` runs with the lock released. `Snapshot: Send`, with a `send_assertions` test mirroring `evara_db::scheduling`'s. |
| 2 | **Manifest facts read from the live project, not the snapshot** | Read from the snapshot (finding 4, D8). |
| 3 | Said "Nine commands", listed ten | **Eleven**, enumerated and counted in [§4.3](#43-tauri-commands--eleven-counted). |
| 4 | `TransferHandle { runId, epoch }` | No `epoch` on a start or a status. `GenerateStarted { run_id, total }` and `RepairStarted { run_id, movable, frozen }` carry none; `epoch` appears only on event payloads, as `RepairProgressEvent` does. |
| 5 | Events named `evara://transfer-progress` | `transfer://progress` and `transfer://finished`, matching `solver://` and `repair://`. |
| 6 | Invented a third frontend error enum (`TransferErrorView`) | Removed. Transfer **is** project lifecycle, so `ProjectError` in `commands/project.rs` is extended — it already carries `SchemaTooNew`, `NotFound`, `WrongExtension`, `AlreadyExists`, `Failed`. `SetupError` is untouched. |
| 7 | `Overwrite { Refuse, Replace }` in Rust vs `replace: bool` in the command vs `Replace` in the tests | One enum, `OnConflict`, crossing IPC as `"refuse"` / `"replace"`. |
| 8 | Two `Cancel` traits (`evara_solver::Cancel` plus a new one) | Propose moving the trait to `evara-core`, re-exported by `evara-solver` — exactly ARCHITECTURE §3 rule 1's criterion. Touches a second crate, so [§10 A1](#10-remaining-decisions-requiring-approval). |
| 9 | A second copy of the 50 ms coalescing arithmetic | Generalise `CoalescingSink` over its payload; one implementation, two callers. |
| 10 | **"Nothing in M10 adds a migration"** | False as written. No **project** migration; **one settings migration** (E-21 proves the settings database has its own chain), `settings-migrations/V2__pending_replacement.sql`, for D11's journal. [§10 A4](#10-remaining-decisions-requiring-approval). |
| 11 | A pre-flight free-space check | Removed. No free-space API in the tree and `unsafe_code` is forbidden, so it needs a dependency to do what D11 already makes safe. Named `ENOSPC` error instead. |
| 12 | Circular checksums | Finding 1, §C.3. |
| 13 | Test matrix drift: T-32 conflated a bomb and an entry-count case; T-49 used the old overwrite vocabulary; nothing covered identity, recovery or the proof tests | T-32 split into T-32a/T-32b; T-49 rewritten for `OnConflict`; T-63…T-83 added. |

---

## C.10 Stage 2 — what was built, measured and corrected

Added in revision 2.3, after Stage 2 was built. **No decision was reopened.** S1, S2 and S3
stand exactly as [§0.2](#02-mandatory-safeguards-from-the-second-review) specifies them, and
no superseded D11 text was reinstated.

### What was built

`evara-project` gains the format and the protocol: the manifest and `checksums.json` with the
acyclic verification order, the ZIP writer and reader, the entry-name policy and the byte
caps, the staging lifecycle, D11 as amended by §0.2's three safeguards, recovery, and
`seal` / `inspect` / `import` / `recover_pending` / `abandoned_staging` behind `TransferError`.
The container migration chain is present and **empty** at `formatVersion 1`, which is the
decision rather than an omission (`migrate.rs`'s header says why).

`evara-db` gains the three things Stage 1 deferred here, plus two the composition needed:
`transfer::stage` (phases V1 and V2 beside Stage 1's V0), `transfer::table_census`, the
`fixtures` feature with its only consumer, `transfer::facts` — every manifest fact read **out
of the file it describes** — and `transfer::paths`, which is F-2's record.

### The one security correction: F-2

**`SQLITE_OPEN_URI` was never a control in this build, and the record said it was.** The
bundled engine carries `-DSQLITE_USE_URI`, which makes URI interpretation the *global default*:
the flag can add the capability and cannot remove it. **E-33** proves that against the engine
rather than inferring it from the build flag — which is the whole lesson of F-2, because the
record had inferred it.

§C.5, [D6](#d6--nothing-is-ever-extracted-to-a-path-taken-from-the-archive) and
[§6.1](#61-the-three-phase-boundary) now describe the control that exists:

1. **No archive byte reaches any path.** D6, unchanged, and it is the load-bearing half.
2. **The path form is asserted anyway**, by `require_plain_database_path`, at every point a
   payload is opened: a `file:` prefix, a `?`, a `#`, a `%` escape, an interior NUL or a `..`
   component is `DbError::DestinationPathUnsafe`. Asked **before** the existence check, so
   `file:x?mode=rwc` is reported as the path form it is.

The flag is still not requested. That is now recorded as a statement of intent rather than as
a control, which is the narrow scope the correction was asked to keep.

### Two defects the composition found, and what they say about the design

Both were in Stage 1 primitives, both were invisible until the protocol composed them, and
both were about **the same mistake**: deciding whether a path holds a project by *opening it*.

| Defect | What happened | Why it mattered |
| --- | --- | --- |
| A **zero-length** destination was written into | `looks_like_a_project` → `open_plain` → `PRAGMA journal_mode = WAL` **creates a database** in an empty file | The Replace path then kept that as the `.replaced-` copy, so what was preserved was Evara's own 4 096-byte header rather than the user's file. **S1 says an import must not write over an unknown file; checkpointing one is writing over it** |
| A stray `-wal` was **deleted** | opening a path beside one makes SQLite recover the log and remove it | [§0.2 S3](#s3--a-main-file-must-never-be-paired-with-another-databases-sidecars) forbids exactly this: a `-wal` may hold committed transactions its main file does not. And `DiskFacts::observe` calls this predicate, so **observing the disk was mutating it** — which S2's reconciliation assumes it does not |

Fixed by deciding from the **sixteen-byte SQLite header** before anything is opened
(`has_database_header`), and by probing `sqlite_master` once inside `UntrustedPayload::open` so
"this is not a database" is a sentence rather than an opaque internal error. **E-34**, with
regression tests in both crates.

The design lesson is worth keeping: §0.2's safeguards were asserted against the primitives in
Stage 1 and were all green. What they could not assert is that a *composed* protocol makes
those calls, in that order, without a mutation on the way in. That is why T-84…T-90 are owed
twice, and why the second payment found two defects the first could not.

### One design fact recorded rather than assumed

`MAX_COMPRESSION_RATIO` **cannot fire** on a `Deflate` entry: Deflate's maximum expansion is
~1 032:1, a property of the format rather than of an encoder (**E-35**, measured). So the cap
is unreachable for the two methods `formatVersion 1` admits, and lowering it below 1 032 would
refuse legitimate payloads — a snapshot's free pages are mostly zeroes and deflate almost
completely.

It is kept, for a future `formatVersion` admitting a method that *can* reach it. **What
actually bounds a Deflate bomb is the byte cap**: `MAX_PAYLOAD_BYTES` and the manifest's
declared `byteLength`, applied through a limited reader, so a stream producing more than it
declared stops at the bound rather than at the end of the disk. `limits.rs` carries the
reasoning so a later reader finds it rather than finding only the constant.

### The measurements

Fixture: `evara_db::fixtures::transfer::Shape::Realistic` — two schools, three campuses, both
cycle kinds, 126 sections, 503 occurrences, 80 teachers, 25 groups of 40, a shared building and
room, an embedded logo, four passes of real updates over every room, plus a real generation, a
second version, a duplicate, a lock and a **real repair** for the lineage edge; then one
timetable deleted, so there is a free list to measure. 4 231 168 bytes on disk.

| # | Figure | Measured | Budget | Verdict |
| --- | --- | --- | --- | --- |
| **T-60** | change feed's share of occupied pages | **26.2%** (4 306 rows, 1 036 288 of 3 956 736 bytes, from `dbstat`) | — | reported, not acted on |
| **T-83** | workspace lock held by a stepped backup | **46–82 ms** — 1 033 pages, 5 steps of 256 | ≤ 500 ms | **inside, by an order of magnitude** |
| **T-83** | the same, with a writer committing throughout | **50–130 ms** (115–133 commits during the copy), census identical | ≤ 500 ms | **inside**; E-27's consistency holds at scale |
| **§7.2** | snapshot over a vacuumed copy | **+6.9%** (67 free pages of 4 096) | — | **no compaction pass added** |
| **§7.2** | archive over payload | **88.2–88.7% smaller** (477–500 KB from 4 231 168 bytes) | — | the archive absorbs the free pages |
| **§7.3** | export / inspect / import, end to end | **91–164 / 11–17 / 179–299 ms** | 1 500 / 50 / 3 000 ms | **inside** |

**Why three of those are ranges and two are exact.** T-60 and the §7.2 inflation figure are
read from `dbstat` and the free list, so they reproduce byte-for-byte on any machine. The three
timing rows do not: they were measured twice on the same Windows laptop, under different load,
and came out 46 and 82 ms, 50 and 130 ms, and 91/11/179 and 164/17/299 ms. The archive-size
row is a range for a different reason: the fixture mints UUIDv7 keys, so its bytes differ a
little between builds and deflate accordingly. The range is what was
observed rather than a tolerance anyone chose, and the verdict is unchanged because the budgets
are an order of magnitude above the whole spread. This is also why the tests assert the
budget's **order of magnitude** and not a figure — a test pinning 46 ms would fail on a slower
disk and would be measuring the machine rather than the design.

**What T-60 decides, and what it does not.** 26.2% is a real share and not a dominating one,
and the figure moves with *use* rather than with school size — so a school ten years in will
see more. It does **not** force A6: the feed is the thing that makes `solve_run.inputs_watermark`
and `timetable_evaluation.inputs_watermark` meaningful, D4's rule that a feed with a hole is
worse than a full one or none still holds, and a quarter of a payload that deflates to an
eighth of its size is not a user-visible cost. **A6 stays deferred**, now with a number beside
the decision instead of a gap. `OPEN-DECISIONS` D37.

**What §7.2 decides.** The compaction question is **answered and closed**: a second
`VACUUM INTO` pass would recover 6.9% of a *staging* file at the cost of one more full write of
the payload, and the figure with a user-visible consequence — archive size — barely moves,
because deflate already absorbs free pages. No pass is added, as §7.2 said it would not be
without a figure to justify one.

### Four departures from the sketch, each with its reason

| # | Sketch | Built | Why |
| --- | --- | --- | --- |
| 1 | `inspect(archive, known: &dyn IdentityLookup)` returning `identityConflicts` | `inspect(archive)`, with no identity lookup | D13's detection needs `Settings::projects_with_identity` and `RecentProject::duplicate_identity`, which [§8](#8-implementation-stages-and-dependencies) puts in **Stage 4**. Adding a trait here with no implementation would have been a seam nothing crossed; T-63…T-67 are Stage 4's |
| 2 | `import(... )` taking no installation identity | `import(..., Importer { app_version, site_id }, journal, ...)` | V1's migration fires change-log triggers, which read `evara_site_id()` — so a staged payload that needs migrating needs the importing installation's identity. And the journal is a parameter rather than reached through a `Workspace`, which is what lets every recovery state be driven from a journal a test controls |
| 3 | `payload.sha256` absent; `exportedBySiteId` written by `seal` | `exportedBySiteId` is `None` from `seal` | `evara-project` has no installation identity of its own, and writing `created_by_site_id` into that field is exactly the confusion T-03 exists to catch. The command layer fills it in — Stage 4 |
| 4 | T-04's *"payload bytes identical"* after export → import → export | asserted as **identical census and identical manifest** but for `exportedAt` and `exportedBySiteId`; payload bytes identical for two exports of **one** project | Byte identity across an import is not achievable and should not be claimed: V1 opens the staged copy, and an open sets `journal_mode` and may stamp `app_meta`. What survives byte-for-byte is the thing that matters — two snapshots of an unchanged project (asserted), and every row (asserted) |

### What Stage 2 deliberately did not build

No CLI command, no Tauri command, no screen, no `TransferSessions`, no `transfer://` events, no
typegen. No `Settings::projects_with_identity` and no `RecentProject::duplicate_identity`
(Stage 4). No per-school export and no history-stripping export (A7, A6). **No project
migration** — V18 is the schema, and transfer reads and copies it. The one settings migration
is Stage 1's `V2__pending_replacement.sql`, unchanged.

### One pre-existing blocker, reported rather than bypassed

`cargo deny check advisories` **fails**, and it fails on `origin/develop` identically — checked
by running it there. **RUSTSEC-2026-0459**: SQLite before 3.54.0 can return uninitialised
page-cache memory when a database is opened together with a sibling `-wal` whose page size
differs from the database's. `libsqlite3-sys 0.35.0` bundles 3.50.2; the fix is
`libsqlite3-sys >= 0.39` via `rusqlite 0.41`.

Not introduced by Stage 2, and not in its scope to fix: `rusqlite 0.37 → 0.41` is a
cross-cutting dependency bump of exactly the shape Stage P was, and it wants its own PR for the
same reason — it touches the connection factory every project open uses. `bans`, `licenses` and
`sources` are **ok**, so `zip 9.1.3` and `sha2` added no licence or duplication problem.

Worth noting precisely *because* it is this advisory: the vulnerability's shape is a main file
paired with a foreign `-wal`, which is the one corruption
[§0.2 S3](#s3--a-main-file-must-never-be-paired-with-another-databases-sidecars) exists to make
unrepresentable. A `.evara` carries exactly three entries and no sidecar can arrive in one;
`refuse_if_sidecars_present` asserts the destination is clean immediately before the rename;
and `DiskFacts::observe` treats a main file with any sidecar as **not a project**. That is a
reason the exposure is narrow, and **not** a reason to leave the engine unpatched.

---

# Part A — Current architecture audit

## 1. What exists, what can be reused, and what the record gets wrong

### 1.1 `evara-project` is a documentation comment

Two files. `Cargo.toml` already declares the right edge (`evara-core`, `evara-db`, `serde`,
`serde_json`, `thiserror`); `src/lib.rs` is nineteen lines naming what Phase 9 will add.
There is no writer, reader, manifest, checksum, migration chain or test. `evara-cli`'s
`PLANNED` table already reserves `export` and `import` for phase 9.

### 1.2 Twelve primitives M10 reuses rather than reinvents

| Primitive | Where | Used for |
| --- | --- | --- |
| `VACUUM INTO` with verification | `evara-db/src/backup.rs` | The snapshot. Its header already argues the point: a WAL project's committed state is split between the file and `-wal`, so a file copy *"opens cleanly and silently lacks the most recent work"*. Its `verify` — `integrity_check`, is-a-project, row-count agreement — is the export's post-condition. |
| `Database::open_for_inspection` | `evara-db/src/lib.rs` | Headless export must not touch a live file. Three guarantees: `query_only` verified by read-back, no `evara_site_id()` registered, no migration or metadata write. Constrained by E-01 — see D8. |
| `Database::create`'s refusal | same | `ProjectAlreadyExists` on a non-empty target **is** "do not overwrite without approval", enforced before any write. A zero-length file is treated as creatable, which D11's atomic claim relies on. |
| `connection::open_configured` / `open_plain` / `open_query_only` | `connection.rs` | The one factory. D11 and §6 add a fourth, hardened constructor beside them. |
| `migrations::run` | `migrations.rs` | The forward-only grouped-transaction chain that migrates a staged payload. |
| `migrations::run_up_to(conn, n)` | same | `pub(crate)`, test-only today. Generates a reference schema at any historical version — the basis of both conformance (§6.1) and the compatibility matrix (§1.4.6). |
| `change_log::install_all` | `change_log.rs` | Recreates missing triggers on every open. |
| `change_log::Watermark`, `inputs_changed_since` | same | The freshness guard §1.4.2 is about. |
| The workspace-lock discipline | ARCHITECTURE §3.3, `commands/session.rs` | *"A solve never holds the workspace lock."* Export inherits it exactly, with an owned `Snapshot` in the role the owned `Assembly` plays. |
| The session state machine | `commands/session.rs` | Five named hazards; `Running → Finished → Saving → Saved`; cancel as an `AtomicBool` request that changes no state; `ProjectEpoch`. Transfer needs four of the five. |
| `CoalescingSink` | same | 50 ms minimum gap plus a guaranteed final event, with the clock read by the caller. Generalised in §4.3. |
| `cloud_sync::detect` | `cloud_sync.rs` | Advisory warnings. OPEN-DECISIONS R8 names `.evara` export as the mitigation for WAL-on-OneDrive; M10 discharges it, and the same detector warns about an export *destination*. |

### 1.3 What M10 must carry: fifty tables in four groups

`grep '^CREATE TABLE' crates/evara-db/migrations/*.sql` returns 50 across V1…V18 — 49 real
tables plus `school_rebuild`, V12's transient rebuild table that exists only inside that
migration's transaction. `TRACKED_TABLES` names 38 entity tables.

| Group | Tables | Status |
| --- | --- | --- |
| **Identity and history** | `app_meta`, `change_log`, `refinery_schema_history` | All travel. `change_log` complete, per approved decision 2. |
| **Structure and time** | `school`, `campus`, `building`, `building_campus`, `room_type`, `room`, `room_campus`, `resource`, `resource_campus`, `school_logo`, `academic_year`, `term`, `cycle`, `cycle_day`, `period_structure`, `period`, `timeslot`, `calendar_day` | All travel. `timeslot` ids are stable identity (ADR 0011) and locked entries reference them. `school_logo.bytes` is a BLOB to 2 MiB. |
| **People, curriculum, lessons, rules** | `year_level`, `teacher`, `student`, `student_group`, `student_group_member`, `teacher_school`, `teacher_campus`, `student_school_enrolment`, `availability`, `department`, `subject`, `teacher_qualification`, `teacher_department`, `course`, `class_section`, `class_teacher`, `class_group`, `class_enrolment`, `activity`, `constraint_instance`, `constraint_ref` | All travel. `activity` is one row per occurrence with a never-renumbered `split_index` (ADR 0029). |
| **Timetables, runs, findings** | `timetable` (+`based_on_timetable_id`), `timetable_entry` (+`is_locked`, `is_manual`), `solve_run` (+`kind`, `scope`), `solve_run_unscheduled`, `timetable_evaluation`, `violation`, `violation_ref` | All travel, per approved decision 3 — which corrects PROJECT-FORMAT §3 and TIMETABLE-PERSISTENCE §7. See §1.4.1. |

### 1.4 Findings in the accepted record

Carried from revision 1 as the record of *why* the approved decisions are what they are.

#### 1.4.1 "Violations are rebuilt on first score" cannot refill a `GENERATION` evaluation

PROJECT-FORMAT §3, TIMETABLE-PERSISTENCE §7 and ADR 0042 §C4 all say `violation` and
`violation_ref` are not exported because re-running `evaluate` reproduces them. But ADR 0042
§C4 *also* makes a `GENERATION` evaluation **immutable for the life of the timetable**, and
`record_revalidation` touches only the `REVALIDATION` row. So: the evaluation row travels
with its scores, its findings do not, and nothing can ever put them back — the two partial
unique indexes make a second `GENERATION` row unrepresentable. Every transferred timetable
would show a generation score of `hard −400` beside an empty findings list, permanently.

The derivation argument is weak even for `REVALIDATION`: "reproduces them exactly" holds only
while the inputs have not moved, which is what `inputs_watermark` exists to say cannot be
assumed. **Resolved by approved decision 3.**

#### 1.4.2 Omitting `change_log` makes two exported columns lie

Written in Phase 1, before `Watermark` existed. Since M7, `solve_run.inputs_watermark` and
`timetable_evaluation.inputs_watermark` point into the feed. Without it: either the importer's
own `INSERT`s fill the feed (so a thirty-year-old school's rows all look created today by this
installation, and the stored watermarks sit inside an unrelated sequence), or the feed is empty
(and every stored watermark is in the future, making "N changes since" negative).
**Resolved by approved decision 2.**

#### 1.4.3 `attachments/` has nothing to hold

PROJECT-FORMAT §1 lists `attachments/ — school logo, report templates, user notes`. ADR 0016,
accepted later, put the logo in a `school_logo` row and V8's migration says *"a future
`.evara` export reads this table like any other; nothing extra is needed to carry the image."*
Report templates and user notes do not exist. **Removed from v1, reserved by name** so a
future `formatVersion` can add it without a rename.

#### 1.4.4 `based_on_timetable_id` is a self-reference

V18's lineage column means a JSON importer would need a topological order *within* the
`timetable` table, not merely between tables — and lineage is not guaranteed to agree with
`created_at` once duplication and repair chains are involved. Under the approved snapshot
payload the problem does not arise: no row is inserted.

#### 1.4.5 ADR 0004's rejection of a SQLite payload rested on a cost already paid

ADR 0004 rejected a snapshot because it *"inherits the migration burden"*. It does not:
`CLAUDE.md` §3 makes a released migration immutable, V1…V18 are carried for the life of the
product because every live `.evaraproj` must keep opening, and `Database::open_existing`
already *is* "a v1 file opens in a v5 app". Its second reason — *"tempts readers to query the
snapshot directly"* — is answered structurally by D12 not creating that API.
**Resolved by approved decision 1.**

#### 1.4.6 Nothing proves a historical project opens in today's build

`find . -name '*.evaraproj'` outside `target/` returns nothing. The only historical coverage is
`tests_multi_school::write_a_pre_v12_project`, which uses `run_up_to(11)`. The compatibility
claim has no test behind it for either candidate design.

**Resolved by a generated matrix** (T-41): for each `n` in V1…V18, build a project at `n` with
`run_up_to`, fill it with everything that schema can hold, export, import into the current
build, assert. No binary fixture enters git — matching this repository's practice of generating
fixtures deterministically — and the matrix extends itself when V19 lands.

#### 1.4.7 "Rebuild derived state" would let a sound import fail

PROJECT-FORMAT §5 step 6 says to rebuild score and violations. Re-scoring needs a compiled
`Problem`, which can legitimately fail (ADR 0033, ADR 0037) for a school mid-setup — an
ordinary state that must still transfer — and it *writes* a `REVALIDATION` row, which is not
the `GENERATION` row §1.4.1 is about. `timeslot` integrity needs no rebuilding either:
ADR 0011 makes materialisation id-preserving and the slots travel with their ids.
**An import rebuilds nothing** (D7).

#### 1.4.8 `evara-db`'s project fixtures are `#[cfg(test)]`

`scheduling/tests/seed.rs` builds twelve synthetic schools through the ordinary repository API,
including a `medium` at 125 sections and 500 occurrences, and is unreachable outside
`evara-db`'s test binary. `evara-core` solved the same problem with a `fixtures` feature.
**`evara-db` gains one**, off by default — a prerequisite of the test matrix, not an
afterthought.

#### 1.4.9 M10 is the first milestone that actually reads an archive

`zip 8.6.0` was reached through `calamine` and `rust_xlsxwriter`; **RUSTSEC-2026-0367, -0368
and -0369** were open — three, not the two this note originally recorded (E-24) — and CI's
licence job had been red on `develop` since ~2026-10-20. The triage note that made it
tolerable — *"Evara never calls the vulnerable extract paths"* — stops applying when M10 links
a ZIP reader.

**Closed by Stage P, merged as PR #35 on 2026-11-01.** `zip` is now `9.1.3`, `8.6.0` is out
of the tree (E-25), all three advisories are resolved, `deny.toml` is untouched, and
`cargo deny check` reports *advisories ok, bans ok, licenses ok, sources ok* (E-23). What the
old triage note asserted as a reason not to act is nevertheless still true, and is now a
design property rather than an excuse: D6 never calls `extract()`, which is where all three
advisories live. [§C.8](#c8-finding-6--dependency-upgrade-feasibility-resolved-and-delivered).

#### 1.4.10 `Database::read` opens no transaction

`read` hands over the connection in autocommit, so PROJECT-FORMAT §4's *"open a read
transaction"* does not exist and fifty sequential table reads would be fifty snapshots.
`VACUUM INTO` is the answer both designs need.

#### 1.4.11 "Bytes, never a path" is a spreadsheet rule, not a project rule

`commands/import.rs` takes base64 bytes because *"the application declares no filesystem
capability"*. Sound for an `<input type="file">` spreadsheet; wrong for a 100 MiB archive,
which base64 inflates by a third. The precedent to follow is `project_create` /
`project_open`: a `PathBuf` the user chose in a native dialog, with Rust doing the I/O.

**M10 therefore needs no new Tauri capability.** `capabilities/default.json` holds
`core:default`, `dialog:allow-open`, `dialog:allow-save`, `dialog:allow-message` — all that is
required — so its own comment predicting "file access in Phase 9" should be corrected to say
why it was not needed.

---

# Part B — Recommended export/import design

Thirteen decisions. D1–D5 implement the approved decisions; D6–D13 are the mechanism.

## D1 — The payload is a verified SQLite snapshot

`.evara` is a ZIP with `manifest.json` as its first entry, per-entry SHA-256 in
`checksums.json`, an integer `formatVersion`, a `capabilities` array and an atomic finalise.
Its payload is one entry: a verified snapshot of the project database.

Completeness is structural rather than vigilant — 49 tables and ~450 columns across eighteen
migrations is too large a surface to keep in step by hand, and every future migration widens
it. The migration chain is the one already shipped (§1.4.5). Verification is stronger:
`integrity_check` beats "the JSON parsed", and `foreign_key_check` is a whole-database
referential audit a JSON importer would have to reimplement (E-06 shows it catches what
`integrity_check` misses). Provenance survives exactly — `rev`, `created_at`, `updated_at`,
every UUID, and a change feed whose sequence continues monotonically.

What is lost, stated: a snapshot is queryable with any SQLite tool but not **diffable** in a
git review. `manifest.payload.kind` preserves ADR 0004's storage-decoupling hedge as a
declared, versioned fact, so a future `formatVersion 2` can add `"json-sections"` and read
both.

**Rejected:** renaming the `.evaraproj` to `.evara` — it would make the two extensions two
names for one thing, which `CLAUDE.md` §3 forbids, and the ZIP wrapper is what carries the
manifest, the capability refusal and the integrity record. A raw snapshot with no container —
same reasons. Protobuf / MessagePack — opaque at exactly the moment it matters.

## D2 — Two version axes, one refusal each

| Axis | Lives in | Means | Refusal |
| --- | --- | --- | --- |
| `formatVersion` | `manifest.json` | The **container**: entry layout, manifest schema, payload kind. Integer from `1`. Its meaning may never change, because every future reader parses it before it knows how to read anything else. | Newer than supported → refuse, naming the Evara version needed. Older → run the container migration chain. |
| `payload.schemaVersion` | `manifest.json`, authoritatively in the payload's `refinery_schema_history` | The **data**: which migration the payload was written at. | Newer than `supported_schema_version()` → `DbError::SchemaTooNew`, already implemented and already a tagged frontend error. Older → `migrations::run` on the staged copy. |

The manifest's copy exists to refuse **early and cheaply**, before the payload is read at all;
the payload's history table is the authority. A disagreement between them is itself a refusal
(`ManifestDisagreesWithPayload`), because it means the archive was not assembled by Evara.
`generator.version` travels as diagnostics and decides nothing.

## D3 — `capabilities` is derived from the data, never hand-maintained

`schemaVersion` subsumes most of ADR 0004's capability job, but not all, and the remaining case
is concrete. ADR 0039 ships constraint types in the **registry**, not in a migration, so a
build can add a tenth `type_key` with no schema change. `Constraints::create` refuses an
unimplemented key, but `Constraints::stored_for` deliberately passes a stored one through
untouched — *"a value this build cannot read becomes a named error from the registry rather
than a row quietly dropped"*. Such a project would import perfectly into an older build and
then fail the moment anybody generates or evaluates.

So `capabilities` stays, **computed from the payload at export time**, because a hand-kept
list goes stale exactly when it matters:

```json
"capabilities": [
  "constraint:room-clash", "constraint:teacher-clash", "constraint:activity-fixed",
  "cycle-kind:CUSTOM", "lineage:repair", "lock:placement", "logo:embedded"
]
```

`constraint:<type_key>` per distinct `constraint_instance.type_key`, checked against
`evara_core::scheduling::ConstraintType::from_key` — which is in `evara-core`, so
`evara-project` needs no `evara-constraints` edge. `cycle-kind:<kind>` per distinct
`cycle.kind`. Feature markers for what a reader must understand to render the project
honestly. An unknown entry is a **refusal** naming the capability. The vocabulary is
namespaced so a future prefix is a known-unknown rather than a parse ambiguity.

## D4 — The change log travels complete

Approved decision 2. Under D1 this needs no code: the feed is in the snapshot, `seq` continues
monotonically because `sqlite_sequence` travels, and nothing writes to a tracked table during
import. The manifest records `changeLog.entries` and `changeLog.watermark` so a user sees what
they are carrying, and the export reports the feed's share of the payload (T-60) — the one
measurement that decides whether A6's option is ever needed. A feed with a hole is worse than
a full one or none, because `seq` continuity is what makes it readable; so truncation is not
an option, only omission with the omission declared.

## D5 — Every evaluation, finding, lock and lineage edge travels

Approved decision 3, and it amends PROJECT-FORMAT §3, TIMETABLE-PERSISTENCE §7's table and
ADR 0042 §C4's table. What it does **not** change: a stored `violation` row still carries no
prose (ADR 0043), so a transferred finding is rendered by the *importing* build's own
`explain()` over the stored structure. The wording follows the reader; the fact follows the
archive.

## D6 — Nothing is ever extracted to a path taken from the archive

1. The reader opens exactly **three** entries, **by exact name**: `manifest.json`,
   `checksums.json`, `payload/project.evaraproj`. It never iterates the central directory to
   decide what to write, and never calls an extract-to-directory API.
2. The two JSON entries are read into **memory** under hard byte caps and never touch disk.
3. The payload streams to a file whose path **Evara chose** — a staging file in a staging
   directory Evara created — so no byte of archive content contributes to any path.
4. Any entry name that is not one of the three is **ignored and reported** (ADR 0004's
   forward tolerance) *unless* it is unsafe — absolute, drive-qualified, containing `..`, a
   backslash or a NUL, or longer than 128 bytes — in which case the **archive is refused**.
   An archive containing a traversal attempt is not one to read the rest of.
5. Symlink and directory entries are refused; no entry's attributes are honoured.
6. **The path a payload is opened by is a plain file path, and that is asserted rather than
   arranged by a flag** — revision 2.3, finding **F-2**. `SQLITE_OPEN_URI` is not requested,
   and clearing it achieves nothing in this build: `-DSQLITE_USE_URI` makes URI
   interpretation the global default (E-14, proven behaviourally as **E-33**). The control is
   point 3 above — no byte of archive content contributes to any path — plus
   `require_plain_database_path`, which refuses a `file:` prefix, a `?`, a `#`, a `%` escape,
   an interior NUL and a `..` component at every point a payload is opened. §C.5 has the
   reasoning.
7. **Observing the disk never changes it.** Deciding whether a path holds a project is done
   from its **sixteen-byte header** before anything is opened
   (`evara_db::transfer::has_database_header`). Two mutations reached through the earlier
   "just open it and see" form, and Stage 2 found both: `PRAGMA journal_mode = WAL` on a
   **zero-length** destination *creates a database there*, so the `.replaced-` copy the
   protocol kept was Evara's own four-kilobyte header rather than the user's file; and opening
   a path with a stray `-wal` beside it makes SQLite recover the log and then **remove it**,
   which is the one act [§0.2 S3](#s3--a-main-file-must-never-be-paired-with-another-databases-sidecars)
   forbids outright. S2's reconciliation rests on the observation being passive, so this is
   not a tidiness point. **E-34**.

Traversal is unexpressible rather than defended against, which is why §1.4.9's advisory class
stays inapplicable after M10 links a ZIP reader. ADR 0055.

## D7 — Import stages, verifies, then renames

```
destination:  …/Documents/Northgate.evaraproj
staging:      …/Documents/.evara-import-<uuid7>/staged.evaraproj
aside:        …/Documents/.evara-import-<uuid7>/aside/     (Replace mode only)
```

1. Validate the destination path and claim it, or refuse — D11 P0–P1.
2. Create the staging directory as a **sibling of the destination**, so every later rename is
   intra-directory on one volume. Journal the intent — D11 P2.
3. Stream the payload into staging under `manifest.payload.byteLength`, hashing; verify length
   and digest. Mismatch → refuse, nothing created.
4. **Verify before any write** — §6.1 phase V0.
5. **Migrate** — `migrations::run` in its grouped transaction, then `change_log::install_all`.
   A failure leaves staging untouched and the destination never existed.
6. **Verify after migrating** — §6.1 phase V2: `foreign_key_check` empty, conformance at the
   current version, census, capability check.
7. Close, then rename into place — D11 P4 or P4′.
8. Register in the recent list **without opening** — D9.

**The import takes no safety backup of the staged file, deliberately.**
`backup::before_schema_rebuild` fires on any open below V12 and would litter staging with a
`.pre-vN.evaraproj`. On this path it is redundant: the archive is an untouched, checksummed
copy of the source, which is a better backup than a `VACUUM` of a throwaway. The transfer
module migrates the staged copy without the pre-rebuild hook. ADR 0057.

**An import rebuilds no derived state** — §1.4.7. Scoring a transferred timetable is an
ordinary `timetable_revalidate` the interface offers, and its failure is a finding about the
project rather than a failed import.

## D8 — Export never mutates the source, and the lock is held for one statement

```
command thread:   lock workspace
                  snapshot  →  <staging>/snapshot.evaraproj     (mechanism per §C.6)
                  one agreement check: COUNT(*) FROM school, source vs copy
                  release the lock
──────────────────────────────────────────────────────────────────────────────
worker thread:    open the SNAPSHOT hardened-read-only; read every manifest fact from it
                  SHA-256 the snapshot
                  write manifest.json, checksums.json, payload/… into <dest>.evara.part
                  rename  <dest>.evara.part → <dest>.evara
                  delete the snapshot
```

> **Amended in revision 2.2.** The heading's *"one statement"* is kept so existing links
> still resolve, but the mechanism is **one pinned read transaction made of many steps**, not
> one statement: §C.6 records why the two mechanisms became one and what it costs. The
> duration claim below is unchanged — the lock still covers the whole copy and one `COUNT`.

- **No write to the source.** The snapshot reads; nothing re-opens the project.
- **One mechanism, both paths.** `BEGIN DEFERRED`, whose first read pins the WAL snapshot, then
  a stepped `Backup` to completion (§C.6, E-27). `VACUUM INTO` is not on this path.
- **Consistency.** From the pinned read snapshot the first read takes (E-27), identically on
  both paths.
- **The payload is not compacted.** The backup API copies page for page, free pages included,
  so a heavily edited project's snapshot is larger than a vacuumed one would be — §7.2.
- **All manifest facts come from the snapshot**, read on the worker thread after the lock is
  released. Finding 4.
- **The lock covers exactly the snapshot plus one `COUNT`.** Budget ≤ 500 ms at the realistic
  upper end (T-83, measured in **Stage 2**). If it exceeds that, the remedy is a second
  `SQLITE_OPEN_READ_ONLY` connection (E-03), not a longer lock.
- **Atomic finalise.** `<dest>.evara.part` beside the destination, then rename — ADR 0004's
  clause, kept. Evara never writes over an existing `.evara` directly, so a failure mid-write
  leaves the previous archive intact (T-47).
- **Cancellation.** An `AtomicBool` behind `Cancel`, read by the ZIP writer between buffers and
  by the snapshot **between page batches** — not through `progress_handler`, which E-32 rules
  out for a borrowed `&dyn Cancel`. On cancel or failure: the snapshot, the staging directory
  and the `.part` file are removed and the destination is never created. **E-01's 0-byte
  leftover is removed explicitly**, in `Snapshot`'s `Drop` so no call site has to remember —
  the same gap E-22 found in `backup.rs`. A cancel after the finalise returns `false` and
  changes nothing.
- **`Snapshot` is `Send`** and owns its path and facts, holding no connection — the property
  that lets it cross to the worker, asserted by a `send_assertions` test mirroring
  `evara_db::scheduling`'s.

## D9 — The transfer session is the generation session's state machine

Four of `commands/session.rs`'s five hazards are transfer's too:

| Hazard | Guard |
| --- | --- |
| Finalising an unfinished run | `Running` refuses |
| Two finalises from two clicks | `Finishing` claimed by compare-and-swap; `Finished` terminal |
| A cancel racing a finish | `request_cancel` sets a flag and sets no state |
| Work applied to a different project | `ProjectEpoch`; `project_create` / `open` / `close` already bump it and clear every session set |

`AppState` gains `transfers: TransferSessions`, cleared on create/open/close exactly as
`imports`, `generations`, `edits` and `repairs` are.

**Import does not open what it imported.** Auto-opening would close whatever the user had
open, discarding an edit session, a generation and a repair, as a side effect of an import.
The interface offers "Open now" — one click, the user's decision. That needs one new public
method, because `Settings::remember_project` is reached today only through `adopt`:

```rust
impl Workspace {
    /// Records a project in the recent list without opening it.
    pub fn remember_imported_project(&mut self, path: &Path, project_id: Uuid) -> Result<()>;
}
```

## D10 — Schema conformance is checked against a generated reference

Hand-listing expected objects is not viable (74 migration triggers, ~114 change-log triggers,
49 tables, dozens of indexes) and would go stale on every migration. So it is computed:

```
expected(n) := sqlite_master of a project built in a temp file by
               migrations::run_up_to(conn, n)  +  change_log::install_all
```

The staged payload's `sqlite_master` must equal `expected(declared schemaVersion)` as a set
over `(type, name, tbl_name, normalised sql)` **before anything writes to it**, and
`expected(supported)` after `migrations::run`. Any extra, missing or divergent object is a
refusal naming it. This is the same shape as the schema-level test that discovers entity
tables and fails if any lacks its three triggers: a property computed from the schema, so
forgetting is a red test rather than a silent hole.

**It is not a sandbox**, and §6.2 says what it does not cover. ADR 0056.

## D11 — Replacement is journalled, non-destructive and recoverable

The protocol, in full. It replaces revision 1's one-line "stage then rename".

### The journal

A settings-database table — **not** a project migration (E-21):
`settings-migrations/V2__pending_replacement.sql`.

```sql
CREATE TABLE pending_replacement (
    id           TEXT    PRIMARY KEY NOT NULL,         -- UUIDv7
    phase        TEXT    NOT NULL CHECK (phase IN ('STAGED', 'ASIDE', 'PLACED')),
    destination  TEXT    NOT NULL CHECK (length(trim(destination)) > 0),
    staging_dir  TEXT    NOT NULL CHECK (length(trim(staging_dir)) > 0),
    keep_name    TEXT             CHECK (keep_name IS NULL OR length(trim(keep_name)) > 0),
    started_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL
) STRICT;

-- At most one in-flight replacement per destination: two concurrent replacements of one
-- file is not a state the protocol has an answer for, so uniqueness turns a race into a
-- refusal. `destination` is NOT NULL, so this needs no COALESCE.
CREATE UNIQUE INDEX pending_replacement_destination_unique
    ON pending_replacement (destination);

-- Recovery reads every row at startup, oldest first.
CREATE INDEX idx_pending_replacement_started ON pending_replacement (started_at);
```

The settings database is the right home: Evara owns it, it is transactional, it is WAL, and
it is the only store that is reachable at startup without knowing which project to look at.
**Every phase row is committed before the rename that advances it**, which is what makes
recovery deterministic rather than a guess.

**Reading a damaged row** — revision 2.2, from Stage 1's review. Two cases the first draft of
this section did not state, both now decided and tested:

- An `id` that will not parse as a UUID is **`DbError::CorruptIdentifier`**, never a
  substituted `Uuid::nil()`. `CLAUDE.md`'s rule is general, and it bites hardest here: a nil
  fallback would make every damaged row compare equal to every other, and recovery compares
  rows. `a_journal_row_whose_id_will_not_parse_is_corruption`.
- A `phase` this build does not recognise — one a **newer** Evara wrote — is carried as an
  unknown rather than coerced to `STAGED`, and **acted on by nothing**. The `CHECK` keeps the
  three known spellings exclusive within a build, so an unrecognised value can only have come
  from a build that knows more than this one; guessing its intent is how a newer protocol's
  half-finished replacement gets completed wrongly by an older binary.
  `a_phase_a_newer_build_wrote_is_carried_as_unknown`, and
  `an_unrecognised_phase_is_reported_and_acted_on_by_nothing`.

### Sidecars are one set

A project is up to four files: `X.evaraproj`, `X.evaraproj-wal`, `X.evaraproj-shm`, and
`X.evaraproj-journal` if an older build left one. **A new main file paired with the old
`-wal` is corruption that opens cleanly** — precisely the failure `backup.rs`'s header
describes. So the aside move takes the whole set, together, and the destination's set is never
mixed.

> **⚠ S3 — this paragraph is wrong as written, and [§0.2 S3](#s3--a-main-file-must-never-be-paired-with-another-databases-sidecars)
> supersedes it.** There is no atomic multi-file move on either platform, so "together" is
> not something a rename can deliver: a crash between moving the main file and moving `-wal`
> leaves the old `-wal` at the destination's name for the new main to be renamed beside.
> The resolution is to **checkpoint and close each side so it is a single file before it
> moves**, and to assert no sidecar exists at the destination before the final rename.
> Implement §0.2 S3, not this paragraph.

### The phases

**P0 — validate the path.** `symlink_metadata` on the destination and every ancestor; refuse
if any is a symlink or a Windows reparse point (`DestinationPathUnsafe`). Refuse a destination
that is not `.evaraproj` (`require_project_extension`). Refuse if the destination is the
**currently open project** (`DestinationIsOpen`) — the remedy is to close it first.

**P1 — claim or decide.**

- Destination absent, or present and **zero length**: claim it atomically with
  `OpenOptions::new().write(true).create_new(true)`. This wins the TOCTOU race against another
  process creating the same name, and a 0-byte file is harmless — `looks_like_a_project`
  rejects it and `Database::create` treats it as creatable. Mode = **Create**.

  > **⚠ S1 — this bullet is self-contradictory and
  > [§0.2 S1](#s1--create_newtrue-contradicts-adopting-a-zero-length-destination)
  > supersedes it.** `create_new(true)` fails when the path exists, zero length included, so
  > it cannot claim an existing empty file. The Create path narrows to a destination that
  > does **not exist**; an existing zero-length file is either our own abandoned claim (which
  > S2's reconciliation owns) or occupied and needing explicit `Replace`. Implement §0.2 S1.
- Destination present and non-empty, `OnConflict::Refuse`: `ProjectAlreadyExists`, naming the
  file. The interface then offers "Choose another name" or "Replace", the latter being a
  second command call with a second confirmation that names the file and the keep-name.
- Destination present and non-empty, `OnConflict::Replace`: Mode = **Replace**.

**P2 — stage.** Create `<dest dir>/.evara-import-<uuid7>/`. Insert the journal row at
`STAGED`. Stream, verify, migrate, verify (D7 steps 3–6). Nothing outside staging has been
touched except the Create-mode 0-byte claim.

**P3 — Create mode finalise.** Rename `staging/staged.evaraproj` → destination, replacing our
own placeholder. Delete the staging directory. Delete the journal row. Done — one rename.

**P4 — Replace mode finalise**, four steps, each after its journal commit:

1. Create `staging/aside/`. Move the destination's whole file set into it. Journal → `ASIDE`.
2. Rename `staging/staged.evaraproj` → destination. Journal → `PLACED`.
3. Move the aside set to `<dest dir>/<stem>.replaced-<ISO-stamp>.evaraproj` (+ its sidecars
   under the matching names, so the kept copy is readable on its own).
4. Delete the journal row; remove the staging directory.

### Backup and rollback

**Evara never deletes the project it replaced.** Step 3 keeps it as
`<stem>.replaced-<stamp>.evaraproj`, following `backup.rs`'s rule that *"nothing in Evara ever
deletes it"*. So "a failed import must not destroy the previous project" is not a promise
about error handling — **there is no code path that deletes it at all**, success or failure.
The interface tells the user where the kept copy is, as it already does for a schema-rebuild
safety copy.

### Crash recovery

> **⚠ S2 — the table below is unsafe as written and
> [§0.2 S2](#s2--recovery-must-reconcile-the-journal-against-the-filesystem-not-trust-it)
> supersedes it.** It maps a journal phase straight to an action, which assumes the phase
> describes the disk. The row is committed **before** the rename it authorises, so a crash in
> that window leaves the phase ahead of the filesystem — and following the `PLACED` row then
> moves the user's only project to a `.replaced-` name and leaves no destination at all.
> §0.2 S2's reconciliation probes the filesystem and treats the journal as **intent**, under
> one dominating invariant: **the old set is released only once a destination that
> `looks_like_a_project` has been observed.** Implement §0.2 S2.

Run at `Workspace::new`. The phase below is the *intended* state, and an implementation must
reconcile it against what is on disk rather than trusting it:

| Phase found | Intended state on disk | Recovery | Direction |
| --- | --- | --- | --- |
| `STAGED` | destination untouched, or holds our 0-byte claim | delete the staging directory; delete the claim if it is still 0 bytes | roll back |
| `ASIDE` | destination missing, old set in `staging/aside/` | move the old set back to the destination | roll back |
| `PLACED` | destination holds the new project, old set in `staging/aside/` | finish P4 step 3 — move the aside set to the keep-name | complete |

No user decision is needed for any state §0.2 S2's table resolves; an **ambiguous** one is
reported and acted on by nothing. Recovery reports what it did through one command
(`project_recovery_report`) so an interrupted import is explained rather than mysterious.

**Abandoned staging with no journal row** — another installation's live staging, or a journal
lost to a power cut — is **reported and never auto-deleted**. Deleting a directory that might
be another process's work in progress is worse than leaving it.

### Guarantees relied on, and guarantees not claimed

**Relied on.** A single rename within one directory on one volume is atomic for other
observers: either the old name or the new name resolves. The destination is only ever written
*by a rename*, never by a stream — which is what makes a partially replaced destination
unreachable. Staging is a sibling of the destination, so no rename crosses a volume.

**Not claimed — durability.** `std::fs` cannot `fsync` a directory handle on Windows, so after
a power loss the filesystem may present either side of a rename, and the journal row may
itself be lost. The claim is deliberately weaker and sufficient: **the set of reachable states
is `{old project intact}` or `{new project in place, old project kept beside it}`.** Never a
partially replaced destination. Where the journal row is lost, the orphaned staging directory
is reported (above) rather than acted on.

**The platform asymmetry, named.** On Windows, a rename over a file another process holds open
**fails** — SQLite opens without `FILE_SHARE_DELETE`, so `MoveFileExW` returns a sharing
violation. On macOS/APFS, `rename(2)` over an open file **succeeds** and the holder keeps
writing to an orphaned inode. Evara closes its own handle (P0 refuses if the destination is
the open project) and **cannot** detect another installation's. That residual risk is
documented; the kept `.replaced-` copy is what makes it survivable rather than silent data
loss.

**Antivirus and indexer interference.** Both platforms can hold a newly written file open
briefly. Every rename gets a bounded retry — five attempts, 100 ms apart — then a named error
(`RenameBlocked`) and automatic rollback to the previous phase. Not an unbounded loop: a
permanently locked destination is a condition the user must be told about.

**Disk exhaustion.** No pre-flight free-space check (§C.4). `ENOSPC` becomes
`InsufficientSpace` with a remedy, and the protocol's structure means it is never a partial
replacement.

## D12 — The engine is in `evara-db`, the format is in `evara-project`

**`evara-db` gains `pub mod transfer`**, because every operation it performs is behind
`pub(crate)`:

| Needs | Why only `evara-db` can |
| --- | --- |
| `VACUUM INTO` / the backup API on the live connection | `Database` never hands out a `Connection` |
| the hardened read-only constructor | one factory, `pub(crate)` |
| `migrations::run`, `migrations::run_up_to` | `pub(crate)` — and `run_up_to` is what makes D10 and T-41 possible |
| `change_log::install_all`, `change_log::watermark` | `pub(crate)` |
| `sqlite_master` and both PRAGMA audits | SQL lives in exactly one crate |

**`evara-project` owns the format**, keeping its declared edge (`-> core, db`): the manifest
type and its serde mapping, SHA-256, the ZIP container, the entry-name policy and byte caps,
the container migration chain, the staging lifecycle, the renames, and the orchestration that
calls `evara-db`'s transfer API in order. In **Stage 2** it gains a **direct** `zip` dependency
(`default-features = false, features = ["deflate-flate2"]`) and `sha2`. Stage P already pinned
the version it will resolve to — **`zip 9.1.3`**, with `8.6.0` out of the tree and all three
RUSTSEC advisories resolved (E-23, E-24, E-25) — so the floor is in place before the reader
is written, which was the whole point of ordering Stage P first. `evara-project` has **no**
`zip` dependency today.

**Fallback if `zip 9`'s reader disappoints** (§C.8): write the three-entry container with
`flate2` + `crc32fast`, both already in the tree, adding no dependency and no advisory
surface. Not recommended — reading a third party's ZIP is the risky half and `zip` does it
properly — and no longer needed for *feasibility*, which Stage P settled.

**`evara-core` gains nothing** except, pending A1, the `Cancel` trait that two crates already
need. A portable archive is not a vocabulary four crates share — ARCHITECTURE §3 rule 1's
test — and ADR 0037 §2's refusal to widen `Problem` for a diagnostic-only field binds a
transfer-only one identically.

```
src-tauri ──► evara-project ──► evara-db ──► evara-core
                    └─────────────────────────────┘
```

No cycle, no new edge into `evara-core` from the format, and `evara-constraints`,
`evara-solver` and `evara-report` are untouched. `evara-project` must **not** gain
`evara-constraints`: the capability check uses `evara_core::scheduling::ConstraintType`.

## D13 — Identity is preserved, and duplication is disclosed

**`project_id` is never regenerated.** ADR 0004's rule; regenerating would destroy round-trip
comparison and any future synchronisation. Evara cannot merge two copies, so the remedy for a
duplicate is disclosure, not prevention.

**Detection.** `Settings` gains a query over the column E-20 shows is already there:

```rust
impl Settings {
    /// Recent entries carrying this project identity. Empty when the project is new here.
    pub fn projects_with_identity(&self, project_id: Uuid) -> Result<Vec<RecentProject>>;
}
```

`project_import_inspect` returns the result as `identityConflicts`, so the warning appears
**before** anything is staged.

**Four behaviours.**

1. **Import to a different path, identity already present here** — allowed, with an explicit
   warning first:
   > *"A campus with this identity is already on this machine: **Northgate.evaraproj** in
   > Documents. Importing creates a second copy. Evara cannot merge two copies, so changes you
   > make in one will not appear in the other."*
2. **Import over that same path (Replace)** — D11's non-destructive protocol, with a sharper
   sentence:
   > *"This replaces **Northgate.evaraproj**, which is the same campus. Your current copy is
   > kept beside it as **Northgate.replaced-2026-10-29T09-14-22Z.evaraproj**."*
3. **The twin is currently open** — refused (`DestinationIsOpen`), remedy named:
   > *"**Northgate.evaraproj** is open. Close it before replacing it."*
4. **No conflict** — import silently, as now.

**Ambiguity is visible everywhere, not only at import.** `RecentProject` gains a derived
`duplicate_identity: bool`, computed in `recent_projects()` by grouping on `project_id`, so
the recent list itself shows that two entries are the same campus. This is the fix for E-20's
real problem: today nothing would notice.

**Not done.** No uniqueness constraint on `recent_project.project_id` — two legitimate copies
must both be listable. No merge (A7).

Tests T-63…T-67.

---

## 3. The proposed file format

### 3.1 Container

```
Northgate.evara                        ZIP, deflate
├── manifest.json                      STORED (uncompressed), first local header
├── checksums.json                     DEFLATE
└── payload/project.evaraproj          DEFLATE   — the verified snapshot
```

- Exactly three entries in v1. Names are ASCII, lowercase, `/`-separated, relative, no `..`,
  no backslash. Unknown-but-safe extra entries are ignored and reported; unsafe names refuse
  the archive (D6).
- `manifest.json` is `STORED` so it is readable with no decompressor and cheaply — it is
  parsed before anything else is trusted.
- Compression method must be `Stored` or `Deflate`. Any other is refused, which closes off
  zstd / bzip2 / lzma / ppmd decoders and their bombs without reasoning about them.
- Every entry timestamp is a **fixed constant** (the ZIP epoch), not the clock, so two exports
  of an unchanged project differ only in `manifest.json`. Both JSON entries are written
  **LF-only**, so their digests are platform-independent.
- ZIP64 is enabled for payloads ≥ 4 GiB; a project that large is refused on the byte cap
  first.
- `.evara` and `.evaraproj` remain different kinds of object, and
  `require_project_extension` already rejects `.evara` as a live project with a test asserting
  the two constants differ.

### 3.2 `manifest.json`

```json
{
  "format": "evara-project",
  "formatVersion": 1,
  "payload": {
    "kind": "sqlite-snapshot",
    "entry": "payload/project.evaraproj",
    "schemaVersion": 18,
    "byteLength": 6291456
  },
  "generator": { "app": "Evara Timetables", "version": "0.1.0" },
  "projectId": "018f3a2c-7c4e-7a19-9f2b-3c1d5e7a9b11",
  "createdBySiteId": "018f3a2c-7c4e-7a19-9f2b-000000000001",
  "exportedBySiteId": "018f3a2c-7c4e-7a19-9f2b-000000000002",
  "exportedAt": "2026-10-29T09:14:22.113Z",
  "changeLog": { "entries": 48210, "watermark": 48210 },
  "schools": [
    { "id": "018f…", "name": "Northgate Secondary (synthetic)", "campuses": 2 }
  ],
  "counts": {
    "schools": 2, "campuses": 3, "rooms": 61, "teachers": 84, "students": 1180,
    "studentGroups": 47, "subjects": 22, "courses": 58, "classSections": 164,
    "activities": 1320, "constraintInstances": 29, "timeslots": 300,
    "timetables": 3, "timetableEntries": 3810, "solveRuns": 4, "violations": 212
  },
  "capabilities": [
    "constraint:room-clash", "cycle-kind:CUSTOM", "lineage:repair", "lock:placement"
  ],
  "integrity": { "algorithm": "sha256", "entry": "checksums.json", "sha256": "4c7e…" }
}
```

- **`payload.sha256` is deliberately absent** — it lives in `checksums.json` only, so there is
  one place for one fact (§C.3). `payload.byteLength` stays, because the payload read must be
  bounded before any digest is known.
- Every fact here is read **from the snapshot**, not from the source project (finding 4, D8).
- `counts` is advisory: a disagreement with the payload is **reported, not refused**, because
  the payload is the authority and a count is a convenience. The one exception is
  `payload.schemaVersion`, which *is* a refusal on disagreement (D2).
- No real school, teacher or student name appears in any fixture or example, here or in CI.

### 3.3 Integrity model

Two levels, one root, one digest per fact. **Acyclic by construction.**

```
manifest.json  ──integrity.sha256──►  checksums.json  ──entries{}──►  payload/project.evaraproj
manifest.json  ──payload.byteLength──►  the bound on the payload read
```

```json
{
  "algorithm": "sha256",
  "entries": { "payload/project.evaraproj": "9f2b…" }
}
```

`manifest.json` is **not** listed in `checksums.json`, and `checksums.json` carries no digest
of the manifest.

### Verification order — each step gates the next

| # | Step | Refusal |
| --- | --- | --- |
| 1 | Locate `manifest.json` by exact name | `ManifestMissing` |
| 2 | Read ≤ **64 KiB** through a limited reader | `EntryTooLarge` — refused *before* being parsed |
| 3 | Parse; check `format` and `formatVersion` | `ManifestUnreadable`, `UnknownFormat`, `FormatTooNew` |
| 4 | Locate `checksums.json`; read ≤ **1 MiB**; SHA-256 it; compare with `manifest.integrity.sha256` | `EntryMissing`, `EntryTooLarge`, `ChecksumMismatch` |
| 5 | Parse it; require `algorithm == "sha256"` exactly | `UnsupportedDigest` |
| 6 | Scan entry names; refuse unsafe ones; record unknown-but-safe ones | `UnsafeEntryName` |
| 7 | Check `payload.schemaVersion` against `supported_schema_version()` | `SchemaTooNew` — **before** a byte of payload is written |
| 8 | Check `capabilities` | `UnsupportedCapability` — also before any payload write |
| 9 | Stream the payload into staging through a reader limited to `payload.byteLength`, hashing as it goes | `EntryTooLarge` (more bytes than the bound), `PayloadLengthMismatch` (fewer), `ChecksumMismatch` |
| 10 | Only now is the payload a file worth opening — §6.1 | — |

Steps 7 and 8 sit before step 9 deliberately: an archive this build cannot use is refused
without writing anything at all.

### Corruption is detected; authenticity is not

The manifest is the **deliberately unverified root**. A self-describing archive cannot verify
its own root without a key, and ADR 0001 forbids the service that would distribute one.

- **Detected:** truncation, bit rot, a bad copy, a partially written archive, a mismatched or
  substituted payload, a corrupted `checksums.json`.
- **Not detected:** a deliberately rewritten archive. Anyone can edit the manifest and
  recompute every digest below it. A `.evara` carries exactly the authority of the person who
  sent it, and the import screen says so — not only this document. R11.

`integrity.algorithm` admits one value; a different one is a refusal rather than a
negotiation, because an algorithm field that accepts alternatives is a downgrade surface and
with no signature there is nothing to negotiate. A future entry slots into `checksums.json`
with no `formatVersion` bump; a changed **algorithm** bumps it.

### 3.4 Windows / macOS portability

| Concern | Why it holds |
| --- | --- |
| Endianness | The SQLite file format is defined big-endian on disk and is byte-identical across platforms. The payload is portable by construction. |
| WAL sidecars | A snapshot is fully checkpointed and self-contained — `backup.rs`'s own argument. No `-wal` or `-shm` travels, and none is needed to read it. |
| Path separators, case, reserved names, Unicode normalisation | Three fixed lowercase ASCII entry names, `/`-separated. No archive content ever becomes a filename (D6). |
| Long paths on Windows | The staging directory is `.evara-import-<uuid7>` — 45 characters — beside the chosen destination. Named as a risk (§7.4). |
| Line endings | Both JSON entries are LF-only, so digests match across platforms and git cannot rewrite them. |
| Replacement semantics | The Windows/macOS asymmetry is the subject of D11's *Guarantees* section, not an assumption. |
| File associations | Double-clicking a `.evara` needs `bundle.fileAssociations` and single-instance handling — Phase 11 packaging, **not** M10. Recorded so it is not assumed. |

---

## 4. Public Rust APIs and Tauri command contracts

### 4.1 `evara_db::transfer`

> **This block is the design sketch, and Stage 1 is now the authority on the shape** — added
> in revision 2.2, with no decision reopened. The sketch predates both the four Q answers and
> the §C.6 deviation, so four things below read differently in the tree:
>
> - **Progress is a plain callback**, `&mut dyn FnMut(SnapshotProgress)` reporting *pages*, not
>   a `Progress` trait reporting bytes. That was **Q1's approved answer**: a page copy is not a
>   solve's shape, and `evara-solver`'s `ProgressSink` stays where it is rather than widening
>   `evara-core` for one caller.
> - **`snapshot_into` takes a directory and names the file itself**, and the `Snapshot` it
>   returns **removes its own file on drop** unless `keep()` is called, rather than offering a
>   `discard()` a failure path has to remember. The facts are accessors on `Snapshot` —
>   `identity()`, `schema_version()`, `census()`, `watermark()`, `byte_len()` — rather than a
>   `SnapshotFacts` struct; the manifest-shaped summary is assembled in `evara-project`, which
>   is the crate that needs it.
> - **One mechanism, and cancellation is the step boundary**, not `progress_handler` (§C.6,
>   E-32).
> - **`stage` and the free-function `table_census` are not here yet.** `stage` is phase V1 and
>   needs a staging lifecycle to migrate into; `table_census(path)` reads a *destination* for
>   the round-trip assertion. Both are Stage 2, which §8 now says. What Stage 1 provides
>   instead is `UntrustedPayload` + `audit()` + `conformance::check` — phase V0 complete — and
>   a census carried on the `Snapshot`.
>
> The `DbError` table below is likewise the sketch: Stage 1 added `SnapshotCancelled`,
> `UnknownSchemaVersion`, `SchemaDoesNotConform` and `RenameBlocked`, reused the existing
> `BackupRefused` where the sketch said `SnapshotRefused`, and reports corruption and
> referential damage as **audit findings** rather than errors, because V0 reads them and the
> caller composing the protocol is what refuses. The remaining names —
> `DestinationIsOpen`, `DestinationPathUnsafe`, `InsufficientSpace` — belong to the protocol
> and so to Stage 2.

```rust
pub mod transfer {
    /// Cancellation, as a request the engine reads. Pending A1 this is
    /// `evara_core::Cancel`, re-exported by `evara-solver`.
    pub trait Cancel: Send + Sync { fn is_cancelled(&self) -> bool; }

    /// Where a long transfer step has got to.
    pub trait Progress: Send + Sync { fn bytes(&self, done: u64, total: u64); }

    /// A verified, self-contained snapshot of a project, on disk.
    ///
    /// Owns its path and its facts and holds NO connection, which is what lets it move to a
    /// worker thread once the workspace lock is released — the `Assembly` pattern,
    /// ARCHITECTURE §3.3. Asserted by `send_assertions`.
    pub struct Snapshot { /* path, facts; no Connection */ }

    impl Snapshot {
        pub fn path(&self) -> &Path;
        pub fn facts(&self) -> &SnapshotFacts;
        pub fn byte_length(&self) -> u64;
        /// Removes the file. Called on cancel, on failure, and after a successful seal.
        pub fn discard(self) -> Result<()>;
    }

    /// Everything the manifest needs. Read FROM THE SNAPSHOT, never from the source.
    pub struct SnapshotFacts {
        pub project_id: Uuid,
        pub created_by_site_id: Option<Uuid>,
        pub schema_version: u32,
        pub change_log: ChangeLogSummary,     // entries, watermark, payload share
        pub schools: Vec<SchoolSummary>,
        pub counts: TableCounts,
        pub constraint_type_keys: Vec<String>,
        pub cycle_kinds: Vec<String>,
        pub has_repair_lineage: bool,
        pub has_placement_locks: bool,
        pub has_embedded_logo: bool,
    }

    impl Database {
        /// Snapshots this project into `target`, then verifies the copy.
        ///
        /// Writes nothing to `self`. Uses `VACUUM INTO` on a writable connection and the
        /// online backup API inside a read transaction on an inspection handle, because
        /// `VACUUM INTO` is refused under `query_only` (E-01, E-02).
        ///
        /// Holds a read transaction for the duration and no longer. Cancellation is checked
        /// through `progress_handler`; a cancelled or failed run removes `target`, including
        /// the zero-byte file a refused `VACUUM INTO` leaves behind (E-01).
        ///
        /// # Errors
        /// `DbError::SnapshotRefused` if the copy could not be written or verified, in which
        /// case `target` does not exist and the project is exactly as it was.
        pub fn snapshot_into(
            &self, target: &Path, progress: &dyn Progress, cancel: &dyn Cancel,
        ) -> Result<Snapshot>;
    }

    /// Verifies a staged payload and migrates it forward. Nothing else may open it.
    ///
    /// Phases V0, V1, V2 of §6.1, in that order, first failure stopping everything.
    ///
    /// # Errors
    /// `PayloadCorrupt`, `NotAnEvaraProject`, `MissingMetadata`, `SchemaTooNew`,
    /// `SchemaUnexpected`, `ReferentialDamage`, `Migration`.
    pub fn stage(path: &Path, cancel: &dyn Cancel) -> Result<StagedPayload>;

    pub struct StagedPayload {
        pub path: PathBuf,
        pub schema_version_found: u32,
        pub schema_version_now: u32,
        pub migrated: bool,
        pub facts: SnapshotFacts,
    }

    /// Every table in the schema, discovered from `sqlite_master`, with its row count.
    ///
    /// Discovery rather than a list, so a table added by a future migration is covered by
    /// the round-trip assertion without anybody remembering to add it.
    pub fn table_census(path: &Path) -> Result<BTreeMap<String, i64>>;
}
```

New `DbError` variants, each naming a remedy:

| Variant | Message |
| --- | --- |
| `SnapshotRefused { name, source }` | "Evara could not take a complete copy of {name}; nothing was changed." |
| `PayloadCorrupt { detail }` | "The campus inside this archive is damaged ({detail}). Ask for a fresh export." |
| `SchemaUnexpected { object }` | "This archive contains something Evara did not write ({object}) and cannot be imported." |
| `ReferentialDamage { count }` | "The campus inside this archive has {count} broken references. Ask for a fresh export." |
| `DestinationIsOpen { name }` | "{name} is open. Close it before replacing it." |
| `DestinationPathUnsafe { name }` | "Evara will not write to {name} because part of that path is a link." |
| `RenameBlocked { name }` | "Another program is using {name}. Close it and try again." |
| `InsufficientSpace { needed, available }` | "This needs about {needed} and {available} is free." |

### 4.2 `evara_project`

```rust
pub const ARCHIVE_EXTENSION: &str = "evara";
pub const FORMAT: &str = "evara-project";
pub const FORMAT_VERSION: u32 = 1;

pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;
pub const MAX_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 64;

pub struct Manifest { /* §3.2 */ }

/// What an archive says about itself. Reading this writes nothing and stages nothing.
pub struct ArchiveSummary {
    pub manifest: Manifest,
    pub archive_bytes: u64,
    pub unknown_entries: Vec<String>,
    pub readable: Readability,
    /// Recent entries already carrying this `projectId` — D13.
    pub identity_conflicts: Vec<IdentityConflict>,
}

pub enum Readability {
    Readable,
    FormatTooNew { found: u32, supported: u32 },
    SchemaTooNew { found: u32, supported: u32 },
    UnsupportedCapability { capability: String },
}

pub enum Phase { Snapshotting, Hashing, Compressing, Staging, Verifying, Migrating, Finalising }
pub trait TransferProgress: Send + Sync {
    fn progress(&self, phase: Phase, done: u64, total: u64);
}

pub enum OnConflict { Refuse, Replace }

/// Seals an owned snapshot into an archive at `destination`.
///
/// Takes the `Snapshot` BY VALUE and no `&Database`, which is what lets the caller release
/// the workspace lock before this runs — D8, and the fix for revision 1's borrow.
pub fn seal(
    snapshot: evara_db::transfer::Snapshot,
    destination: &Path,
    progress: &dyn TransferProgress,
    cancel: &dyn evara_db::transfer::Cancel,
) -> Result<ExportReport, TransferError>;

/// Reads an archive's manifest and checksums. No staging, no writes, no database.
pub fn inspect(
    archive: &Path, known: &dyn IdentityLookup,
) -> Result<ArchiveSummary, TransferError>;

/// Imports. Stages, verifies, migrates, renames. Never touches an open project.
pub fn import(
    archive: &Path,
    destination: &Path,
    on_conflict: OnConflict,
    progress: &dyn TransferProgress,
    cancel: &dyn evara_db::transfer::Cancel,
) -> Result<ImportReport, TransferError>;

/// Completes or rolls back every interrupted replacement — D11's recovery table.
pub fn recover_pending(journal: &mut dyn ReplacementJournal) -> Result<Vec<Recovered>, TransferError>;

pub struct ImportReport {
    pub path: PathBuf,
    pub project_id: Uuid,
    pub schema_version_found: u32,
    pub schema_version_now: u32,
    pub migrated: bool,
    pub counts: TableCounts,
    pub replaced_copy_kept_at: Option<PathBuf>,
    pub unknown_entries: Vec<String>,
    pub count_disagreements: Vec<CountDisagreement>,
}
```

`TransferError` is a `thiserror` enum, and **no `rusqlite` or `zip` type crosses its
boundary** — `CLAUDE.md`'s driver-type rule applied to a second crate, asserted the way
`evara-db` asserts it, by not having the dev-dependency (T-62):

`NotAnArchive` · `ManifestMissing` · `ManifestUnreadable` · `UnknownFormat { found }` ·
`FormatTooNew { found, supported }` · `ManifestDisagreesWithPayload { field }` ·
`UnsupportedCapability { capability }` · `UnsupportedDigest { algorithm }` ·
`EntryMissing { entry }` · `UnsafeEntryName { entry }` · `UnsupportedCompression { entry }` ·
`EntryTooLarge { entry, limit }` · `TooManyEntries { found, limit }` ·
`PayloadLengthMismatch { declared, actual }` · `ChecksumMismatch { entry }` · `Truncated` ·
`DestinationExists { name }` · `WrongExtension { expected }` · `Cancelled` · `Io { action }` ·
`Database(DbError)`

### 4.3 Tauri commands — **eleven**, counted

| # | Command | Writes? | Returns |
| --- | --- | --- | --- |
| 1 | `project_archive_extension()` | no | `"evara"` — one source for the dialog filter |
| 2 | `project_export_start(path)` | the archive only | `ExportStarted { run_id, total_bytes }` |
| 3 | `project_export_status(run_id)` | no | `ExportStatus` |
| 4 | `project_export_cancel(run_id)` | no | `bool` — `false` if it had already finished |
| 5 | `project_export_discard(run_id)` | no | `()` |
| 6 | `project_import_inspect(path)` | **no** | `ArchiveSummaryView` — including `identityConflicts` (D13) |
| 7 | `project_import_start(archive_path, destination_path, on_conflict)` | the new project only | `ImportStarted { run_id, total_bytes }` |
| 8 | `project_import_status(run_id)` | no | `ImportStatus` |
| 9 | `project_import_cancel(run_id)` | no | `bool` |
| 10 | `project_import_discard(run_id)` | no | `()` |
| 11 | `project_recovery_report()` | no | `Vec<RecoveredView>` — what startup recovery did (D11) |

Shapes match the existing ones exactly: `ExportStarted { run_id, total_bytes }` mirrors
`GenerateStarted { run_id, total }` and `RepairStarted { run_id, movable, frozen }`;
`ExportStatus { run_id, phase, done, total, cancel_requested }` mirrors `GenerateStatus` and
`RepairStatus`. **No `epoch` on a start or a status** — it lives in the session and crosses
only on events, as `RepairProgressEvent` does.

Events follow the `solver://` and `repair://` convention:
`transfer://progress` → `{ run_id, epoch, phase, done, total }`;
`transfer://finished` → `{ run_id, epoch, ok }`. Coalesced at 50 ms with a guaranteed final
event.

Errors extend **`ProjectError`** in `commands/project.rs` — transfer *is* project lifecycle,
and that enum already carries `SchemaTooNew`, `NotFound`, `WrongExtension`, `AlreadyExists`
and `Failed`. `SetupError` is untouched. No third enum. Every DTO is generated into TypeScript
by `evara-typegen` under the `ts` feature (ADR 0012), so no hand-written mirror exists.

`on_conflict` crosses IPC as `"refuse"` / `"replace"` — one vocabulary, in the Rust enum, the
command, the interface and the tests.

**Shared, not copied.** `CoalescingSink` hard-codes `Progress` and
`evara_solver::ProgressSink`. It is generalised over its payload so the 50 ms interval
arithmetic has one implementation and two callers — the duplication `CLAUDE.md` §2 is about.

### 4.4 CLI

```
evara export --project <path> --to <archive.evara>              # writes the archive only
evara import <archive.evara> --to <project.evaraproj> [--replace]
evara inspect <archive.evara>                                   # manifest + readability, writes nothing
```

Exit codes follow the established convention: `0` done, `1` refused for a reason the command
correctly produced (unsupported capability, schema too new, destination exists), `2` the
command itself failed. `export` uses `open_for_inspection` plus the backup-API mechanism
(E-02), because `VACUUM INTO` is refused there (E-01).

---

## 5. Migration and compatibility strategy

| Situation | Behaviour |
| --- | --- |
| `format` is not `"evara-project"` | Refuse, naming the file |
| `formatVersion` > supported | Refuse, naming the Evara version needed — before any staging |
| `formatVersion` < supported | Run the container migration chain `N → … → current`; note it in the summary |
| `formatVersion` = supported | Read directly |
| `payload.schemaVersion` > `supported_schema_version()` | Refuse — `SchemaTooNew` with both numbers, already implemented and already a tagged frontend error. Step 7 of §3.3, before any payload byte is written |
| `payload.schemaVersion` < current | Migrate the **staged copy** forward; the destination is created only on success |
| Manifest and payload disagree about `schemaVersion` | Refuse — the archive was not assembled by Evara |
| Unknown `capabilities` entry | Refuse, naming the capability — step 8, before any payload write |
| Unknown manifest field | Ignore, record in the summary |
| Unknown archive entry, safe name | Ignore, record in the summary |
| Unknown archive entry, unsafe name | Refuse the archive |
| `counts` disagrees with the payload | Report; do not refuse — the payload is the authority |
| Checksum mismatch, wrong length, truncation | Refuse; nothing staged, nothing created |

**Bumps `formatVersion`:** renaming or removing an entry; changing the payload kind; changing
the meaning of a manifest field; changing the digest algorithm. **Does not:** adding an
optional manifest field; adding a `capabilities` prefix; adding an entry (unknown safe entries
are ignored); anything inside the payload, which is `schemaVersion`'s axis.

**The compatibility claim and its test.** "A v1 export opens in a v5 app" is tested by T-41's
generated matrix (§1.4.6). One honest limit: a project generated *today* at V11 by `run_up_to`
is not bit-for-bit what a 2026 build actually wrote at V11 — the migration SQL is identical,
but no data a later migration touches is reconstructed. It is strictly better than the nothing
that exists now, and it is not the same as having kept the old binaries.

---

## 6. Security and integrity guarantees

### 6.1 The three-phase boundary

Every control below is proven available — E-09 through E-14.

**Phase V0 — hardened, read-only, before any write.** A fourth constructor beside
`open_configured` / `open_plain` / `open_query_only`:

| Control | Value | Why |
| --- | --- | --- |
| open flags | `SQLITE_OPEN_READ_ONLY` only | **Corrected in revision 2.3 — F-2.** The earlier row claimed `SQLITE_OPEN_URI` *off* as a control. It is not one here: `-DSQLITE_USE_URI` (E-14) makes URI interpretation the global default, so clearing the flag changes nothing (E-33). The flag is not requested, as a statement of intent; the control is the row below |
| the **path form** | `require_plain_database_path` before the open | F-2's actual control, beside D6's "no archive byte reaches any path". Refuses a `file:` prefix, a `?`, a `#`, a `%` escape, an interior NUL and a `..` component, as `DbError::DestinationPathUnsafe`. Asked **before** the existence check, so `file:x?mode=rwc` is reported as the path form it is rather than as a file that happens not to exist |
| is it a database at all | the sixteen-byte header, read before anything is opened | **New in revision 2.3 — E-34.** A read-only open of arbitrary bytes *succeeds* (SQLite does not read the header until the first statement), and opening a non-database **mutates**: see [D6](#d6--nothing-is-ever-extracted-to-a-path-taken-from-the-archive) item 7. `UntrustedPayload::open` also probes `sqlite_master` once, so "this is not a database" is `NotAnEvaraProject` with a sentence rather than an opaque internal error from whichever query ran first |
| `SQLITE_DBCONFIG_DEFENSIVE` | on | blocks `PRAGMA writable_schema` (E-07 proves it is otherwise permitted) and writes to virtual-table shadow tables |
| `SQLITE_DBCONFIG_WRITABLE_SCHEMA` | off | belt and braces |
| `SQLITE_DBCONFIG_TRUSTED_SCHEMA` | **off** | safe here: nothing is written and no tracked table is queried, so no change-log trigger can fire. §C.5 |
| `SQLITE_DBCONFIG_ENABLE_VIEW` | off | free — the schema has no views (verified across V1…V18) |
| `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE` / `_WRITE` | off | 3.49+; the bundled engine is 3.50.2 |
| `SQLITE_LIMIT_ATTACHED` | **0** | forbids `ATTACH` outright, independent of engine version |
| `SQLITE_LIMIT_SQL_LENGTH` / `EXPR_DEPTH` / `VDBE_OP` / `LIKE_PATTERN_LENGTH` / `TRIGGER_DEPTH` / `COMPOUND_SELECT` | 1 MiB / 100 / 250 000 / 1 000 / 1 / 10 | bounds resource exhaustion from a crafted schema object |
| `PRAGMA query_only` | on, **read back and verified** | the existing pattern |
| `PRAGMA cell_size_check` | on | catches a class of corrupt page early |
| `PRAGMA mmap_size` | 0 | never mmap an untrusted file — a corrupt page raises an error instead of a `SIGBUS` |
| `journal_mode` | **not set** | `open_plain`'s WAL pragma writes to the file header; the hardened path must skip `configure` entirely |
| `evara_site_id()` | **not registered** | an unattributable write is impossible even if the pragma were lifted |

**V0 does not write the payload, and is not directory-inert** — added in revision 2.2, because
the paragraph above says "nothing is written" and that is true of the *file* and false of its
*folder*. Opening a WAL-mode database with `SQLITE_OPEN_READ_ONLY` makes SQLite **create a
`-wal` and a `-shm` beside it**, because read-only WAL access still needs the shared-memory
file to locate the log. The payload's own bytes are byte-identical before and after an audit —
that is the guarantee, and **E-29** is the test that asserts it — but the directory is not
untouched. Handled rather than ignored: the payload sits in staging, and S3's
`checkpoint_to_single_file` reduces it to one file before any rename, which is asserted
separately. `?immutable=1` would suppress the sidecars and was **rejected**: it tells SQLite
the file cannot change and makes concurrent modification undefined behaviour, which is not a
property to assume about a file from somewhere else.

Then, in this order, **touching no domain table**:

1. `PRAGMA integrity_check`. A **corruption** check only — E-05 proves it returns `ok` on a
   database carrying a hostile trigger. **Not cancellable today** — revision 2.2: E-32 rules
   `progress_handler` out for a borrowed `&dyn Cancel`, and `hooks` is no longer enabled, so
   the statement runs to completion. It is bounded by the staged payload's declared byte
   length, which the container caps, and making the audit interruptible is a **Stage 2**
   question to answer with a figure rather than a guess (§7.4).
2. **`sqlite_master` conformance** against `expected(declared schemaVersion)`, built in a temp
   file by `run_up_to` (D10). This is what catches the hostile trigger, the extra table, the
   virtual table and the shadow table that step 1 misses.
3. `app_meta` identity read — a domain table, and safe only *after* conformance. Revision 1
   had this before conformance, which was wrong.
4. `reject_if_newer_than_supported`.

**Phase V1 — writable staging.** `open_configured` on the staged copy with the site id.
`DEFENSIVE` stays on. `TRUSTED_SCHEMA` is left at the build default — on (E-15) — because
conformance has already proven the schema is exactly ours, so there is no crafted function
call left to guard against, and turning it off would break every change-log trigger
(§C.5, and A2 if you want it off here too). Then `migrations::run` and
`change_log::install_all`.

**Phase V2 — after migrating.** `PRAGMA foreign_key_check` must be empty (E-06 proves it
catches what `integrity_check` does not); conformance against `expected(supported)`;
`table_census`; the capability check against `ConstraintType::from_key`.

### 6.2 What is **not** guaranteed

- **Checksums detect corruption, not authenticity.** §3.3. There is no signature, because a
  signature needs key distribution and ADR 0001 forbids the service that would provide it.
- **`sqlite_master` conformance is not a sandbox.** It proves the schema matches ours. It does
  **not** bound page-content risk. What bounds that is defence in depth: `DEFENSIVE`,
  `cell_size_check`, `mmap_size = 0`, the seven limits, `query_only` for all of V0, a
  **bundled** engine (one known SQLite on every install, never the host's), extension loading
  off three ways over, and the fact that the staged file is a throwaway deleted on any
  failure. **A bug in SQLite's own b-tree parser is not mitigated by any of this**, and this
  proposal does not claim otherwise.
- **Crash-safe durability of replacement is not claimed.** D11's *Guarantees* section states
  exactly which filesystem property is relied on and which is not, and the recovery procedure
  for every reachable state.
- **A concurrent holder of the destination cannot be detected on macOS.** D11 names the
  asymmetry; the kept `.replaced-` copy is the mitigation.
- **No malware scanning.** A `.evara` is data; Evara executes nothing from it.
- **A capability refusal is a refusal, not a repair.** Evara does not partially import such an
  archive and does not offer to drop the rule — dropping a constraint silently is how a
  timetable comes to be wrong.

### 6.3 Offline and licensing

No network code on any path. `evara-project` depends on `zip`, `sha2`, `serde_json`,
`evara-core` and `evara-db`; none reaches a socket. `cargo deny check` keeps the licence and
advisory policy enforced, and it is **not** part of `npm run check` — it must be run
separately. Nothing in the archive identifies a person: `site_id` is a random installation
UUID. Every fixture is generated synthetically, and T-61 asserts no binary project or archive
fixture is checked in.

---

## 7. Performance and resource budgets

Every figure is a **budget to be measured and reported**, in the style the M6 and M8 records
set. None of the figures below is a measurement.

> **Where the measurements now happen — revision 2.2.** Stage 1 is built and measured **none**
> of them. Every §7 figure needs a project of realistic size with real edit history, and
> Stage 1 builds no such fixture: its projects are a handful of rows created inside a test.
> **T-60, T-83 and the snapshot-inflation figure §7.2 now asks for are all Stage 2**, where the
> round-trip fixtures first build a project at §7.3's scale. What Stage 1 *did* establish is in
> [§C.2](#c2-evidence-log--what-is-proven-and-what-still-needs-an-experiment): behavioural
> facts proven by tests, not performance figures.

### 7.1 Memory

| Phase | Budget | Why it holds |
| --- | --- | --- |
| Export | **O(1) in project size**, ≤ 8 MiB over baseline | the snapshot streams inside SQLite; the ZIP writer takes a 1 MiB buffer; SHA-256 is incremental; the only whole thing in memory is the manifest (≤ 64 KiB) |
| Import | **O(1)**, ≤ 8 MiB | the payload streams through the same buffer; the JSON entries are capped at 64 KiB and 1 MiB |
| Inspect | ≤ 2 MiB | two small entries, nothing else read |

### 7.2 Disk

| Phase | Peak extra | Where |
| --- | --- | --- |
| Export | ≈ 1 × project size (snapshot) **+ its free pages** + archive size | staging beside the destination; `.part` beside the destination |
| Import, Create mode | ≈ 1 × payload | staging beside the destination |
| Import, Replace mode | ≈ 2 × payload (staged + aside) **and the kept copy is permanent** | staging, then `<stem>.replaced-<stamp>.evaraproj` |

**The snapshot is not compacted** — revision 2.2. This table was written assuming
`VACUUM INTO`, which defragments as it copies; the stepped backup §C.6 settled on reproduces
the source page for page, free pages included. So "≈ 1 × project size" is now *the project
file's size on disk*, not its live-data size, and a heavily edited project — one that has had
timetables deleted, say — carries its free list into the snapshot. The archive absorbs most of
it: free pages are mostly zeroes and deflate well, so the figure with a user-visible
consequence (`.evara` size) moves far less than the staging figure does.

**No compaction pass is added on that account, deliberately.** Snapshotting and then
`VACUUM INTO`-ing our own snapshot is cheap to write, needs no cancellation — nobody else holds
that file — and costs a second full write of it. Adding it now would be paying for a problem
with no measurement. **Stage 2 measures the real inflation** (alongside T-60 and T-83, which
moved there for the same want of a realistic fixture), and that figure, not this paragraph,
decides whether the second pass is worth its cost.

**No pre-flight free-space check** (§C.4): there is no free-space API in the tree and
`unsafe_code` is forbidden, so it would need a dependency to do what D11's structure already
makes safe. `ENOSPC` becomes `InsufficientSpace` with the figures named.

The permanent kept copy in Replace mode is a deliberate cost: disk grows by one project per
replacement, and nothing in Evara deletes it. A6-adjacent, and worth confirming (A3).

### 7.3 Time

Two reference schools. **Realistic upper end** is M6/M8's 2 000-occurrence scale (~125
sections, ~300 timeslots, three saved timetables). **Ten times** is a synthetic
20 000-occurrence multi-school project built for the scale test.

| Measure | Realistic upper end | Ten times |
| --- | --- | --- |
| Project file size | 5–15 MiB | 60–150 MiB |
| Snapshot size over the project file (free pages, §7.2) | to be measured — Stage 2 | — |
| Archive size | to be measured — SQLite deflates well | — |
| **Workspace lock held** (the figure with a user-visible consequence) | **≤ 500 ms** | ≤ 3 s |
| Export, end to end | ≤ 1.5 s | ≤ 15 s |
| `inspect` | ≤ 50 ms | ≤ 50 ms — it reads two small entries |
| Import, end to end (stage + verify + migrate + verify + rename) | ≤ 3 s | ≤ 30 s |
| Peak RSS over baseline | ≤ 8 MiB | ≤ 8 MiB |

For the lock's duration every other command blocks. If T-83 comes in above 500 ms, the remedy
is a second `SQLITE_OPEN_READ_ONLY` connection (E-03), not a longer lock.

### 7.4 Risks on the budget

- ⚠ **`change_log` size** (D4) — the one component that grows with *use* rather than with
  school size. T-60 reports its share for a project with real edit history, and that
  measurement decides A6. **Stage 2** since revision 2.2.
- ⚠ **The snapshot's free pages** (§C.6, §7.2) — new in revision 2.2, and the reason §7.2's
  export row is no longer "≈ 1 × project size" of live data.
- ⚠ **Replace mode's permanent kept copy** — §7.2.
- ⚠ **Windows `MAX_PATH`** with a long destination plus a 45-character staging directory.
- ⚠ **Antivirus holding a newly written file open** — D11's bounded rename retry, and T-70
  needs a manual pass as well as CI.
- ⚠ **`integrity_check` on a very large payload** is unbounded in time, and since revision
  2.2 it is **not interruptible either** — E-32 and the `hooks` removal. The payload's declared
  byte length is the only bound; Stage 2 measures the audit on the ten-times project and
  decides whether an interrupt mechanism is worth a `Send + 'static` cancellation path.
- New feature flags on an existing dependency, no new crate: `rusqlite`'s `backup` (E-13) and
  `limits` (E-11). **`hooks` is not enabled** — revision 2.2. It is `progress_handler`'s
  feature, and E-32 is why that API is not on the snapshot path; a feature nothing calls is
  compiled surface nobody tests, so Stage 1 removed it and recorded in `Cargo.toml` why it is
  absent.

---

## 8. Implementation stages and dependencies

```
Stage P  zip >= 9 upgrade   ✅ DONE — PR #35, 0faff47, 2026-11-01
   │
Stage 1  evara-db::transfer  +  E-01…E-08 re-proven                        ◄── DONE
   │        ├── settings migration V2 (replacement journal)
   │        └── S1 / S2 / S3 primitives the Stage 2 protocol is built from
   │
Stage 2  evara-project container  ◄──── Stage P (satisfied)                ◄── NEXT
   │        ├── the fixtures feature, with its only consumer
   │        ├── §7's measurements: T-60, T-83, snapshot inflation
   │        └── D11 as amended by §0.2 S1 + S2 + S3 — never D11's superseded text
   │
   ├── Stage 3  CLI + the generated historical-version matrix
   └── Stage 4  Tauri commands, session, progress, typegen
                    │
                Stage 5  the interface
                    │
                Stage 6  documents and ADRs   (runs alongside from Stage 1)
```

| Stage | Scope | Depends on | Done when |
| --- | --- | --- | --- |
| **P** | `calamine 0.36.1 → 0.37.0`, `rust_xlsxwriter 0.99.1 → 0.102.0`, converging `zip` on `9.1.3` and removing `8.6.0` from the tree. **Its own PR.** | — | ✅ **Done — PR #35, merged 2026-11-01 as `0faff47`.** Five gates green (`cargo deny check` all four sections, `rust:fmt`, `rust:lint`, `cargo test -p evara-import` 38, `cargo test --workspace --all-features` 1 709/0), CI on `windows-latest` + `macos-latest`. `typed-path` and `nix` licences **confirmed** (E-23). All three advisories resolved (E-24). The sixth gate, full `npm run check`, **did not pass** — two pre-existing jsdom timeouts unrelated to the change, reported not suppressed (§C.8). |
| **1** | `evara-db::transfer`: the hardened V0 constructor, `snapshot_into` returning an owned `Snapshot` that carries its own census, conformance against a generated reference, the new `DbError` variants. The replacement journal and `settings-migrations/V2`. Fix the `backup.rs` 0-byte leftover (E-22). `evara_site_id` marked `SQLITE_INNOCUOUS` (A2). **Re-prove E-01…E-08 against the bundled engine** (T-75…T-82). **Build the S1/S2/S3 primitives** the Stage 2 protocol composes — safe destination claiming, filesystem-truth reconciliation, single-file-per-side sidecar handling — with T-84…T-90. | P (satisfied) | ✅ **Done — PR #37.** `cargo deny check` four sections ok, `rust:fmt` and `rust:lint` clean, `cargo test --workspace --all-features` **1 794 / 0**, `npm run check` green, CI green on `windows-latest` and `macos-latest`. T-75…T-82 and T-84…T-90 green; E-27…E-32 added; **E-04 corrected**. **One deviation**: one snapshot mechanism, not two (§C.6, E-32). **Three items moved out**: the `fixtures` feature to Stage 2 with its only consumer, and T-60 and T-83 to Stage 2 for want of a realistic fixture (§C.2). **No ZIP, manifest, CLI command, Tauri command or screen**, and no project migration. |
| **2** | `evara-project`: manifest, `checksums.json`, the acyclic verification order, ZIP writer and reader, entry policy, byte caps, staging lifecycle, **D11's protocol as amended by [§0.2](#02-mandatory-safeguards-from-the-second-review) S1/S2/S3 — never its superseded text** — and recovery, `seal` / `inspect` / `import` / `recover_pending`, `TransferError`. Container migration chain, empty, at `formatVersion 1`. Takes the direct `zip 9.1.3` dependency. **Plus, moved here from Stage 1:** `evara-db`'s `fixtures` feature (§1.4.8), `transfer::stage`, the free-function `table_census(path)`, and the §7 measurements. | 1, P | ✅ **Done.** Round-trip, corruption, replacement and recovery tests green; `cargo test --workspace --all-features` **1 892 / 0**; `rust:fmt` and `rust:lint` clean. **T-60 = 26.2%, T-83 = 46-82 ms (50-130 ms under a writer), §7.2 = +6.9% over a vacuumed copy** — and the compaction question **answered: no pass added** (E-36…E-38, §C.10). One **security correction**: F-2, `SQLITE_OPEN_URI` was never a control here (E-33). Two **defects found by the composition** and fixed: observing the disk mutated it (E-34). One **fact recorded**: Deflate cannot express a bomb (E-35). Four departures, each in §C.10. `cargo deny check advisories` fails on a **pre-existing** advisory that fails on `develop` too — RUSTSEC-2026-0459, §C.10. |
| **3** | `evara export` / `import` / `inspect`; move them from `PLANNED` to `IMPLEMENTED`. T-41's generated matrix in CI on both platforms. | 2 | Matrix green for V1…V18 on Windows and macOS. **Partially met.** The three commands are built on §4.4's syntax and its three exit codes, as the first callers of Stage 2's API — no second writer, no second import path. 160 CLI tests, `rust:fmt` and `rust:lint` clean. **T-41 is not built**, so the stated exit criterion is *not* met: the matrix needs `migrations::run_up_to` reachable from outside `evara-db` (it is `pub(crate)`) and a per-version fill routine, because today's repository API cannot write to a V3 schema. Reported rather than dropped — §1.4.6's gap is still open. Two smaller notes: `seal` does not refuse an occupied archive, so the guard is the CLI's and **Stage 4 needs its own**; and a corrupt deflate stream arrives as `Io`, which the CLI cannot distinguish from a failing disk, so it exits 2 rather than 1. |
| **4** | Eleven Tauri commands, `TransferSessions` in `AppState` cleared on create/open/close, generalised `CoalescingSink`, `transfer://` events, extended `ProjectError`, `Workspace::remember_imported_project`, `Settings::projects_with_identity`, `RecentProject::duplicate_identity`, typegen. | 2 | `npm run check` green; command tests green. |
| **5** | Export and Import screens: native dialogs, inspect-then-confirm, the identity warnings of D13 verbatim, destination conflict with Replace and the keep-name named, progress with cancel, the synced-folder advisory, the recovery report, the recent-list entry with "Open now", and a keyboard path to all of it. | 4 | vitest green; the GUI smoke-test gap M9A left is not widened. |
| **6** | Rewrite [PROJECT-FORMAT.md](PROJECT-FORMAT.md). ADRs 0053–0059 (§11). Mark Phase 9 complete; close M1-17 / M1-18. Discharge R8; add D37, D38, R11. Amend TIMETABLE-PERSISTENCE §7's table and ADR 0042 §C4's. Correct `capabilities/default.json`'s description. | from 1 | No document still describes the rejected design. |

**M10 adds no project migration.** V18 is the schema, and transfer reads and copies it. It
**does** add one settings migration (`V2__pending_replacement.sql`) for D11's journal — E-21
proves the settings database has its own chain, so this is not a change to any `.evaraproj`.
That distinction was wrong in revision 1 and is corrected here (§C.9 item 10). If a stage
finds itself wanting a project column, that is a signal the design went wrong, not a reason to
write V19.

---

## 9. Test matrix

Every fixture is synthetic. No real school, teacher, parent or student name, in the repository
or in any CI artefact.

### Round trips and semantic equality

| # | Test | Where |
| --- | --- | --- |
| T-01 | Export → import → `table_census` equal for **every** table discovered from `sqlite_master`. A future table is covered by discovery, not by memory | `evara-project` |
| T-02 | Row-by-row equality for all 38 tracked tables, including `id`, `rev`, `created_at`, `updated_at` | `evara-project` |
| T-03 | `project_id` and `created_by_site_id` identical after transfer; `exportedBySiteId` is the exporting installation and does **not** become `created_by_site_id` | `evara-project` |
| T-04 | Export → import → export: payload bytes identical, only `manifest.exportedAt` / `exportedBySiteId` differ | `evara-project` |
| T-05 | Two exports of an unchanged project differ only in those two fields — fixed ZIP timestamps, LF-only JSON | `evara-project` |
| T-06 | The manifest's `integrity.sha256` matches `checksums.json`; `checksums.json`'s entry matches the payload; `manifest` is **not** listed in `checksums.json` (the acyclicity assertion) | `evara-project` |
| T-07 | All twelve `evara-db` fixtures round-trip, via the new `fixtures` feature | `evara-project` |

### Complete data preservation

| # | Test | Where |
| --- | --- | --- |
| T-08 | Multi-school: a building, room and resource **shared across two schools** keep both `*_campus` rows, and each remains destroyable only by its home school after import | `evara-project` |
| T-09 | `teacher_qualification` survives with no `school_id` and outlives a withdrawn `teacher_school`; `teacher_department` survives and is still held to membership by its trigger pair | `evara-project` |
| T-10 | `timeslot` ids identical, and re-materialising the grid on the imported project preserves them (ADR 0011) | `evara-db` |
| T-11 | `activity.split_index` not renumbered; occurrence counts per section match | `evara-project` |
| T-12 | `school_logo` bytes, `media_type`, `width`, `height` byte-identical | `evara-project` |
| T-13 | `constraint_instance` / `constraint_ref` survive, and `stored_for` on the imported project compiles to the same `StoredConstraint` set | `evara-db` |
| T-14 | `availability` survives for every entity kind, and the polymorphic reference still resolves | `evara-project` |
| T-15 | `timetable_entry` placements, `is_locked`, `is_manual` preserved exactly, for several versions of one `(cycle, term)` | `evara-project` |
| T-16 | `solve_run` keeps `seed`, `engine`, `kind`, `scope`, all six counters, both timestamps, `inputs_watermark` | `evara-project` |
| T-17 | `solve_run_unscheduled` survives with `cause` and `diagnostic_code`; `is_impossible()` answers the same | `evara-project` |
| T-18 | Lineage `A → B → C` keeps both edges, and `Timetables::delete(A)` on the imported project still refuses with `HasDescendants` naming the count | `evara-db` |
| T-19 | **`GENERATION` findings survive**: the generation evaluation has the same `violation` / `violation_ref` rows, and §1.4.1's regression — a generation score with an empty findings list — is asserted against | `evara-project` |
| T-20 | The two partial unique indexes still hold; `timetable_revalidate` on the imported project replaces only the `REVALIDATION` row | `evara-db` |
| T-21 | **Change-feed continuity**: `change_log` rows identical, `MAX(seq)` matches the manifest watermark, and the next write gets `MAX(seq) + 1` — so no stored `inputs_watermark` is in the future | `evara-db` |
| T-22 | An editing write on the imported project produces a correct `ArrangementStamp` guard and refuses a stale one (ADR 0047) | `evara-db` |

### Corruption, hostility and refusal

| # | Test | Where |
| --- | --- | --- |
| T-23 | Not a ZIP; no `manifest.json`; manifest not JSON; manifest missing `formatVersion` — four distinct named errors, nothing created | `evara-project` |
| T-24 | Payload truncated mid-stream → refused; staging removed; destination absent | `evara-project` |
| T-25 | One payload byte flipped → `ChecksumMismatch`; nothing staged past the hash | `evara-project` |
| T-26 | Checksum right, `byteLength` wrong → `PayloadLengthMismatch` | `evara-project` |
| T-27 | Payload is valid SQLite but not an Evara project → `NotAnEvaraProject` | `evara-project` |
| T-28 | Payload with a deliberately broken foreign key → `ReferentialDamage` with the count (the E-06 case, as a product test) | `evara-db` |
| T-29 | Payload with an **extra trigger**, an **extra table**, a **missing index**, and a **divergent trigger body** → four `SchemaUnexpected` refusals, each naming the object, and **no write reached the staged file** | `evara-db` |
| T-30 | Payload whose `integrity_check` fails → `PayloadCorrupt` | `evara-db` |
| T-31 | Entry named `../../evil`, `/etc/passwd`, `C:\evil`, `a\b`, and a 4 KiB name → each refuses the archive, and **no file is created outside staging**, asserted by listing the parent before and after | `evara-project` |
| T-32a | A 1 GiB-of-zeroes payload declared as 1 KiB → refused at the bound, with bounded memory and bounded disk | `evara-project` |
| T-32b | An archive with 10 000 entries → `TooManyEntries` | `evara-project` |
| T-33 | An entry compressed with anything but Stored/Deflate → `UnsupportedCompression` | `evara-project` |
| T-34 | A symlink entry and a directory entry → refused | `evara-project` |
| T-35 | `manifest.json` ≥ 64 KiB → refused **without being parsed** | `evara-project` |
| T-36 | An unknown extra entry and an unknown manifest field → **accepted**, both listed in the report. Forward tolerance tested, not assumed | `evara-project` |
| T-68 | `checksums.json` with `"algorithm": "md5"` → `UnsupportedDigest`, not a fallback | `evara-project` |
| T-69 | `checksums.json` tampered so its own digest no longer matches the manifest → `ChecksumMismatch` before the payload is touched | `evara-project` |
| T-71 | Payload with a **virtual table** (`USING fts5` — compiled in per E-14) and with a **shadow table** → `SchemaUnexpected` each | `evara-db` |
| T-72 | On a hardened V0 connection: `SELECT load_extension('x')` fails, `ATTACH` fails, `PRAGMA writable_schema = ON` fails, `PRAGMA query_only` reads `1` — four assertions, not assumptions | `evara-db` |
| T-73 | A payload with a 50 000-deep expression in a CHECK, and one with a trigger recursing on itself → refused by conformance, and the limits hold if conformance is bypassed in the test | `evara-db` |
| T-74 | Conformance runs **before** the `app_meta` read and before any write — asserted by a payload whose `app_meta` is poisoned *and* whose schema diverges, where the schema error must be the one reported | `evara-db` |

### Snapshot mechanism — the Stage 1 proof tests

**As implemented in Stage 1.** The numbering below is the one in the code, and it is
authoritative: revision 2's draft assigned T-78…T-82 to different claims, and the mapping was
reconciled in revision 2.2 rather than leaving the record pointing at identifiers that mean
something else in the tree. Every claim the draft listed is covered; two of the eight are
covered by a named test rather than a `T-` number, which is noted.

| # | Test | Claim | Where |
| --- | --- | --- | --- |
| T-75 | `t75_vacuum_into_is_refused_under_query_only_and_leaves_a_zero_byte_target` | E-01, both halves | `evara-db` ✅ |
| T-76 | `t76_the_backup_api_works_from_a_query_only_connection` | E-02 | `evara-db` ✅ |
| T-77 | `t77_a_stepped_backup_in_a_read_transaction_does_not_see_a_concurrent_write` | **E-27** — the pinned read snapshot, the fact the design rests on | `evara-db` ✅ |
| T-78 | `t78_vacuum_into_refuses_a_non_empty_target_but_adopts_a_zero_length_one` | **E-28**, which corrects E-04 | `evara-db` ✅ |
| T-79 | `t79_integrity_check_is_ok_on_a_database_carrying_a_hostile_trigger` | E-05 — the test that justifies conformance existing | `evara-db` ✅ |
| T-80 | `t80_foreign_key_check_reports_a_violation_integrity_check_calls_ok` | E-06 | `evara-db` ✅ |
| T-81 | `t81_writable_schema_succeeds_by_default_and_defensive_blocks_it` | E-07 | `evara-db` ✅ |
| T-82 | `t82_a_vacuum_into_to_a_missing_directory_leaves_nothing_behind` | E-08 — so E-01's leftover is specific to the refused write | `evara-db` ✅ |
| — | `a_read_only_open_flag_is_not_the_same_as_the_query_only_pragma` | E-03, the draft's T-78 | `evara-db` ✅ |
| — | `send_assertions::a_snapshot_can_be_moved_to_a_worker` | `Snapshot` is `Send`, the draft's T-82 | `evara-db` ✅ |
| T-83 | `t83_the_lock_duration_of_a_stepped_backup`, `t83_a_concurrent_writer_changes_the_duration_and_not_the_copy` | **E-37 — measured: 46 ms, 1 033 pages, 5 steps, ~9 ms/step; 50 ms under a writer** | `evara-project` ✅ |
| T-91 | `t91_a_uri_filename_is_interpreted_without_the_open_uri_flag` | **E-33** — F-2: the flag is not a control in this build | `evara-db` ✅ |

### What Stage 2's suites cover, by matrix identifier

Added in revision 2.3. Every row below is green; the identifiers are the matrix's, and where a
row was built differently from its description the difference is in
[§C.10](#c10-stage-2-what-was-built-measured-and-corrected)'s departures table rather than
quietly absorbed.

| Rows | Where | Note |
| --- | --- | --- |
| T-01, T-03, T-21, T-43, T-44, T-64 | `evara-project::tests_round_trip` | census compared over every table **discovered** from `sqlite_master`, so a future table is covered by discovery |
| T-04, T-05, T-06 | same | T-04 asserted as identical census and identical manifest but for the two fields that may differ — §C.10 departure 4 |
| T-08…T-19 | same, as one sweep | `every_table_the_record_names_as_travelling_arrives_populated` names each table, so a failure says *which* came through empty; the census equality is what proves the counts |
| T-23…T-27, T-31…T-39, T-42, T-45, T-47, T-48, T-50, T-68, T-69 | `evara-project::tests_refusal` | each asserts the refusal **and** that the destination's directory listing is unchanged |
| T-32a | same, as two tests | a payload producing more than it declared stops at the bound; one declared beyond the absolute cap is refused unread. The *ratio* half is E-35 — Deflate cannot reach the cap |
| T-49, T-84…T-90 | `evara-project::tests_replacement` | the second payment of the assertions Stage 1 made against the primitives, and the one that found E-34's two defects |
| T-56, T-61, T-62 | `evara-project::tests_hygiene` | T-62 asserted by `rusqlite`'s **absence** from the manifest, and by every `zip`-reachable refusal being matchable as plain data |
| T-60, T-83, §7.2, §7.3 | `evara-project::tests_measurements` | figures reported with `--nocapture`; only the order-of-magnitude bound is asserted, because the value depends on the disk |

**Owed to later stages, and named rather than assumed:** T-02 (row-by-row equality over all 38
tracked tables — the census plus the tracked-table sweep is what stands in today), T-07 (all
twelve catalogue fixtures round-tripped; the catalogue is in place behind the `fixtures`
feature and one project is round-tripped), T-10, T-20, T-22, T-28…T-30, T-71…T-74 (the
`evara-db` half of the hostile-payload rows, which need a crafted SQLite file and so a driver
this crate deliberately does not have), T-41 (the generated historical matrix — **Stage 3**),
T-51…T-55 and T-63…T-67 and T-70 (**Stage 4**), T-57…T-59 (**CI and the ten-times fixture**).

### Versions and compatibility

| # | Test | Where |
| --- | --- | --- |
| T-37 | `formatVersion: 2` → `FormatTooNew` naming the Evara version needed; nothing staged | `evara-project` |
| T-38 | `format: "something-else"` → `UnknownFormat` | `evara-project` |
| T-39 | `payload.schemaVersion` above `supported_schema_version()` → `SchemaTooNew` with both numbers, **before any payload byte is written**, and the tagged frontend error still distinguishes it | `evara-project`, `src-tauri` |
| T-40 | Manifest says V17, payload history says V18 → `ManifestDisagreesWithPayload` | `evara-project` |
| T-41 | **The generated matrix**: for each `n` in V1…V18, `run_up_to(n)` → fill → export → import → assert identity, census and conformance at the current version, with the migration recorded. Extends itself when V19 lands | `evara-project`, CI |
| T-42 | A project configuring an unimplemented `type_key` → refused with the capability named, **before** staging; and the same archive imports in a build that implements it | `evara-project` |
| T-43 | `CUSTOM` and `WEEKLY` cycle kinds produce the right `cycle-kind:` capability and import cleanly | `evara-project` |

### Workflow, replacement, interruption and identity

| # | Test | Where |
| --- | --- | --- |
| T-44 | Export writes nothing to the source: change-log watermark, every row's `rev`, and the file mtime unchanged | `evara-db` |
| T-45 | Export cancelled mid-compress: no `.evara`, no `.part`, no snapshot, no staging | `evara-project` |
| T-46 | Export cancelled mid-snapshot: the same, and the source untouched | `evara-db` |
| T-47 | Export over an existing `.evara` that fails mid-write: the previous archive is still intact and readable | `evara-project` |
| T-48 | Import cancelled at each phase (staging, verifying, migrating): no destination, no staging | `evara-project` |
| T-49 | `OnConflict::Refuse` on an occupied destination → refused by name. `OnConflict::Replace` → succeeds, **and the old project is kept at `<stem>.replaced-<stamp>.evaraproj`** | `evara-project` |
| T-50 | Destination without `.evaraproj` → `WrongExtension`; `.evara` as a destination refused | `evara-project` |
| T-51 | Import does **not** open the imported project, does **not** disturb the open one, and **does** add a recent-list entry | `src-tauri` |
| T-52 | A session's `epoch`: export started, project closed, status and finalise both refuse | `src-tauri` |
| T-53 | Two concurrent finalises: one wins, the other is refused; exactly one archive and one project exist | `src-tauri` |
| T-54 | Progress events coalesced ≥ 50 ms apart with a guaranteed final one — asserted against the **generalised** sink, shared with the solver's | `src-tauri` |
| T-55 | `project_import_inspect` writes nothing — asserted by comparing the filesystem and the archive digest before and after | `src-tauri` |
| T-63 | Importing a project whose `project_id` matches a recent entry → `identityConflicts` names it, with the path, and the import still proceeds when confirmed | `evara-project` |
| T-64 | `project_id` is **never** regenerated, including when a twin is present | `evara-project` |
| T-65 | `RecentProject::duplicate_identity` is true for both entries once two copies exist, and false otherwise | `evara-db` |
| T-66 | Replacing the destination whose project is **currently open** → `DestinationIsOpen`, and the file is untouched | `src-tauri` |
| T-67 | Each of D13's three sentences is produced verbatim for its case | `src-tauri`, vitest |
| T-70 | **Crash recovery**, one test per phase: kill between each journal commit and the rename that follows, then run `recover_pending` and assert the state is exactly `{old intact}` or `{new in place, old kept}` — never a partial destination. Plus a bounded-retry test with the destination held open | `evara-project` |
| T-84 | **§0.2 S1**: a destination that does not exist is claimed by `create_new(true)`; a **zero-length** destination with no journal row is **occupied**, is refused without `Replace`, and is kept as a `.replaced-` copy when Replace is used — never adopted and never overwritten | `evara-db` ✅ · `evara-project` |
| T-85 | **§0.2 S1**: the TOCTOU window is closed — a second claim of the same destination between the first claim and its rename fails, and the first import's destination is unharmed | `evara-db` ✅ · `evara-project` |
| T-86 | **§0.2 S2**: journal `PLACED` with the destination **absent** (crash between the commit and the rename) → recovery restores the old project to the destination and **does not** move it to the keep-name. This is the data-loss case revision 2's table got wrong | `evara-db` ✅ · `evara-project` |
| T-87 | **§0.2 S2**: every row of the reconciliation table, driven by fabricated on-disk states rather than by the journal, including the two **ambiguous** combinations, which must report and change nothing | `evara-db` ✅ · `evara-project` |
| T-88 | **§0.2 S2**: the dominating invariant, asserted directly — no code path releases the aside set unless a destination satisfying `looks_like_a_project` has been observed | `evara-db` ✅ · `evara-project` |
| T-89 | **§0.2 S3**: both sides are a **single file** before they move — the destination is checkpointed and its `-wal`/`-shm` are gone before the aside rename, and a sidecar that cannot be removed refuses the replacement **before anything has moved** | `evara-db` ✅ · `evara-project` |
| T-90 | **§0.2 S3**: interrupt at each individual sidecar step and assert that **no reachable state pairs a main file with a sidecar belonging to a different database** — including the pre-rename assertion that no `-wal`, `-shm` or `-journal` exists at the destination | `evara-db` ✅ · `evara-project` |

**T-84…T-90 are owed twice, and the first half is paid** — revision 2.2. Revision 2 listed them
against `evara-project`, but §8 put the S1/S2/S3 *primitives* in Stage 1, which is where the
decisions they assert actually live. Stage 1 therefore asserts each one against the primitive:
✅ in `evara-db`, including **T-87 over every row of the table under all three phases**, so the
facts decide and the journal does not; **T-88 over all 128 reachable combinations** — four
destination states × staged × aside × keep-name × four phases — rather than only the twelve
rows written down; **T-86** as the data-loss case directly, and **T-90** against a
child process that is genuinely killed (`std::process::abort`) at each journal-commit-to-rename
window, whose real disk state is then fed to the same `reconcile`. The `evara-project` half —
the same assertions against the **composed protocol**, with the `.replaced-` keep-name and the
bounded rename retry — is Stage 2's, and Stage 2 is not done until they are green there too.

**What T-88's sweep deliberately does not reach.** Two of `DiskFacts`' fields are held at "no
sidecars" across all 128 combinations, by decision rather than omission: `DestinationState::Project`
*means* "no sidecar beside it" — `DiskFacts::observe` applies that conjunction once — so a
fabricated project-with-a-`-wal` is a state the observer cannot produce, and sweeping over it
would assert about nothing. The sidecar-bearing destinations are T-89's. The one combination the
sweep **does** reach without the invariant firing — an **absent** main file with a stray
sidecar — is pinned separately by
`an_absent_destination_with_a_stray_sidecar_still_blocks_the_rename`, which shows that what
blocks the rename there is `refuse_if_sidecars_present` (S3 step 4) and not the reconciler.

### Portability, scale and hygiene

| # | Test | Where |
| --- | --- | --- |
| T-56 | Every entry name ASCII, lowercase, `/`-separated, relative; every timestamp the fixed constant; both JSON entries LF-only | `evara-project` |
| T-57 | The full suite runs on `windows-latest` and `macos-latest` | CI |
| T-58 | An archive produced on Windows imports on macOS and vice versa, identical census — by passing the archive as a CI artefact between jobs | CI |
| T-59 | The ten-times project exports and imports inside §7's budgets, with peak RSS asserted | `evara-project` |
| T-60 | The change log's share of the payload, reported for a project with realistic edit history — the D4/A6 measurement. **Stage 2** since revision 2.2: Stage 1 builds no project with real edit history, and a figure measured against a handful of test rows would not have tested the budget | `evara-project` |
| T-61 | No checked-in `.evara` or `.evaraproj` fixture exists; the matrix generates every one | CI |
| T-62 | `evara-project`'s public API exposes no `rusqlite` and no `zip` type — asserted by not having the dev-dependency, as `evara-db` does | `evara-project` |

---

## 10. Remaining decisions requiring approval

Seven as originally tabled. **All seven were answered at the second review**, 2026-10-29:
**A1–A5 approved** and scheduled into Stage 1 or Stage 2; **A6 and A7 deferred** out of M10
(see [§0.1](#01-approval-status-of-a1a7), which is the authoritative status). The table below
is kept as the record of what each decision was and why it was the user's to make, not as an
open question list. **Nothing here blocks Stage 1**, and Stage P is complete.

The three **mandatory safeguards** attached to that approval are not in this table, because
they are conditions rather than choices: [§0.2](#02-mandatory-safeguards-from-the-second-review)
S1, S2 and S3, each superseding a named part of D11.

| # | Decision | Recommendation | Why it is yours |
| --- | --- | --- | --- |
| **A1** | Move the `Cancel` trait to `evara-core`, re-exported by `evara-solver`, so transfer and the solver share one | **Yes.** It is exactly ARCHITECTURE §3 rule 1's criterion — two crates must agree — and a ~10-line change | It touches `evara-solver`, outside M10's stated scope |
| **A2** | Also mark `evara_site_id` `SQLITE_INNOCUOUS` (E-09) and keep `trusted_schema = OFF` through phase V1 | **Yes**, as extra hardening. V0 is already hardened without it, so this is additive, not required | It touches `connection.rs`, the factory **every** project open uses |
| **A3** | The replaced copy (`<stem>.replaced-<stamp>.evaraproj`) is **never** deleted by Evara | **Yes** — consistent with `backup.rs`'s rule. Cost: disk grows by one project per replacement | It is a standing disk cost the user lives with |
| **A4** | One **settings** migration, `V2__pending_replacement.sql`, for D11's journal | **Yes.** No project migration; E-21 proves the settings database has its own chain | Revision 1 claimed M10 added no migration at all; this corrects it, and you should know |
| **A5** | Fix `backup.rs`'s 0-byte leftover (E-01 + E-22) inside Stage 1 rather than as its own PR | **Yes** — M10 shares the primitive, and a separate PR would conflict with Stage 1 | It is a pre-existing defect in shipped code, not M10's |
| **A6** | A `--without-history` export option | **Not in M10**, decided by T-60's measurement. If the feed dominates, add it with the omission **declared** in the manifest (`changeLog.omitted: true`) so an importing build knows the feed starts empty | It is a scope question, and the measurement does not exist yet |
| **A7** | Scoped per-school export, and merge-into-existing | **Neither in M10.** A per-school export is a *split*, and it must decide what happens to a building shared across two schools (ADR 0019's deliberate exception) — answering that badly produces two projects that each believe they own a room. Merge needs identity remapping, which is where a transfer stops being lossless. Worth noting D1 makes a future merge *easier*: two SQLite files can be `ATTACH`ed and reconciled with set-based SQL | Both are features, deferred with reasons rather than refused |

**Also recorded, not needing approval:** `.evara` file associations are Phase 11 packaging, not
M10 (§3.4), and M10 needs **no new Tauri capability** (§1.4.11).

---

## 11. Proposed ADRs

Seven, from 0053. Each is written only after its decision is settled.

| # | Title | Supersedes / amends | Why it needs a record |
| --- | --- | --- | --- |
| **0053** | `.evara` carries a verified SQLite snapshot, not a directory of JSON | **Supersedes [0004](adr/0004-evara-project-format.md)'s container decision**; every other clause of 0004 survives | Reverses an accepted decision and rejects the alternative the record currently mandates. §1.4.5 is the argument: 0004's rejection rested on a migration burden `CLAUDE.md` §3 already requires us to carry |
| **0054** | Two version axes: `formatVersion` is the container, the schema version is the data | Amends [0004](adr/0004-evara-project-format.md) | 0004 made them independent without saying which refuses what. A reader needs to know which number refuses their import, and that one axis is an already-implemented mechanism |
| **0055** | A transfer archive is read by name and never extracted to a path | — | Answers the whole traversal class structurally rather than by validation, and rejects the obvious alternative — iterate the directory and sanitise each name — which is where every traversal CVE in every language has come from. §1.4.9 makes it current |
| **0056** | A payload's schema is checked against a generated reference before anything writes to it | — | Rejects two reasonable alternatives with evidence: a hand-written allow-list (stale on every migration — 74 migration triggers, ~114 change-log triggers) and `trusted_schema = OFF` during migration (breaks every change-log trigger, because `evara_site_id()` is not `SQLITE_INNOCUOUS`, E-15). It also records that conformance precedes both the migration *and* the identity read |
| **0057** | An imported project is staged, verified and renamed; the archive is its own safety copy | Amends [0008](adr/0008-installation-identity-and-project-lifecycle.md) in one respect | Why the import path skips `backup::before_schema_rebuild` (redundant when the source is a checksummed archive), why staging is a sibling of the destination (a cross-volume rename fails at the worst moment), and why an import rebuilds no derived state (§1.4.7) |
| **0058** | Replacement is journalled and non-destructive; the replaced project is kept, never deleted | — | **New in revision 2.** States the filesystem guarantee relied on and the one not claimed, the three-phase journal and its deterministic recovery, and the Windows/macOS asymmetry. It rejects the plain "rename over it" that revision 1 implied, and rejects deleting the old project on success |
| **0059** | A transfer archive's integrity is a two-level tree, and it proves corruption not authenticity | — | **New in revision 2.** Records the acyclic model, the fixed verification order, why the manifest is the deliberately unverified root, why `integrity.algorithm` admits one value, and why there is no signature (ADR 0001 forbids the key distribution a signature needs) |

Three smaller amendments need only an edit to the document that is wrong, recorded in Stage 6:

- `violation` / `violation_ref` **are** exported — PROJECT-FORMAT §3, TIMETABLE-PERSISTENCE
  §7's table, ADR 0042 §C4's table. ADR 0042's decision is untouched; only its export contract
  is corrected, and §1.4.1 is the reason.
- `attachments/` is removed from v1 and reserved by name — PROJECT-FORMAT §1, already implied
  by ADR 0016.
- `capabilities/default.json`'s description no longer predicts "file access in Phase 9" —
  §1.4.11.

New [OPEN-DECISIONS.md](OPEN-DECISIONS.md) entries: **D37** (A6, the history option, pending
T-60), **D38** (A7, scoped export and merge, both deferred with reasons), **R11** (a
transferred archive is unsigned — corruption is detected, authenticity is not), and **R12**
(macOS allows a rename over a file another process holds open, so a second installation
holding the replaced project cannot be detected; the kept copy is the mitigation). **R8 is
discharged**: `.evara` export becomes the blessed way to move a project, which is exactly the
mitigation R8 named.

---

## 12. What this changes in the record

| Document | Change |
| --- | --- |
| [PROJECT-FORMAT.md](PROJECT-FORMAT.md) | Rewritten: container, manifest, payload, integrity model, export, import, replacement, compatibility, tests |
| [adr/0004](adr/0004-evara-project-format.md) | Container decision superseded by 0053; status becomes *Superseded in part by 0053*. Every other clause survives |
| [ARCHITECTURE.md](ARCHITECTURE.md) | §2's diagram gains the real `evara-project`; §3 gains a rule for it and, pending A1, notes `Cancel` in `evara-core` |
| [ROADMAP.md](ROADMAP.md) | Phase 9 marked complete with the departures stated, in the house style of Phases 7 and 8. M1-17 and M1-18 closed |
| [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) | §7's export column corrected for `violation` / `violation_ref` and for `change_log` |
| [adr/0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) | §C4's export table corrected; the decision untouched |
| [OPEN-DECISIONS.md](OPEN-DECISIONS.md) | R8 discharged; D37, D38, R11, R12 added |
| `capabilities/default.json` | Description corrected — no new capability was needed |
| `CLAUDE.md` §3 | One new bullet: a transfer archive carries a verified snapshot, is read by name and never extracted to a path, and an import is staged and renamed rather than inserted — and the project it replaces is kept |

---

## Appendix — decisions and risks in brief

**Decisions.** The payload is a verified snapshot, so completeness is structural and the
migration chain is the one already shipped (D1). Two version axes, one refusal each (D2).
`capabilities` computed from data (D3). The change log travels complete (D4). Every
evaluation, finding, lock and lineage edge travels (D5). Nothing is extracted to a path from
the archive (D6). Import stages, verifies, migrates, then renames, and rebuilds nothing (D7).
Export holds the workspace lock for the snapshot alone — one pinned read transaction since
revision 2.2, not one statement — and every manifest fact comes from the snapshot (D8). The session is the generation session's state machine, and an import does not
open what it imported (D9). Conformance against a generated reference, before any write (D10).
Replacement is journalled, non-destructive and deterministically recoverable (D11). The engine
is in `evara-db`, the format in `evara-project` (D12). Identity is preserved and duplication is
disclosed (D13).

**Risks, ranked.**

1. **The staged payload is a file from elsewhere opened with SQLite.** The controls are real,
   layered and proven available (§6.1), and they do not make the engine's parser infallible.
   §6.2 says so.
2. **Replacement durability is not crash-proof, only crash-recoverable.** D11 states exactly
   which filesystem property is relied on. The reachable-state set is the guarantee, not
   `fsync`.
3. **macOS lets a rename over an open file succeed.** A second installation holding the
   replaced project cannot be detected. R12; the kept copy is the mitigation.
4. **`change_log` size is still unmeasured**, and so is the snapshot's free-page inflation
   (§7.2). T-60 decides A6, and it is the one component that grows with use rather than with
   school size. Both are **Stage 2** since revision 2.2, which is also the first stage with a
   fixture either figure can honestly be measured against.
5. ~~**Stage P is a breaking dependency bump across two 0.x crates.**~~ **Retired — it
   happened.** PR #35 landed it with **no source change** (E-26), the licence question came
   out clean (E-23), and all three advisories are resolved (E-24). What remains of this risk
   is only that `zip 9`'s *reader* is still unexercised by Evara, because Stage 2 has not been
   written; D12's `flate2` + `crc32fast` fallback is the contingency.
6. **The compatibility matrix is generated, not archival.** `run_up_to(11)` today is not
   bit-for-bit what a 2026 build wrote at V11. Much better than the nothing that exists now;
   not the same as having kept the old binaries.
7. **The GUI smoke-test gap M9A left is still open**, and Stage 5 adds two screens to it. M10
   should not widen it further without saying so.
8. **The frontend suite is flaky under full-suite load, and `npm run check` is therefore not
   a reliable gate today.** Two jsdom tests time out against vitest's default 5 000 ms and
   pass in isolation; the failing set moves between runs (§C.8). Stage P met it and reported
   it rather than raising a timeout or skipping a test. It is **carried into Stage 1 unfixed**,
   and it means a red `npm run check` must be read before it is believed — which is exactly
   the condition under which a real regression gets waved past as "the known flake". Worth
   fixing on its own branch, not inside a transfer stage.
