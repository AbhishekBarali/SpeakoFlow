import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  getProviderBrand,
  ProviderTile,
  type ProviderKind,
} from "./ProviderLogos";

/** Ids exactly as the backend stores them. Keep in sync with
 *  `post_process_providers` / the cloud STT registry in settings.rs,
 *  `assistant_tts_engine` in tts.rs, and the web-search switch. */
const IDS: Record<ProviderKind, string[]> = {
  llm: [
    "openai",
    "zai",
    "openrouter",
    "anthropic",
    "groq",
    "cerebras",
    "gemini",
    "xai",
    "deepseek",
    "mistral",
    "moonshot",
    "together",
    "fireworks",
    "perplexity",
    "azure_openai",
    "apple_intelligence",
    "bedrock_mantle",
    "builtin",
    "local",
    "custom",
  ],
  stt: [
    "elevenlabs",
    "groq",
    "openai",
    "openrouter",
    "deepgram",
    "mistral",
    "custom",
  ],
  tts: ["kokoro", "openai", "openrouter", "elevenlabs", "azure"],
  search: ["serper", "brave", "tavily", "exa", "serpapi", "tinyfish"],
};

/** Ids with no brand of their own, rendered with a lucide glyph on purpose:
 *  our own engine, user-defined endpoints, the open-source local voice, and
 *  search backends with no open-source monochrome mark — each with its own
 *  glyph, so a grid of search backends never shows three identical tiles. */
const LUCIDE: Record<string, string> = {
  builtin: "lucide-cpu",
  custom: "lucide-plug",
  kokoro: "lucide-audio-lines",
  serper: "lucide-search",
  serpapi: "lucide-search-code",
  tinyfish: "lucide-fish",
};

const FALLBACK_TILE = "bg-surface-strong text-muted";

const markup = (id: string, kind?: ProviderKind) =>
  renderToStaticMarkup(<>{getProviderBrand(id, kind).icon}</>);

describe("provider brands", () => {
  for (const [kind, ids] of Object.entries(IDS) as [ProviderKind, string[]][]) {
    for (const id of ids) {
      test(`${kind}:${id} resolves to a real mark`, () => {
        const brand = getProviderBrand(id, kind);
        const html = markup(id, kind);
        expect(brand.tileClass.trim().length).toBeGreaterThan(0);
        expect(brand.tileClass).not.toBe(FALLBACK_TILE);
        expect(html).not.toContain("lucide-sparkles");
        expect(html).toMatch(/ d="[^"]+"/);
        expect(html).toContain('aria-hidden="true"');
        const expected =
          kind === "stt" && id === "custom" ? "lucide-mic" : LUCIDE[id];
        if (expected) {
          expect(html).toContain(expected);
        } else {
          // A brand mark, not an icon-library glyph.
          expect(html).not.toContain("lucide");
          expect(html).toContain('fill="currentColor"');
          expect(html).toContain('viewBox="0 0 24 24"');
        }
      });
    }
  }

  test("kind disambiguates shared ids", () => {
    expect(markup("azure", "tts")).not.toBe(markup("azure_openai", "llm"));
    expect(markup("custom", "stt")).toContain("lucide-mic");
    expect(markup("custom", "llm")).toContain("lucide-plug");
    expect(markup("openai", "tts")).toBe(markup("openai", "llm"));
  });

  test("search backends without a mark are still told apart", () => {
    const glyphs = new Set(
      ["serper", "serpapi", "tinyfish"].map((id) => markup(id, "search")),
    );
    expect(glyphs.size).toBe(3);
  });

  test("loose matching picks up brands in unfamiliar ids", () => {
    expect(markup("my-azure-east")).toBe(markup("azure_openai"));
    expect(markup("bedrock")).toBe(markup("bedrock_mantle"));
    expect(markup("google_gemini")).toBe(markup("gemini"));
    // Order matters: these must not collapse into the OpenAI mark.
    expect(markup("azure_openai")).not.toBe(markup("openai"));
    expect(markup("openrouter_2")).toBe(markup("openrouter"));
  });

  test("unknown ids fall back to the neutral tile", () => {
    const brand = getProviderBrand("definitely-not-a-provider", "llm");
    expect(brand.tileClass).toBe(FALLBACK_TILE);
    expect(markup("definitely-not-a-provider")).toContain("lucide-sparkles");
  });

  test("size reaches the glyph", () => {
    expect(
      renderToStaticMarkup(
        <>{getProviderBrand("anthropic", "llm", 22).icon}</>,
      ),
    ).toContain('width="22"');
  });
});

describe("ProviderTile", () => {
  test.each([
    ["sm", "h-6 w-6 rounded-md", "14"],
    ["md", "h-8 w-8 rounded-lg", "17"],
    ["lg", "h-10 w-10 rounded-xl", "20"],
  ] as const)("%s tile", (size, box, glyph) => {
    const html = renderToStaticMarkup(
      <ProviderTile id="groq" kind="llm" size={size} className="extra" />,
    );
    expect(html).toContain("grid shrink-0 place-items-center");
    expect(html).toContain(box);
    expect(html).toContain("extra");
    expect(html).toContain(`width="${glyph}"`);
  });
});
