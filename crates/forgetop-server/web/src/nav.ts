import { createContext, useContext } from "react";
import type { SectionId } from "./types";

/** App-wide section navigation, so any view (e.g. the Launchpad "more…" links) can switch sections. */
export const NavContext = createContext<(s: SectionId) => void>(() => {});
export const useNavigateSection = () => useContext(NavContext);
