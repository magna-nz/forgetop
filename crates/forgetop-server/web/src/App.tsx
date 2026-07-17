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
import type { SectionId } from "./types";

const VIEWS: Record<SectionId, ComponentType> = {
  launchpad: Launchpad,
  prs: PullRequests,
  "work-items": WorkItems,
  pipelines: Pipelines,
  notifications: Notifications,
  settings: Settings,
};

const SECTIONS: SectionId[] = ["launchpad", "prs", "work-items", "pipelines", "notifications", "settings"];
const sectionFromHash = (): SectionId | undefined => {
  const h = window.location.hash.replace(/^#/, "");
  return SECTIONS.find((s) => s === h);
};

export default function App() {
  // The TUI deep-links here (e.g. `#settings` when you press C), so honour the hash on load.
  const [section, setSection] = useState<SectionId>(() => sectionFromHash() ?? "launchpad");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [skippedFirstRun, setSkippedFirstRun] = useState(false);
  const connections = useConnections();
  const View = VIEWS[section];

  // First launch with nothing configured → the setup wizard, like the TUI.
  const firstRun = connections.data?.length === 0 && !skippedFirstRun;

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
          <div className="flex h-full">
            <Sidebar section={section} onSelect={setSection} />
            <main className="flex flex-col flex-1 min-w-0 h-full">
              <TopBar section={section} onOpenPalette={() => setPaletteOpen(true)} />
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
        </PipelineDetailProvider>
      </WiDetailProvider>
    </PrDetailProvider>
  );
}
