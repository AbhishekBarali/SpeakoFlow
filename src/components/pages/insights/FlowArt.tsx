import React, { useMemo } from "react";
import { bandPath, flowBands, type FlowSpec } from "./flowGeometry";

/**
 * The share cards' flowing bands as SVG, for the page: the same geometry, so
 * the summary on the page and the cards it shares are visibly one thing. The
 * bands take their colours from `fills`, outermost first, which are CSS
 * colours so they follow the theme.
 */
export const FlowArt: React.FC<{
  spec: FlowSpec;
  /** The drawing's own size; it is scaled to cover the element. */
  width: number;
  height: number;
  fills: string[];
  className?: string;
}> = ({ spec, width, height, fills, className = "" }) => {
  const paths = useMemo(
    () => flowBands(spec, width, height).map(bandPath),
    [spec, width, height],
  );
  return (
    <svg
      aria-hidden="true"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMaxYMid slice"
      className={`pointer-events-none rtl:-scale-x-100 ${className}`}
    >
      {paths.map((d, index) => (
        <path
          key={index}
          d={d}
          style={{ fill: fills[Math.min(index, fills.length - 1)] }}
        />
      ))}
    </svg>
  );
};
