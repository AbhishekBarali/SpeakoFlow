import React from "react";
import { AudioLines, Mic, Moon, Sparkles } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";

/**
 * Real brand marks for the model cards (onboarding + catalogs), so a model
 * reads as "NVIDIA / Qwen / Gemma" instead of a generic glyph. All render at
 * `currentColor` so the surrounding tile controls the hue. Paths are the
 * official marks from the simple-icons set (CC0).
 */

interface LogoProps {
  size?: number;
  className?: string;
}

export const NvidiaLogo: React.FC<LogoProps> = ({ size = 20, className }) => (
  <svg
    role="img"
    aria-hidden
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="currentColor"
    className={className}
  >
    <path d="M8.948 8.798v-1.43a6.7 6.7 0 0 1 .424-.018c3.922-.124 6.493 3.374 6.493 3.374s-2.774 3.851-5.75 3.851c-.398 0-.787-.062-1.158-.185v-4.346c1.528.185 1.837.857 2.747 2.385l2.04-1.714s-1.492-1.952-4-1.952a6.016 6.016 0 0 0-.796.035m0-4.735v2.138l.424-.027c5.45-.185 9.01 4.47 9.01 4.47s-4.08 4.964-8.33 4.964c-.37 0-.733-.035-1.095-.097v1.325c.3.035.61.062.91.062 3.957 0 6.82-2.023 9.593-4.408.459.371 2.34 1.263 2.73 1.652-2.633 2.208-8.772 3.984-12.253 3.984-.335 0-.653-.018-.971-.053v1.864H24V4.063zm0 10.326v1.131c-3.657-.654-4.673-4.46-4.673-4.46s1.758-1.944 4.673-2.262v1.237H8.94c-1.528-.186-2.73 1.245-2.73 1.245s.68 2.412 2.739 3.11M2.456 10.9s2.164-3.197 6.5-3.533V6.201C4.153 6.59 0 10.653 0 10.653s2.35 6.802 8.948 7.42v-1.237c-4.84-.6-6.492-5.936-6.492-5.936z" />
  </svg>
);

export const QwenLogo: React.FC<LogoProps> = ({ size = 20, className }) => (
  <svg
    role="img"
    aria-hidden
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="currentColor"
    className={className}
  >
    <path d="M23.919 14.545 20.817 9.17l1.47-2.544a.56.56 0 0 0 0-.566l-1.633-2.83a.57.57 0 0 0-.49-.283h-6.207L12.487.402a.57.57 0 0 0-.49-.284H8.732a.56.56 0 0 0-.49.284L5.139 5.775h-2.94a.56.56 0 0 0-.49.284L.077 8.887a.56.56 0 0 0 0 .567L3.18 14.83l-1.47 2.545a.56.56 0 0 0 0 .566l1.634 2.83a.57.57 0 0 0 .49.283h6.205l1.47 2.545a.57.57 0 0 0 .49.284h3.266a.57.57 0 0 0 .49-.284l3.104-5.375h2.94a.57.57 0 0 0 .49-.283l1.634-2.828a.55.55 0 0 0-.004-.568M8.733.686l1.634 2.828-1.634 2.828H21.8L20.164 9.17H7.425L5.63 6.06Zm1.306 19.801-6.205-.002 1.634-2.83h3.265L2.201 6.344h3.267q3.182 5.517 6.367 11.032zm10.124-5.66L18.53 12l-6.532 11.315-1.634-2.83c2.129-3.673 4.25-7.351 6.373-11.028h3.592l3.102 5.374z" />
  </svg>
);

/** Google's four-point spark — the Gemma / Gemini family mark. */
export const GemmaLogo: React.FC<LogoProps> = ({ size = 20, className }) => (
  <svg
    role="img"
    aria-hidden
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="currentColor"
    className={className}
  >
    <path d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81" />
  </svg>
);

/** OpenAI's knot mark — used for the Whisper family. */
export const OpenAILogo: React.FC<LogoProps> = ({ size = 20, className }) => (
  <svg
    role="img"
    aria-hidden
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="currentColor"
    className={className}
  >
    <path d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.073zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.8956zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654 2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997z" />
  </svg>
);

/** Brand identity a model card renders: the mark plus a softly tinted tile in
 *  the brand's own hue. Tints stay quiet in light mode and lift in dark so the
 *  catalog reads as a scannable list of products, not anonymous rows. */
export interface ModelBrand {
  icon: React.ReactNode;
  /** Classes for the leading icon tile (background wash + glyph color). */
  tileClass: string;
}

const BRAND_TILES = {
  nvidia:
    "bg-[#76b900]/12 text-[#538200] dark:bg-[#76b900]/20 dark:text-[#a3e635]",
  qwen: "bg-[#615ced]/12 text-[#544fe0] dark:bg-[#8b7bff]/20 dark:text-[#b0a6ff]",
  gemma:
    "bg-[#4285f4]/12 text-[#2f6fe4] dark:bg-[#60a5fa]/20 dark:text-[#93c5fd]",
  openai: "bg-ink/8 text-ink/80 dark:bg-ink/15 dark:text-ink",
  moonshine:
    "bg-amber-500/12 text-amber-600 dark:bg-amber-400/20 dark:text-amber-300",
  neutral: "bg-surface-strong text-muted",
} as const;

/**
 * Resolve the brand mark + tile tint for a catalog model. Known families get
 * their real logo (NVIDIA for Parakeet/Canary/Nemotron, OpenAI for Whisper,
 * Qwen, Gemma); everything else falls back to a quiet category glyph so every
 * card keeps the same anatomy.
 */
export const getModelBrand = (model: ModelInfo): ModelBrand => {
  const key = `${model.id} ${model.name}`.toLowerCase();
  if (/nemotron|parakeet|canary/.test(key)) {
    return { icon: <NvidiaLogo size={18} />, tileClass: BRAND_TILES.nvidia };
  }
  if (key.includes("qwen")) {
    return { icon: <QwenLogo size={18} />, tileClass: BRAND_TILES.qwen };
  }
  if (key.includes("gemma")) {
    return { icon: <GemmaLogo size={18} />, tileClass: BRAND_TILES.gemma };
  }
  if (key.includes("whisper")) {
    return { icon: <OpenAILogo size={18} />, tileClass: BRAND_TILES.openai };
  }
  if (key.includes("moonshine")) {
    return {
      icon: <Moon size={17} strokeWidth={2} />,
      tileClass: BRAND_TILES.moonshine,
    };
  }
  const category = getModelCategory(model);
  const icon =
    category === "llm" ? (
      <Sparkles size={17} strokeWidth={2} />
    ) : category === "tts" ? (
      <AudioLines size={17} strokeWidth={2} />
    ) : (
      <Mic size={17} strokeWidth={2} />
    );
  return { icon, tileClass: BRAND_TILES.neutral };
};
