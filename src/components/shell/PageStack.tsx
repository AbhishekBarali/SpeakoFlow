import React, { useLayoutEffect, useRef, useState } from "react";
import { HomePage } from "@/components/pages/HomePage";
import { InsightsPage } from "@/components/pages/InsightsPage";
import { HistoryPage } from "@/components/pages/HistoryPage";
import { AssistantPage } from "@/components/pages/AssistantPage";
import { MeetingsPage } from "@/components/pages/MeetingsPage";
import { CleanupPage } from "@/components/pages/CleanupPage";
import { DictionaryPage } from "@/components/pages/DictionaryPage";
import { ModelsPage } from "@/components/pages/ModelsPage";
import { PortalTargetProvider } from "@/components/ui/portal";
import { Freeze } from "./Freeze";
import { useNavigation, type PageId } from "./navigation";

const PAGES: Record<PageId, React.ComponentType> = {
  home: HomePage,
  insights: InsightsPage,
  history: HistoryPage,
  assistant: AssistantPage,
  meetings: MeetingsPage,
  cleanup: CleanupPage,
  dictionary: DictionaryPage,
  models: ModelsPage,
};

/**
 * One layer per page. A layer is its own scroll container, so every page keeps
 * its scroll position when you leave and come back, and the layers stack in the
 * same box so switching never shifts the layout.
 */
const PageLayer: React.FC<{
  id: PageId;
  active: boolean;
  children: React.ReactNode;
}> = ({ id, active, children }) => {
  const ref = useRef<HTMLDivElement>(null);
  // Recorded on every scroll rather than read at hide time: by the time a
  // layer learns it is being hidden, its content is already display:none and
  // the browser has clamped the offset to zero.
  const scrollTop = useRef(0);
  // This page's own root for menus, tooltips and dialogs (see ui/portal.ts). A
  // sibling of the content, not inside it: the content animates `translate`
  // on reveal, which would make it the containing block for `position: fixed`.
  const [portalRoot, setPortalRoot] = useState<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    element.inert = !active;
    if (active) element.scrollTop = scrollTop.current;
  }, [active]);

  return (
    <div
      ref={ref}
      data-page={id}
      aria-hidden={!active}
      onScroll={(event) => {
        if (active) scrollTop.current = event.currentTarget.scrollTop;
      }}
      className={`absolute inset-0 overflow-y-auto overflow-x-hidden ${
        active ? "page-layer-active" : "page-layer-hidden"
      }`}
    >
      <div className="page-layer-content min-h-full">
        <PortalTargetProvider value={portalRoot}>
          <Freeze freeze={!active}>{children}</Freeze>
        </PortalTargetProvider>
      </div>
      <div ref={setPortalRoot} className="page-layer-portal" />
    </div>
  );
};

/**
 * Every page the user has opened, kept alive; only the current one is live.
 *
 * Pages mount on first visit (not up front, so launch stays cheap) and then
 * stay mounted, frozen while hidden (see `Freeze`). Coming back to a page is a
 * reveal of what is already rendered — no refetch, no skeleton, no lost scroll.
 */
export const PageStack: React.FC = () => {
  const { page } = useNavigation();
  const [visited, setVisited] = useState<PageId[]>([page]);
  // Derived during render (not in an effect) so a first visit paints in the
  // same frame as the click.
  if (!visited.includes(page)) {
    setVisited([...visited, page]);
  }

  return (
    <div className="relative min-h-0 flex-1">
      {visited.map((id) => {
        const Component = PAGES[id] ?? HomePage;
        return (
          <PageLayer key={id} id={id} active={id === page}>
            <Component />
          </PageLayer>
        );
      })}
    </div>
  );
};
