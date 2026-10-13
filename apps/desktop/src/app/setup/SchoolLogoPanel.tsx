/**
 * The "School logo" section of School Details.
 *
 * Optional, and nothing else waits on it: a school with no logo is complete, and the
 * preview shows Evara's mark rather than an empty box so the fallback is visible rather
 * than merely documented.
 *
 * ## Why a file input and not a native dialogue
 *
 * The obvious Tauri answer is `plugin-dialog` for the picker and a filesystem read for the
 * bytes. This uses a plain `<input type="file">` instead, and that is a security decision
 * rather than a convenience one.
 *
 * A native picker hands back a *path*, which means the application needs the capability to
 * read an arbitrary path the frontend names — and then needs to be careful about what it
 * will read. A file input hands back the *bytes* of a file the user chose in an OS-level
 * dialogue the page cannot script. There is no path to traverse, no filesystem capability
 * to grant, and nothing for a bug to turn into an arbitrary-file read. It also means this
 * panel works unchanged under Vitest, where no Tauri runtime exists.
 *
 * The filename is never read at all, and never reaches the backend: the format is decided
 * from the content by `evara_core::branding`.
 *
 * ## Validation happens twice, and the backend is the authority
 *
 * The instant checks below exist so a 40 MB photograph is refused before it is read into
 * memory and base64'd across the IPC boundary, not because they decide anything. The
 * backend re-checks everything from the bytes, and its refusal is what gets shown — the
 * limits used here come from it, over {@link getSchoolLogoLimits}, so the two cannot drift.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { LogoLimits, School, SchoolLogoView } from "@evara/domain";
import {
  clearSchoolLogo,
  describeSetupError,
  getSchoolLogo,
  getSchoolLogoLimits,
  setSchoolLogo,
} from "../../ipc/setup";
import { LogoMark, resolveLogo, schoolLogoChanged } from "../branding";
import { Banner, ConfirmDialog } from "../ui/Shell";

/** Reads a file's bytes as base64, without the `data:` prefix a FileReader adds. */
function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      // `data:<type>;base64,<payload>` — everything after the comma is what the backend
      // wants. Splitting rather than slicing a fixed length because the media type the
      // browser guesses varies and is not trusted anyway.
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

/** A byte count a person can read. */
export function describeBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  const megabytes = bytes / (1024 * 1024);
  // One decimal below ten, none above: "1.8 MB" is useful, "12.4 MB" is noise. A trailing
  // ".0" is dropped, because the limit is written as "2 MB" everywhere a person says it.
  const shown =
    megabytes < 10 ? megabytes.toFixed(1).replace(/\.0$/, "") : String(Math.round(megabytes));
  return `${shown} MB`;
}

/**
 * The instant check, done before a file is read into memory.
 *
 * Returns a message, or `null` to go ahead. Deliberately only the two things knowable
 * without reading the file — its declared type and its size — because everything else
 * needs the bytes, and the backend is already reading those.
 */
export function quickReject(file: File, limits: LogoLimits | null): string | null {
  if (!limits) return null;
  if (file.size > limits.maxBytes) {
    return `That image is ${describeBytes(file.size)}. The limit is ${describeBytes(limits.maxBytes)}.`;
  }
  if (file.size === 0) return "That file is empty.";
  // The browser's guess from the extension, which is exactly what the backend refuses to
  // trust. Used only to catch an obvious mistake early; a renamed file still reaches the
  // backend and is identified properly there.
  if (file.type && !limits.mediaTypes.includes(file.type)) {
    return "That file is not a PNG, JPEG or WebP image.";
  }
  return null;
}

export function SchoolLogoPanel(props: { school: School }) {
  const { school } = props;

  const [logo, setLogo] = useState<SchoolLogoView | null | undefined>(undefined);
  const [limits, setLimits] = useState<LogoLimits | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  const reload = useCallback(async () => {
    try {
      setLogo(await getSchoolLogo(school.id));
    } catch (e: unknown) {
      // A logo that cannot be read is not worth blocking the screen for: the fallback is
      // a correct answer, so the panel shows Evara's mark and says what went wrong.
      setLogo(null);
      setError(describeSetupError(e));
    }
  }, [school.id]);

  useEffect(() => {
    void reload();
    getSchoolLogoLimits()
      .then(setLimits)
      // The limits only drive hint text and the instant pre-check. Without them the
      // backend still enforces everything, so this is not worth an error banner.
      .catch(() => setLimits(null));
  }, [reload]);

  const upload = useCallback(
    async (file: File) => {
      setError(null);
      const refusal = quickReject(file, limits);
      if (refusal !== null) {
        setError(refusal);
        return;
      }

      setBusy(true);
      try {
        const encoded = await readAsBase64(file);
        setLogo(await setSchoolLogo(school.id, encoded));
        // The title bar shows this too, and it is not in this component's tree.
        schoolLogoChanged();
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : describeSetupError(e));
      } finally {
        setBusy(false);
      }
    },
    [limits, school.id],
  );

  const remove = useCallback(async () => {
    setConfirmingRemove(false);
    setBusy(true);
    setError(null);
    try {
      await clearSchoolLogo(school.id);
      setLogo(null);
      schoolLogoChanged();
    } catch (e: unknown) {
      setError(describeSetupError(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const resolved = resolveLogo(logo ?? null, school.name);
  const accept = limits?.mediaTypes.join(",") ?? "image/png,image/jpeg,image/webp";
  const maxLabel = limits ? describeBytes(limits.maxBytes) : "2 MB";

  return (
    <section className="subPanel logoPanel">
      <h3>School logo</h3>
      <p className="muted">
        Optional. Shown on this school&rsquo;s screens, and on timetables and reports when
        those arrive. Without one, Evara&rsquo;s own logo is used.
      </p>

      {error && <Banner tone="error">{error}</Banner>}

      <div className="logoRow">
        <div className="logoPreview">
          <LogoMark logo={resolved} size={96} />
          <p className="hint">
            {logo === undefined
              ? "Loading…"
              : resolved.isSchoolLogo
                ? `${logo?.width}×${logo?.height} · ${describeBytes(logo?.byteSize ?? 0)}`
                : "Evara default"}
          </p>
        </div>

        <div className="logoActions">
          {/* The real control. Hidden from sight but not from the accessibility tree or
              the keyboard, and driven by the button beside it — a bare file input cannot
              be styled to match the rest of the interface, and `display: none` would make
              it unreachable. */}
          <input
            ref={fileInput}
            id="school-logo-file"
            type="file"
            className="srOnly"
            accept={accept}
            disabled={busy}
            onChange={(event) => {
              const file = event.currentTarget.files?.[0];
              // Cleared so choosing the same file twice in a row still fires a change.
              event.currentTarget.value = "";
              if (file) void upload(file);
            }}
          />
          <label htmlFor="school-logo-file" className="buttonish">
            {resolved.isSchoolLogo ? "Replace logo…" : "Upload logo…"}
          </label>

          {resolved.isSchoolLogo && (
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => setConfirmingRemove(true)}
            >
              Remove logo
            </button>
          )}

          <p className="hint">
            PNG, JPEG or WebP, up to {maxLabel}. A square image with a transparent
            background works best — it is shown small and on both light and dark
            backgrounds.
          </p>
          {resolved.isSchoolLogo && logo && logo.width !== logo.height && (
            <p className="hint">
              This logo is not square, so it will be letterboxed where a square one would
              fill the space. That is fine if it is the logo you have.
            </p>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={confirmingRemove}
        title="Remove this school's logo?"
        confirmLabel="Remove logo"
        tone="danger"
        busy={busy}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmingRemove(false)}
      >
        <p>
          The uploaded image is deleted from this campus file. Evara&rsquo;s own logo will
          be used instead, and you can upload a new one at any time.
        </p>
      </ConfirmDialog>
    </section>
  );
}
