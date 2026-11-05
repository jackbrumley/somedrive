import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { coalesced } from "./coalesce";

/** One area's data as the store holds it. */
export interface AreaState<T> {
  /** Null until the first read finishes. */
  data: T | null;
  /** Why the last read failed; the last good data is kept. */
  error: string | null;
}

/**
 * Reads `read(key)` when the area appears on screen and again whenever its
 * change counter (`revision`) moves, one read at a time. Nothing is read
 * while `enabled` is false. Data read for another key is never returned.
 */
export function useRevisionedRead<T>(
  key: string,
  revision: number | undefined,
  read: (key: string) => Promise<T>,
  enabled = true
): AreaState<T> {
  const [state, setState] = useState<AreaState<T> & { key: string }>({ key, data: null, error: null });
  const latest = useRef({ key, read });
  latest.current = { key, read };
  const mounted = useRef(true);

  const reload = useMemo(
    () =>
      coalesced(async () => {
        const { key: readKey, read: readArea } = latest.current;
        try {
          const data = await readArea(readKey);
          if (mounted.current) {
            setState({ key: readKey, data, error: null });
          }
        } catch (reason) {
          if (mounted.current) {
            setState((current) => ({
              key: readKey,
              data: current.key === readKey ? current.data : null,
              error: String(reason),
            }));
          }
        }
      }),
    []
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (enabled) {
      reload();
    }
  }, [key, revision, enabled, reload]);

  if (!enabled || state.key !== key) {
    return { data: null, error: null };
  }
  return { data: state.data, error: state.error };
}
