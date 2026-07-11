/**
 * Extract the GGUF quantization token (e.g. "Q8_0", "Q5_K_M", "Q4_K_M", "Q6_K",
 * "BF16", "F16", "F32") from a model filename.
 *
 * The quant is the trailing `-`-delimited token of a `.gguf` filename, e.g.
 * `parakeet-unified-en-0.6b-Q8_0.gguf` → `Q8_0`. Returns `null` for non-GGUF
 * files (directory/ONNX models) or when the trailing token isn't a recognised
 * quant, so the UI simply omits the tag.
 */
export function extractQuant(filename: string): string | null {
  if (!filename.toLowerCase().endsWith(".gguf")) return null;
  const base = filename.slice(0, filename.length - ".gguf".length);
  const token = base.split("-").pop() ?? "";
  // Recognised quant shapes: Q<n>[_...], IQ<n>[...], BF16, F16, F32.
  return /^(Q\d[\w]*|IQ\d[\w]*|BF16|F16|F32)$/i.test(token)
    ? token.toUpperCase()
    : null;
}
