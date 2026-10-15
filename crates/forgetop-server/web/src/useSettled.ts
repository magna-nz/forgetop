import { useEffect, useState } from "react";

/**
 * `value` once it has stayed the same for `ms`; `undefined` until then (on mount included).
 *
 * For work that should follow where the user *stopped*, not every place they passed through: a
 * held `j` walks twenty rows in a second, and only the row it comes to rest on is worth a fetch.
 * Callers compare the result to the live value — `useSettled(key, ms) === key` — so a value that
 * moved on stops counting at once rather than `ms` later.
 */
export function useSettled<T>(value: T, ms: number): T | undefined {
  const [settled, setSettled] = useState<T | undefined>(undefined);
  useEffect(() => {
    const t = setTimeout(() => setSettled(() => value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}
