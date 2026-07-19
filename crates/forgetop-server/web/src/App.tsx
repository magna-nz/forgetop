import { type ComponentType, useEffect, useState } from "react";
import { useConnections } from "./api";
import { FirstRun } from "./components/FirstRun";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { CommandPalette } from "./components/CommandPalette";
import { PrDetailProvider } from "./components/PrDetail";
import { WiDetailProvider } from "./components/WiDetail";
import { PipelineDetailProvider } from "./components/PipelineDetail";
import { Launchpad } from "./components/Launchpad";
import { PullRequests } from "./components/PullRequests";
import { WorkItems } from "./components/WorkItems";
import { Pipelines } from "./components/Pipelines";
import { Notifications } from "./components/Notifications";
import { Settings } from "./components/Settings";
import { Feedback } from "./components/Feedback";
import { NavContext } from "./nav";
import type { SectionId } from "./types";

const VIEWS: Record<SectionId, ComponentType> = {
  launchpad: Launchpad,
  prs: PullRequests,
  "work-items": WorkItems,
  pipelines: Pipelines,
  notifications: Notifications,
  settings: Settings,
  feedback: Feedback,
};

const SECTIONS: SectionId[] = ["launchpad", "prs", "work-items", "pipelines", "notifications", "settings", "feedback"];
export const sectionFromHash = (): SectionId | undefined => {
  const h = window.location.hash.replace(/^#/, "");
  return SECTIONS.find((s) => s === h);
};

export const shouldShowFirstRun = (section: SectionId, connectionCount: number | undefined, skipped: boolean) =>
  connectionCount === 0 && !skipped && section !== "feedback";

export default function App() {
  // The TUI deep-links here (e.g. `#settings` when you press C), so honour the hash on load.
  const [section, setSection] = useState<SectionId>(() => sectionFromHash() ?? "launchpad");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [skippedFirstRun, setSkippedFirstRun] = useState(false);
  // The sidebar always starts open on each load; the toggle collapses it to give a section the
  // full page width, but that choice is intentionally not persisted across sessions.
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const connections = useConnections();
  const View = VIEWS[section];

  // First launch with nothing configured → the setup wizard, like the TUI.
  const firstRun = shouldShowFirstRun(section, connections.data?.length, skippedFirstRun);

  // Cmd/Ctrl-K toggles the command palette from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Re-opening the dashboard at a new hash (e.g. pressing C again) navigates there.
  useEffect(() => {
    const onHash = () => {
      const s = sectionFromHash();
      if (s) setSection(s);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  if (firstRun) {
    return (
      <FirstRun
        onDone={() => connections.refetch()}
        onSkip={() => setSkippedFirstRun(true)}
      />
    );
  }

  return (
    <PrDetailProvider>
      <WiDetailProvider>
        <PipelineDetailProvider>
          <NavContext.Provider value={setSection}>
          <div className="flex h-full">
            <Sidebar section={section} onSelect={setSection} collapsed={!sidebarOpen} />
            <main className="flex flex-col flex-1 min-w-0 h-full">
              <TopBar
                section={section}
                onOpenPalette={() => setPaletteOpen(true)}
                sidebarOpen={sidebarOpen}
                onToggleSidebar={() => setSidebarOpen((o) => !o)}
              />
              <div className="flex-1 overflow-auto">
                {/* key forces a remount per section so the CSS fade replays; row-level entrance
                    animations live in <Row>. */}
                <div key={section} className="fade-in">
                  <View />
                </div>
              </div>
            </main>
            <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} onNavigate={setSection} />
          </div>
          </NavContext.Provider>
        </PipelineDetailProvider>
      </WiDetailProvider>
    </PrDetailProvider>
  );
}
