import React from "react";
import { Page } from "@/components/ui/Page";
import { HistorySettings } from "@/components/settings/history/HistorySettings";

/** Everything you have said, grouped by day. */
export const HistoryPage: React.FC = () => (
  <Page>
    <HistorySettings />
  </Page>
);
