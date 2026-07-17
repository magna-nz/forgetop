import { useEffect, useState } from "react";

export const THEMES = ["slate", "dark", "light", "matrix"] as const;
export type Theme = (typeof THEMES)[number];

const KEY = "forgetop_theme";

function stored(): Theme {
  const t = localStorage.getItem(KEY);
  return (THEMES as readonly string[]).includes(t ?? "") ? (t as Theme) : "slate";
}

/** Applies the saved theme immediately (call once before render to avoid a flash). */
export function initTheme() {
  document.documentElement.dataset.theme = stored();
}

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(stored);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem(KEY, theme);
  }, [theme]);
  const cycle = () => setTheme((t) => THEMES[(THEMES.indexOf(t) + 1) % THEMES.length]);
  return [theme, cycle];
}
