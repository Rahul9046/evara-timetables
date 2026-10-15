/**
 * Section 18 — Import data from a spreadsheet.
 *
 * Five steps, in one direction: choose what is being imported, choose the file, say which
 * column is which, review the plan, apply it. Nothing is written before the last step, and
 * the Apply button is unavailable until every row the planner could not decide alone has
 * been answered.
 *
 * ## Why a wizard rather than a form
 *
 * Because the user is not entering data — they are reconciling a file they did not write
 * with records Evara already holds, and the questions that arise are about *rows*, not
 * about fields. A form has no way to ask "row 14 names a teacher you already have under a
 * different name; which is right?", and that question is most of the work.
 *
 * ## The interface holds no rule, and decides nothing
 *
 * Every judgement below comes from the backend's plan: what will be created, what will be
 * updated, what is wrong with a row, and whether Apply may be offered at all
 * ({@link ImportPlan.counts}). This file renders that and sends decisions back. In
 * particular it does **not** decide whether a row is a duplicate, whether two names are
 * the same person, or whether a value is a valid date — doing any of it here would be a
 * second answer to a question `evara-db` has already answered, and the two would drift.
 *
 * ## Why a file input and not a native open dialogue
 *
 * The same security decision `SchoolLogoPanel` documents: a plain `<input type="file">`
 * hands back the *bytes* of a file the user chose in an OS dialogue the page cannot
 * script, where a native picker hands back a path the backend would then need the
 * capability to read. There is no path to traverse and nothing for a bug to turn into an
 * arbitrary-file read.
 *
 * Saving is the other way round, and is a save dialogue: there the path is the thing the
 * user is choosing, the bytes are the backend's own, and the backend never takes content
 * from this window — see {@link saveImportFile}.
 *
 * ## A school switch discards the plan
 *
 * `SchoolSetup` keys the section body on the active school, so switching remounts this
 * component and the plan goes with it (ADR 0023). The session's *file* survives in the
 * backend and is re-read on mount, which is deliberate: the file is not about a school, and
 * making the user find it again would be punishing them for using the school selector. The
 * plan is about a school and does not survive. The backend refuses a fingerprint from
 * another school regardless.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { save as saveDialog } from "@tauri-apps/plugin-dialog";

import type {
  Choice,
  ColumnMapping,
  ImportKind,
  ImportKindView,
  ImportPlan,
  ImportSourceView,
  ImportSummary,
  PlannedRow,
  RowDecision,
  School,
  TemplateKindFormat,
} from "@evara/domain";
import {
  applyImport,
  beginImport,
  cancelImport,
  currentImport,
  importKinds,
  prepareImportReport,
  prepareImportTemplate,
  previewImport,
  saveImportFile,
  selectImportSheet,
} from "../../ipc/import";
import { describeSetupError, isReviewRequired } from "../../ipc/setup";
import { Banner, EmptyState, Loading, Panel } from "../ui/Shell";

/** A message shown above the step the user is on. */
interface Note {
  readonly tone: "ok" | "warning" | "error" | "info";
  readonly text: string;
}

/** Reads a file's bytes as base64, without the `data:` prefix a `FileReader` adds. */
function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const comma = result.indexOf(",");
      if (comma < 0) {
        reject(new Error("That file could not be read."));
        return;
      }
      resolve(result.slice(comma + 1));
    };
    reader.readAsDataURL(file);
  });
}

export function ImportData(props: { school: School }) {
  const { school } = props;

  const [kinds, setKinds] = useState<readonly ImportKindView[] | null>(null);
  const [kind, setKind] = useState<ImportKind>("teachers");
  const [source, setSource] = useState<ImportSourceView | null>(null);
  const [mapping, setMapping] = useState<ColumnMapping>({});
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [decisions, setDecisions] = useState<readonly RowDecision[]>([]);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [note, setNote] = useState<Note | null>(null);
  const [busy, setBusy] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let live = true;
    void importKinds()
      .then((list) => {
        if (live) setKinds(list);
      })
      .catch((error: unknown) => {
        if (live) setNote({ tone: "error", text: describeSetupError(error) });
      });
    // The file survives a school switch and this component does not, so the session is
    // read back rather than assumed absent. See this module's header.
    void currentImport()
      .then((existing) => {
        if (!live || !existing) return;
        setSource(existing);
        setKind(existing.kind);
        setMapping(existing.suggestedMapping);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  /** Everything downstream of the file is now wrong. Said in one place. */
  const resetPlan = useCallback(() => {
    setPlan(null);
    setDecisions([]);
    setSummary(null);
  }, []);

  const chosen = useMemo(
    () => kinds?.find((view) => view.kind === kind) ?? null,
    [kinds, kind],
  );

  const handleTemplate = useCallback(
    (format: TemplateKindFormat) => {
      void (async () => {
        setBusy(true);
        setNote(null);
        try {
          const suggested = await prepareImportTemplate(kind, format);
          const path = await saveDialog({
            title: "Save the import template",
            defaultPath: suggested,
          });
          if (!path) return;
          const written = await saveImportFile(path);
          setNote({
            tone: "ok",
            text: `${written} saved. Fill it in, keeping the headings as they are, and come back.`,
          });
        } catch (error: unknown) {
          setNote({ tone: "error", text: describeSetupError(error) });
        } finally {
          setBusy(false);
        }
      })();
    },
    [kind],
  );

  const handleFile = useCallback(
    (file: File) => {
      void (async () => {
        setBusy(true);
        setNote(null);
        resetPlan();
        try {
          const base64 = await readAsBase64(file);
          const read = await beginImport(kind, file.name, base64);
          setSource(read);
          setMapping(read.suggestedMapping);
          if (read.truncatedAt !== null) {
            setNote({
              tone: "warning",
              text:
                `This file is longer than Evara reads in one go, so it stops at row ${read.truncatedAt}. ` +
                "Split it and import each part, or nothing after that row will be imported.",
            });
          }
        } catch (error: unknown) {
          setNote({
            tone: "error",
            text: error instanceof Error ? error.message : describeSetupError(error),
          });
        } finally {
          setBusy(false);
          // So choosing the same file again re-reads it: without this a user who corrected
          // the file in place and picked it again would get no `change` event at all.
          if (fileInput.current) fileInput.current.value = "";
        }
      })();
    },
    [kind, resetPlan],
  );

  const handleSheet = useCallback(
    (sheet: string) => {
      void (async () => {
        setBusy(true);
        setNote(null);
        resetPlan();
        try {
          const read = await selectImportSheet(sheet);
          setSource(read);
          setMapping(read.suggestedMapping);
        } catch (error: unknown) {
          setNote({ tone: "error", text: describeSetupError(error) });
        } finally {
          setBusy(false);
        }
      })();
    },
    [resetPlan],
  );

  const runPreview = useCallback(
    (next: readonly RowDecision[], keepNote = false) => {
      void (async () => {
        setBusy(true);
        if (!keepNote) setNote(null);
        setSummary(null);
        try {
          setPlan(await previewImport(school.id, mapping, next));
        } catch (error: unknown) {
          setPlan(null);
          setNote({ tone: "error", text: describeSetupError(error) });
        } finally {
          setBusy(false);
        }
      })();
    },
    [school.id, mapping],
  );

  const handleDecision = useCallback(
    (line: number, choice: Choice) => {
      const next = [...decisions.filter((decision) => decision.line !== line), { line, choice }];
      setDecisions(next);
      // Re-previewed rather than patched locally: an answer can change another row's
      // outcome — answering "create a new one" claims a code the next row wanted — and the
      // fingerprint must belong to the plan as the backend computes it.
      runPreview(next);
    },
    [decisions, runPreview],
  );

  const handleApply = useCallback(() => {
    if (!plan) return;
    void (async () => {
      setBusy(true);
      setNote(null);
      try {
        const applied = await applyImport(school.id, mapping, decisions, plan.fingerprint);
        setSummary(applied);
        setPlan(null);
        setNote({
          tone: "ok",
          text:
            `Imported. ${applied.created} created, ${applied.updated} updated, ` +
            `${applied.unchanged} already matched, ${applied.skipped} skipped.`,
        });
      } catch (error: unknown) {
        if (isReviewRequired(error)) {
          setNote({
            tone: "warning",
            text:
              "This campus file changed while the preview was open, so nothing was imported. " +
              "The plan has been re-read — review it and apply again.",
          });
          // Keeping the message: it is the whole explanation for the plan changing
          // under the user, and a re-preview that cleared it would leave them looking at
          // different figures with nothing said about why.
          runPreview(decisions, true);
        } else {
          setNote({ tone: "error", text: describeSetupError(error) });
        }
      } finally {
        setBusy(false);
      }
    })();
  }, [plan, school.id, mapping, decisions, runPreview]);

  const handleReport = useCallback(() => {
    void (async () => {
      setBusy(true);
      try {
        const suggested = await prepareImportReport();
        const path = await saveDialog({
          title: "Save the import report",
          defaultPath: suggested,
        });
        if (!path) return;
        const written = await saveImportFile(path);
        setNote({ tone: "ok", text: `${written} saved.` });
      } catch (error: unknown) {
        setNote({ tone: "error", text: describeSetupError(error) });
      } finally {
        setBusy(false);
      }
    })();
  }, []);

  const handleStartOver = useCallback(() => {
    void (async () => {
      await cancelImport().catch(() => undefined);
      setSource(null);
      setMapping({});
      resetPlan();
      setNote(null);
    })();
  }, [resetPlan]);

  const handleKindChange = useCallback(
    (next: ImportKind) => {
      setKind(next);
      // A mapping is a mapping *of one kind's fields*, so it cannot survive this. The file
      // goes too: its headings were read for the old kind and would suggest nothing useful.
      void cancelImport().catch(() => undefined);
      setSource(null);
      setMapping({});
      resetPlan();
      setNote(null);
    },
    [resetPlan],
  );

  if (kinds === null && note === null) return <Loading what="the import types" />;

  return (
    <div className="section">
      <ImportIntro />

      {note && <Banner tone={note.tone}>{note.text}</Banner>}

      <Panel
        title="1 — What are you importing?"
        note="One kind of record at a time, and each one into the school selected above."
      >
        <div className="importKinds">
          <label htmlFor="import-kind">Record type</label>
          <select
            id="import-kind"
            value={kind}
            disabled={busy}
            onChange={(event) => handleKindChange(event.target.value as ImportKind)}
          >
            {(kinds ?? []).map((view) => (
              <option key={view.kind} value={view.kind}>
                {view.label}
              </option>
            ))}
          </select>
        </div>
        {chosen?.dependsOn && (
          <p className="muted">
            Needs {chosen.dependsOn} to exist already. Rows naming something Evara does not
            hold are reported — nothing is invented to fill the gap.
          </p>
        )}
        <p className="importActions">
          <button type="button" disabled={busy} onClick={() => handleTemplate("workbook")}>
            Download a template (Excel)
          </button>
          <button type="button" disabled={busy} onClick={() => handleTemplate("csv")}>
            Download a template (CSV)
          </button>
        </p>
        <p className="muted">
          The Excel template formats code columns as text, which is what stops a spreadsheet
          turning student <code>00124</code> into <code>124</code> before Evara sees the
          file. If you export from another system instead, format those columns as text
          yourself.
        </p>
      </Panel>

      <Panel
        title="2 — Choose the file"
        note="An .xlsx workbook or a .csv file. Nothing is read from it until you choose it, and nothing is written until step 5."
      >
        <input
          ref={fileInput}
          id="import-file"
          type="file"
          accept=".xlsx,.csv"
          disabled={busy}
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) handleFile(file);
          }}
        />
        {source && (
          <>
            <p className="muted">
              Reading <strong>{source.fileName}</strong> — {source.rowCount}{" "}
              {source.rowCount === 1 ? "row" : "rows"} on sheet <strong>{source.sheet}</strong>.
            </p>
            {source.availableSheets.length > 1 && (
              <div className="importKinds">
                <label htmlFor="import-sheet">Worksheet</label>
                <select
                  id="import-sheet"
                  value={source.sheet}
                  disabled={busy}
                  onChange={(event) => handleSheet(event.target.value)}
                >
                  {source.availableSheets.map((sheet) => (
                    <option key={sheet} value={sheet}>
                      {sheet}
                    </option>
                  ))}
                </select>
              </div>
            )}
            <p className="importActions">
              <button type="button" disabled={busy} onClick={handleStartOver}>
                Start over
              </button>
            </p>
          </>
        )}
      </Panel>

      {source && (
        <>
          <MappingPanel
            source={source}
            mapping={mapping}
            busy={busy}
            onChange={(next) => {
              setMapping(next);
              resetPlan();
            }}
          />

          <Panel
            title="4 — Review what would happen"
            note="A preview reads the campus file and writes nothing to it."
          >
            <p className="importActions">
              <button type="button" disabled={busy} onClick={() => runPreview(decisions)}>
                {plan ? "Preview again" : "Preview"}
              </button>
            </p>
            {plan && <PlanReview plan={plan} busy={busy} onDecide={handleDecision} />}
          </Panel>

          <Panel
            title="5 — Import"
            note="Everything in one go: if any row fails, none of them is imported."
          >
            {plan === null && !summary && (
              <p className="muted">Preview the file first — Apply works from the plan above.</p>
            )}
            {plan && !plan.counts.create && !plan.counts.update && (
              <p className="muted">
                Nothing to import: every row already matches what Evara holds, or was
                skipped.
              </p>
            )}
            {plan && (
              <p className="importActions">
                <button
                  type="button"
                  disabled={busy || blocked(plan) || (!plan.counts.create && !plan.counts.update)}
                  title={blocked(plan) ? "Answer the rows that need a decision first." : undefined}
                  onClick={handleApply}
                >
                  Import {plan.counts.create + plan.counts.update}{" "}
                  {plan.counts.create + plan.counts.update === 1 ? "record" : "records"}
                </button>
              </p>
            )}
            {summary && <Summary summary={summary} busy={busy} onReport={handleReport} />}
          </Panel>
        </>
      )}
    </div>
  );
}

/** Whether the plan still holds something the user has to answer. */
function blocked(plan: ImportPlan): boolean {
  return plan.counts.invalid > 0 || plan.counts.conflict > 0;
}

/** Step 3: which column fills which field. */
function MappingPanel(props: {
  source: ImportSourceView;
  mapping: ColumnMapping;
  busy: boolean;
  onChange: (next: ColumnMapping) => void;
}) {
  const { source, mapping, busy, onChange } = props;

  const used = useMemo(() => new Set(Object.values(mapping)), [mapping]);

  return (
    <Panel
      title="3 — Which column is which?"
      note="Evara's suggestions come from the headings. Change any of them; leave a column unmapped and Evara will not touch that field at all."
    >
      <table className="importMapping">
        <thead>
          <tr>
            <th scope="col">Evara field</th>
            <th scope="col">Column in your file</th>
            <th scope="col">What it is for</th>
          </tr>
        </thead>
        <tbody>
          {source.fields.map((field) => {
            const at = mapping[field.key];
            return (
              <tr key={field.key}>
                <th scope="row">
                  {field.label}
                  {field.required && (
                    <abbr title="Required" aria-label="required">
                      {" *"}
                    </abbr>
                  )}
                </th>
                <td>
                  <select
                    aria-label={`Column for ${field.label}`}
                    value={at === undefined ? "" : String(at)}
                    disabled={busy}
                    onChange={(event) => {
                      const next = { ...mapping };
                      if (event.target.value === "") delete next[field.key];
                      else next[field.key] = Number(event.target.value);
                      onChange(next);
                    }}
                  >
                    <option value="">Not in this file</option>
                    {source.headers.map((heading, index) => (
                      <option key={index} value={index}>
                        {heading === "" ? `Column ${index + 1}` : heading}
                        {used.has(index) && at !== index ? " (already used)" : ""}
                      </option>
                    ))}
                  </select>
                </td>
                <td className="muted">{field.hint}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {source.sample.length > 0 && (
        <>
          <h4>The first few rows, as Evara read them</h4>
          <table className="importSample">
            <thead>
              <tr>
                <th scope="col">Row</th>
                {source.headers.map((heading, index) => (
                  <th key={index} scope="col">
                    {heading === "" ? `Column ${index + 1}` : heading}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {source.sample.map((row) => (
                <tr key={row.line}>
                  <th scope="row">{row.line}</th>
                  {source.headers.map((_, index) => (
                    <td key={index}>{row.cells[index] ?? ""}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </Panel>
  );
}

/** Step 4: the plan, with the rows that need an answer first. */
function PlanReview(props: {
  plan: ImportPlan;
  busy: boolean;
  onDecide: (line: number, choice: Choice) => void;
}) {
  const { plan, busy, onDecide } = props;
  const counts = plan.counts;

  if (plan.rows.length === 0) {
    return (
      <EmptyState title="Nothing in this file">
        The sheet has headings but no rows below them. Check you chose the right worksheet.
      </EmptyState>
    );
  }

  const needsAnswer = plan.rows.filter((row) => row.action === "conflict");
  const wrong = plan.rows.filter((row) => row.action === "invalid");
  const warned = plan.rows.filter(
    (row) => row.action !== "conflict" && row.action !== "invalid" && row.findings.length > 0,
  );

  return (
    <>
      <ul className="importCounts">
        <Count label="to create" value={counts.create} />
        <Count label="to update" value={counts.update} />
        <Count label="already match" value={counts.unchanged} />
        <Count label="skipped" value={counts.skipped} />
        <Count label="need a decision" value={counts.conflict} tone={counts.conflict ? "warn" : undefined} />
        <Count label="cannot be imported" value={counts.invalid} tone={counts.invalid ? "bad" : undefined} />
        <Count label="warnings" value={counts.warnings} />
      </ul>

      {needsAnswer.length > 0 && (
        <>
          <h4>Rows that need a decision ({needsAnswer.length})</h4>
          <p className="muted">
            Evara will not choose for you. A matching code identifies a record; a matching
            name does not, and two people can share one.
          </p>
          <table className="importRows">
            <thead>
              <tr>
                <th scope="col">Row</th>
                <th scope="col">In your file</th>
                <th scope="col">What Evara found</th>
                <th scope="col">Your answer</th>
              </tr>
            </thead>
            <tbody>
              {needsAnswer.map((row) => (
                <tr key={row.line}>
                  <th scope="row">{row.line}</th>
                  <td>
                    {row.sourceLabel}
                    {row.findings.map((finding, index) => (
                      <p className="muted" key={index}>
                        {finding.message}
                      </p>
                    ))}
                  </td>
                  <td>{row.targetLabel ?? "—"}</td>
                  <td>
                    <div className="importChoices">
                      <button type="button" disabled={busy} onClick={() => onDecide(row.line, "update")}>
                        Use this file
                      </button>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onDecide(row.line, "keepExisting")}
                      >
                        Keep what Evara has
                      </button>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onDecide(row.line, "createNew")}
                      >
                        Create a separate record
                      </button>
                      <button type="button" disabled={busy} onClick={() => onDecide(row.line, "skip")}>
                        Skip this row
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      {wrong.length > 0 && (
        <>
          <h4>Rows that cannot be imported ({wrong.length})</h4>
          <p className="muted">
            Correct these in the file and choose it again. Nothing is imported while any of
            them stands.
          </p>
          <RowFindings rows={wrong} />
        </>
      )}

      {warned.length > 0 && (
        <>
          <h4>Worth knowing ({warned.length})</h4>
          <p className="muted">These do not stop the import.</p>
          <RowFindings rows={warned} />
        </>
      )}
    </>
  );
}

/** Rows and what was said about them. */
function RowFindings(props: { rows: readonly PlannedRow[] }) {
  return (
    <table className="importRows">
      <thead>
        <tr>
          <th scope="col">Row</th>
          <th scope="col">In your file</th>
          <th scope="col">What Evara says</th>
        </tr>
      </thead>
      <tbody>
        {props.rows.map((row) => (
          <tr key={row.line}>
            <th scope="row">{row.line}</th>
            <td>{row.sourceLabel}</td>
            <td>
              {row.findings.map((finding, index) => (
                <p key={index} className={finding.severity === "error" ? "" : "muted"}>
                  {finding.field ? `${finding.field}: ` : ""}
                  {finding.message}
                </p>
              ))}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** One figure of the tally. */
function Count(props: { label: string; value: number; tone?: "warn" | "bad" }) {
  return (
    <li className={props.tone ? `importCount importCount--${props.tone}` : "importCount"}>
      <strong>{props.value}</strong> {props.label}
    </li>
  );
}

/** What happened, once it had. */
function Summary(props: { summary: ImportSummary; busy: boolean; onReport: () => void }) {
  const { summary, busy, onReport } = props;
  return (
    <>
      <ul className="importCounts">
        <Count label="created" value={summary.created} />
        <Count label="updated" value={summary.updated} />
        <Count label="already matched" value={summary.unchanged} />
        <Count label="skipped" value={summary.skipped} />
        <Count label="warnings" value={summary.warnings} />
      </ul>
      <p className="importActions">
        <button type="button" disabled={busy} onClick={onReport}>
          Save the report
        </button>
      </p>
      <p className="muted">
        The report lists every row and everything Evara said about it, so the file can be
        corrected away from this screen and imported again. Importing the same file twice is
        safe: rows that already match are left alone.
      </p>
    </>
  );
}

/** What this section is for, and what it will not do. */
function ImportIntro() {
  return (
    <header className="sectionIntro">
      <h2>Import data from a spreadsheet</h2>
      <p>
        For getting a school&rsquo;s existing records into Evara without typing them again.
        Works entirely on this computer: the file is read here, and nothing is sent
        anywhere.
      </p>
      <ul className="importPromises">
        <li>
          <strong>Nothing is written until you say so.</strong> A preview reads the campus
          file and changes nothing in it.
        </li>
        <li>
          <strong>All or nothing.</strong> If any row fails, the whole import is undone —
          there is no half-imported state to clean up.
        </li>
        <li>
          <strong>Existing records are never overwritten silently.</strong> Where a file
          disagrees with what Evara holds, you are asked.
        </li>
        <li>
          <strong>Nothing is invented.</strong> A row naming a subject, course or year level
          that does not exist is reported, not created as a placeholder.
        </li>
        <li>
          <strong>Importing twice is safe.</strong> Rows that already match are left exactly
          as they are, including their revisions.
        </li>
      </ul>
    </header>
  );
}
