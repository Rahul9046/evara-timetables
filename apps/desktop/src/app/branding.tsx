/**
 * Which logo to show, decided in one place.
 *
 * # The fallback rule, and why it lives here
 *
 * A school may upload its own logo. When it has not, every school-specific view shows
 * Evara's. That is one sentence, and it is the kind of sentence that gets re-implemented
 * slightly differently in four screens until two of them disagree about whether an empty
 * string counts as a logo.
 *
 * So it is written once, in {@link resolveLogo}, as a pure function over the value the
 * backend returns. Everything that shows a logo goes through it, including the places that
 * do not exist yet: a future timetable export or printed report asks the same question and
 * gets the same answer, without this file knowing anything about them.
 *
 * # Evara's own mark is never the school's
 *
 * The application's branding — the desktop icon, the installer, the start screen — is
 * always Evara's and is not affected by anything a school uploads. {@link EVARA_LOGO} is
 * for both that and the fallback, which is deliberate: they are the same mark, and a
 * school with no logo is shown the application's, not a blank space.
 */
import { useCallback, useEffect, useState } from "react";

import evaraLogo from "../assets/evara-logo.png";
import type { SchoolLogoView } from "@evara/domain";
import { getSchoolLogo } from "../ipc/setup";

/**
 * The Evara application mark.
 *
 * Imported so the bundler fingerprints it and the `<img>` needs no network and no
 * filesystem — it is the same bytes as the desktop icon. See `assets/README.md`.
 */
export const EVARA_LOGO = evaraLogo;

/** A logo, and whose it is. */
export interface ResolvedLogo {
  /** What to put in an `<img src>`. Never empty. */
  readonly src: string;
  /** Whether this is the school's own, or Evara's standing in for it. */
  readonly isSchoolLogo: boolean;
  /**
   * An accessible name for the image.
   *
   * Names the school when the logo is the school's, because that is what the image
   * conveys. Falls back to naming Evara, so the mark is never announced as an unlabelled
   * image.
   */
  readonly alt: string;
}

/**
 * Picks the logo for a school-specific view.
 *
 * `logo` is whatever `getSchoolLogo()` returned: the school's logo, or `null` for a school
 * that has none. `null` is the ordinary case — a new project, a project upgraded from a
 * schema older than V8, and a school that removed its logo all produce it.
 *
 * Defensive beyond `null` on purpose. A logo whose `dataUrl` is missing or empty is
 * treated as absent rather than rendered as a broken image: the row may have been written
 * by a future build, or damaged, and a recognisable Evara mark is a better answer than a
 * grey box with a torn-paper icon. The backend already declines to render a format it
 * cannot name, so this is the second of two guards rather than the only one.
 */
export function resolveLogo(
  logo: SchoolLogoView | null | undefined,
  schoolName?: string | null,
): ResolvedLogo {
  const src = logo?.dataUrl?.trim();
  if (src !== undefined && src.length > 0) {
    const named = schoolName?.trim();
    return {
      src,
      isSchoolLogo: true,
      alt: named ? `${named} logo` : "School logo",
    };
  }
  return { src: EVARA_LOGO, isSchoolLogo: false, alt: "Evara" };
}

/**
 * The application's own mark, for a branding area that is about Evara rather than a school.
 *
 * Separate from {@link resolveLogo} because it takes no school and can have no fallback:
 * the start screen is Evara's, always, whatever project was last open.
 */
export function evaraBrand(): ResolvedLogo {
  return { src: EVARA_LOGO, isSchoolLogo: false, alt: "Evara" };
}

/**
 * Everything currently showing the school's logo, so a change reaches all of it.
 *
 * The title bar and the School Details panel sit in unrelated parts of the tree, and the
 * panel is three prop-less components below the bar. Threading a callback up through
 * `SchoolSetup` and `SchoolDetails` would add a prop to two components that have no
 * interest in logos; a notifier keeps the whole subject in this module, which is also what
 * "centralise logo selection" asks for.
 *
 * One `Set`, cleared by each subscriber's own effect cleanup, so nothing leaks between
 * tests or across a project being closed.
 */
const listeners = new Set<() => void>();

/**
 * Tells every view showing the school's logo to read it again.
 *
 * Called after an upload or a removal. Deliberately carries no payload: subscribers
 * re-read from the backend rather than being handed a value, so what they show is what is
 * stored rather than what some component believed it stored.
 */
export function schoolLogoChanged(): void {
  for (const listener of [...listeners]) listener();
}

/**
 * The school's current logo, kept in step with uploads and removals anywhere.
 *
 * `undefined` while it is being read for the first time, `null` for a school that uses
 * Evara's. Both resolve to the Evara mark through {@link resolveLogo}, so a view never
 * flickers between two different images on load.
 *
 * A read failure resolves to `null` rather than propagating. A logo that cannot be read is
 * not worth failing a screen over — the fallback is a correct answer — and the panel that
 * owns uploading reports the error properly.
 */
export function useSchoolLogo(): {
  logo: SchoolLogoView | null | undefined;
  reload: () => void;
} {
  const [logo, setLogo] = useState<SchoolLogoView | null | undefined>(undefined);

  const reload = useCallback(() => {
    let live = true;
    getSchoolLogo()
      .then((value) => {
        if (live) setLogo(value);
      })
      .catch(() => {
        if (live) setLogo(null);
      });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const cancel = reload();
    const listener = () => {
      reload();
    };
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      cancel();
    };
  }, [reload]);

  return { logo, reload };
}

/** Renders a resolved logo at a fixed size. */
export function LogoMark(props: {
  logo: ResolvedLogo;
  /** Rendered size in CSS pixels. The source is 128px, so 64 and below stay crisp. */
  size?: number;
  className?: string;
}) {
  const { logo, size = 32, className } = props;
  return (
    <img
      className={className ? `logoMark ${className}` : "logoMark"}
      src={logo.src}
      alt={logo.alt}
      width={size}
      height={size}
      // A logo is decorative-adjacent and never the reason a screen is slow; letting it
      // decode off the main thread keeps a large school logo from stalling a paint.
      decoding="async"
      draggable={false}
    />
  );
}
