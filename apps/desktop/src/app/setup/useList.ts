/**
 * Loading a list from the backend, with its loading and error states.
 *
 * Every section needs the same three things — the rows, whether they have arrived, and
 * what went wrong if they did not — and needs them for lists it only reads, to fill a
 * picker or a select. (`CrudPanel` owns the editable case.)
 *
 * The returned `reload` is stable, so a section can hand it to a `CrudPanel`'s
 * `onChanged` without re-subscribing on every render.
 */
import { useCallback, useEffect, useState } from "react";

import { describeSetupError } from "../../ipc/setup";

/** The state of one loaded list. */
export interface Listed<T> {
  /** The rows, or `null` until the first load finishes. */
  readonly items: readonly T[] | null;
  readonly error: string | null;
  /** Re-reads the list. Stable across renders. */
  readonly reload: () => void;
}

/**
 * Loads a list when `load` changes.
 *
 * `load` must be memoised by the caller — with `useCallback` over whatever it depends on
 * — because its identity is the signal to re-read. That is deliberate: it makes "reload
 * when the selected campus changes" the caller's explicit decision rather than something
 * this hook guesses at.
 */
export function useList<T>(load: () => Promise<readonly T[]>): Listed<T> {
  const [items, setItems] = useState<readonly T[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    let live = true;
    setItems(null);
    load()
      .then((rows) => {
        if (!live) return;
        setItems(rows);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setItems([]);
        setError(describeSetupError(e));
      });
    // A section can switch parent rows faster than a round trip completes; without this
    // the slower response would land last and show the wrong list.
    return () => {
      live = false;
    };
  }, [load, nonce]);

  return { items, error, reload };
}

/**
 * Loads a single value the same way.
 *
 * Used for the school record and for a cycle's coverage, neither of which is a list.
 */
export function useValue<T>(load: () => Promise<T>): {
  readonly value: T | undefined;
  readonly error: string | null;
  readonly reload: () => void;
} {
  const [value, setValue] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    let live = true;
    setValue(undefined);
    load()
      .then((next) => {
        if (!live) return;
        setValue(next);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setError(describeSetupError(e));
      });
    return () => {
      live = false;
    };
  }, [load, nonce]);

  return { value, error, reload };
}
