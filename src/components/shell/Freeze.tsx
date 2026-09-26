import React, { Fragment, Suspense, useRef } from "react";

/**
 * Keep a subtree mounted while it is off screen, without paying for it.
 *
 * Pages and tabs used to remount on every visit, so each switch re-ran every
 * fetch and effect below it — the "weird delay" on each section — and threw away
 * the scroll position. Keeping them mounted fixes both, but naively it would
 * make every hidden page re-render on every settings change, because the
 * settings store is one subscription for the whole app.
 *
 * Suspending the hidden subtree is what avoids that. React keeps a suspended
 * tree's state and effects, hides its DOM, and renders updates inside it at
 * offscreen (idle) priority instead of synchronously, so a toggle on the page
 * you are looking at never waits for the pages you are not. This is the same
 * trick `react-freeze` uses; it is small enough not to need the dependency.
 *
 * A subtree must be rendered unfrozen at least once: the promise lives in a ref,
 * and a component that throws on its very first render never gets one.
 */

interface Suspension {
  promise?: Promise<void>;
  resolve?: () => void;
}

const Suspender: React.FC<{ freeze: boolean; children: React.ReactNode }> = ({
  freeze,
  children,
}) => {
  const suspension = useRef<Suspension>({}).current;
  if (freeze) {
    if (!suspension.promise) {
      suspension.promise = new Promise<void>((resolve) => {
        suspension.resolve = resolve;
      });
    }
    throw suspension.promise;
  }
  if (suspension.promise) {
    suspension.resolve?.();
    suspension.promise = undefined;
    suspension.resolve = undefined;
  }
  return <Fragment>{children}</Fragment>;
};

export const Freeze: React.FC<{
  freeze: boolean;
  children: React.ReactNode;
}> = ({ freeze, children }) => (
  <Suspense fallback={null}>
    <Suspender freeze={freeze}>{children}</Suspender>
  </Suspense>
);
