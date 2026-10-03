// Must be first: installs the fake Tauri backend before any module touches it.
import { previewParams } from "./mockBackend";

import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";
import { useTranslation } from "react-i18next";
import { MessageCircle, Mic, PhoneCall } from "lucide-react";
import "@fontsource-variable/inter";
import "@fontsource/instrument-serif/400.css";
import "../App.css";
import "../i18n";
import "@/components/onboarding/onboarding.css";
import { Keycaps } from "@/components/ui/Keycaps";
import {
  ASK_EXAMPLE_IDS,
  AskScene,
  CallScene,
  DictateScene,
  Stage,
  type SceneId,
} from "@/components/onboarding/tourScenes";
import { applyThemePreference, type ThemePreference } from "@/lib/theme";
import "./readme.css";

/*
 * The README's demo: the onboarding tour's three scenes, played one after
 * another on one stage, with a story-style progress strip above them.
 *
 * Dev-only, like the rest of src/preview. It is not a Vite build input. It
 * exists to be recorded by `scripts/readme-media.mjs`, which drives the clock
 * itself so every frame lands on an exact time. The scenes are the tour's own
 * components, so the README shows the surfaces the app draws.
 *
 * URL knobs: ?scenes=dictate,ask,call  (which scenes, in order)
 *            &asks=1..3                (how many quick-ask examples to play;
 *                                       with `scenes=ask` alone, each example
 *                                       gets its own stretch of the strip)
 *            &chrome=0                 (stage only: no progress strip, no title)
 *            &theme=dark|light
 */

/** One full pass of each scene's loop, in ms. These mirror the beat tables in
 *  tourScenes.tsx (DICTATE_BEATS, ASK_BEAT_MS, CALL_BEATS): change them
 *  together, or a scene is cut off or repeats its first beat. */
const DICTATE_MS = 1000 + 700 + 2600 + 900 + 2400;
const ASK_EXAMPLE_MS = [8500, 8500, 8100];
const CALL_MS = 1300 + 2600 + 1000 + 3400 + 900 + 600;

/** Windows defaults, the platform most people download. */
const BINDINGS: Record<SceneId, string> = {
  dictate: "ctrl_left+super",
  ask: "ctrl_left+alt_left",
  call: "ctrl_left+alt_left+c",
};
const ICONS = { dictate: Mic, ask: MessageCircle, call: PhoneCall } as const;
const ALL_SCENES: SceneId[] = ["dictate", "ask", "call"];

const scenes: SceneId[] = (() => {
  const picked = (previewParams.get("scenes") ?? "")
    .split(",")
    .map((id) => id.trim())
    .filter((id): id is SceneId => ALL_SCENES.includes(id as SceneId));
  return picked.length > 0 ? picked : ALL_SCENES;
})();
const asks = Math.min(3, Math.max(1, Number(previewParams.get("asks") ?? 1)));
const showChrome = previewParams.get("chrome") !== "0";

/** One stretch of the progress strip. Normally a scene; when the quick ask
 *  plays alone with several examples, each example is its own stretch, named
 *  by the chip the tour shows for it. */
interface Segment {
  id: string;
  labelKey: string;
  icon: React.ComponentType<{ className?: string }> | null;
  ms: number;
  scene: SceneId;
}

const askOnly = scenes.length === 1 && scenes[0] === "ask" && asks > 1;
const segments: Segment[] = askOnly
  ? ASK_EXAMPLE_IDS.slice(0, asks).map((id, i) => ({
      id,
      labelKey: `onboarding.tour.ask.chips.${id}`,
      icon: null,
      ms: ASK_EXAMPLE_MS[i],
      scene: "ask",
    }))
  : scenes.map((id) => ({
      id,
      labelKey: `onboarding.tour.tabs.${id}`,
      icon: ICONS[id],
      ms:
        id === "dictate"
          ? DICTATE_MS
          : id === "call"
            ? CALL_MS
            : ASK_EXAMPLE_MS.slice(0, asks).reduce((sum, ms) => sum + ms, 0),
      scene: id,
    }));
const total = segments.reduce((sum, segment) => sum + segment.ms, 0);

declare global {
  interface Window {
    __readmeDemo?: { total: number; restart: () => void };
  }
}

const ReadmeDemo: React.FC = () => {
  const { t } = useTranslation();
  const [index, setIndex] = useState(0);
  // Bumped whenever the scene starts over (a new scene, or the loop wrapping
  // round), so a scene that follows itself still remounts at its first beat.
  const [cycle, setCycle] = useState(0);

  useEffect(() => {
    window.__readmeDemo = {
      total,
      restart: () => {
        setIndex(0);
        setCycle((value) => value + 1);
      },
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      const next = (index + 1) % segments.length;
      setIndex(next);
      if (next === 0 || segments[next].scene !== segments[index].scene) {
        setCycle((value) => value + 1);
      }
    }, segments[index].ms);
    return () => window.clearTimeout(timer);
  }, [index, cycle]);

  const scene = segments[index].scene;
  const binding = BINDINGS[scene];
  const sceneProps = { still: false, holdToTalk: true, binding };
  const title = useMemo(
    () => t(`onboarding.tour.${scene}.title`).replace(/\s*\n\s*/g, " "),
    [t, scene],
  );
  const hint =
    scene === "call"
      ? t("onboarding.tour.keys.call")
      : t("onboarding.tour.keys.hold");

  return (
    <div className="readme-demo">
      {showChrome && (
        <>
          <div
            className="readme-progress"
            style={{ "--n": segments.length } as React.CSSProperties}
            aria-hidden="true"
          >
            {segments.map((segment, i) => {
              const Icon = segment.icon;
              const state =
                i < index ? "done" : i === index ? "current" : "next";
              return (
                <div key={segment.id} className="readme-seg" data-state={state}>
                  <span className="readme-seg-label">
                    {Icon && <Icon className="h-4 w-4" aria-hidden="true" />}
                    {t(segment.labelKey)}
                  </span>
                  <span className="readme-seg-track">
                    <span
                      key={`${segment.id}-${cycle}`}
                      className="readme-seg-fill"
                      style={{ animationDuration: `${segment.ms}ms` }}
                    />
                  </span>
                </div>
              );
            })}
          </div>

          <div key={`head-${scene}-${cycle}`} className="readme-head">
            <h1 className="readme-title text-ink">{title}</h1>
            <span className="readme-keys">
              <Keycaps binding={binding} size="md" />
              <span>{hint}</span>
            </span>
          </div>
        </>
      )}

      <Stage scene={scene}>
        {scene === "dictate" ? (
          <DictateScene
            key={`dictate-${cycle}`}
            {...sceneProps}
            compactOverlay={false}
          />
        ) : scene === "ask" ? (
          <AskScene key={`ask-${cycle}`} {...sceneProps} />
        ) : (
          <CallScene key={`call-${cycle}`} {...sceneProps} />
        )}
      </Stage>
    </div>
  );
};

document.documentElement.dataset.platform = "windows";
applyThemePreference((previewParams.get("theme") ?? "dark") as ThemePreference);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ReadmeDemo />
  </React.StrictMode>,
);
