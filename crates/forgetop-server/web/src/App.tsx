import { type ComponentType, useEffect, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { CommandPalette } from "./components/CommandPalette";
import { PrDetailProvider } from "./components/PrDetail";
import { Launchpad } from "./components/Launchpad";
import { PullRequests } from "./components/PullRequests";
import { WorkItems } from "./components/WorkItems";
import { Pipelines } from "./components/Pipelines";
import { Notifications } from "./components/Notifications";
import type { SectionId } from "./types";

const VIEWS: Record<SectionId, ComponentType> = {
  launchpad: Launchpad,
  prs: PullRequests,
  "work-items": WorkItems,
  pipelines: Pipelines,
  notifications: Notifications,
};

export default function App() {
  const [section, setSection] = useState<SectionId>("launchpad");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const View = VIEWS[section];

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

  return (
    <PrDetailProvider>
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
    </PrDetailProvider>
  );
}
