import { describe, expect, test } from "bun:test";
import { prettyModelName, splitLocalModelName } from "./prettyModelName";

describe("prettyModelName", () => {
  test.each([
    // The ids from the Home page that read as raw configuration.
    ["scribe_v2_realtime", "Scribe v2 Realtime"],
    ["google.gemma-4-26b-a4b", "Gemma 4 26B A4B"],
    ["openai/gpt-oss-120b", "GPT-OSS 120B"],
    ["eleven_v3_conversational", "Eleven v3 Conversational"],
    // Hosted catalogs.
    ["microsoft/mai-transcribe-2", "MAI Transcribe 2"],
    ["google/gemini-2.5-flash", "Gemini 2.5 Flash"],
    ["google/gemma-3-27b-it", "Gemma 3 27B IT"],
    ["meta-llama/llama-4-maverick:free", "Llama 4 Maverick"],
    ["moonshotai/kimi-k2-instruct", "Kimi K2 Instruct"],
    ["z-ai/glm-4.6", "GLM 4.6"],
    ["x-ai/grok-4-fast", "Grok 4 Fast"],
    ["llama-3.3-70b-versatile", "Llama 3.3 70B Versatile"],
    ["gpt-4o-mini-tts", "GPT-4o Mini TTS"],
    ["gpt-4.1", "GPT-4.1"],
    ["whisper-large-v3", "Whisper Large v3"],
    ["nova-3", "Nova 3"],
    ["mixtral-8x7b-32768", "Mixtral 8x7B 32768"],
  ])("%s → %s", (raw, pretty) => {
    expect(prettyModelName(raw)).toBe(pretty);
  });

  test("keeps the vendor when it is the model's only name", () => {
    expect(prettyModelName("deepseek.v3.2")).toBe("DeepSeek v3.2");
  });

  test("drops Bedrock's region prefix, release date and revision", () => {
    expect(
      prettyModelName("us.anthropic.claude-sonnet-4-5-20250929-v1:0"),
    ).toBe("Claude Sonnet 4.5 v1");
    expect(prettyModelName("claude-3-5-haiku-latest")).toBe("Claude 3.5 Haiku");
  });

  test("leaves names that are already readable alone", () => {
    expect(prettyModelName("Gemma 4 E4B")).toBe("Gemma 4 E4B");
    expect(prettyModelName("SpeakoFlow Mini")).toBe("SpeakoFlow Mini");
    expect(prettyModelName("Qwen3.5 0.8B")).toBe("Qwen3.5 0.8B");
  });

  test("is empty for empty input", () => {
    expect(prettyModelName("")).toBe("");
    expect(prettyModelName("   ")).toBe("");
    expect(prettyModelName(null)).toBe("");
    expect(prettyModelName(undefined)).toBe("");
  });
});

describe("splitLocalModelName", () => {
  test("moves a parenthesised quant into its own field", () => {
    expect(splitLocalModelName("Tiger Gemma 9B v3 (Q4_K_M)")).toEqual({
      name: "Tiger Gemma 9B v3",
      quant: "Q4_K_M",
    });
  });

  test("reads a quant the file name turned into spaces", () => {
    expect(
      splitLocalModelName(
        "FLOWQwen3.5 0.8B.Q8 0",
        "FLOWQwen3.5-0.8B.Q8_0.gguf",
      ),
    ).toEqual({ name: "FLOWQwen3.5 0.8B", quant: "Q8_0" });
    expect(splitLocalModelName("Qwen3.5 0.8B.Q4 K M")).toEqual({
      name: "Qwen3.5 0.8B",
      quant: "Q4_K_M",
    });
  });

  test("drops a repository owner repeated in front of the name", () => {
    expect(splitLocalModelName("Qwen Qwen3.5 0.8B (Q4_K_M)")).toEqual({
      name: "Qwen3.5 0.8B",
      quant: "Q4_K_M",
    });
  });

  test("falls back to the file name for the quant", () => {
    expect(
      splitLocalModelName("My fine-tune", "my-fine-tune-IQ3_XS.gguf"),
    ).toEqual({ name: "My fine-tune", quant: "IQ3_XS" });
  });

  test("leaves catalog names and their vision suffix tidy", () => {
    expect(splitLocalModelName("Gemma 4 E2B (Vision)")).toEqual({
      name: "Gemma 4 E2B",
      quant: null,
    });
    expect(splitLocalModelName("s1 mini (Q4_K_M)")).toEqual({
      name: "s1 mini",
      quant: "Q4_K_M",
    });
    expect(splitLocalModelName("SpeakoFlow Mini")).toEqual({
      name: "SpeakoFlow Mini",
      quant: null,
    });
  });
});
