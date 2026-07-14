import { type ComponentType, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { PullRequests } from "./components/PullRequests";
import { WorkItems } from "./components/WorkItems";
import { Pipelines } from "./components/Pipelines";
import { Notifications } from "./components/Notifications";
import type { SectionId } from "./types";

const VIEWS: Record<SectionId, ComponentType> = {
  prs: PullRequests,
  "work-items": WorkItems,
  pipelines: Pipelines,
  notifications: Notifications,
};

export default function App() {
  const [section, setSection] = useState<SectionId>("prs");
  const View = VIEWS[section];

  return (
    <div className="flex h-full">
      <Sidebar section={section} onSelect={setSection} />
      <main className="flex flex-col flex-1 min-w-0 h-full">
        <TopBar section={section} />
        <div className="flex-1 overflow-auto">
          {/* key forces a remount per section so the CSS fade replays; row-level entrance
              animations live in <Row>. */}
          <div key={section} className="fade-in">
            <View />
          </div>
        </div>
      </main>
    </div>
  );
}
