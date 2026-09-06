import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { readFileSync } from "node:fs";
import type { Plugin } from "vite";

/** Ship the VAD model, worklet and matching WASM runtime locally. No CDN,
 * microphone upload, or first-use model download is needed for turn detection. */
export function voiceAssets(): Plugin {
  const require = createRequire(import.meta.url);
  const vadEntry = require.resolve("@ricky0123/vad-web");
  const ortEntry = createRequire(vadEntry).resolve("onnxruntime-web/wasm");
  const files = new Map([
    ["silero_vad_v5.onnx", resolve(dirname(vadEntry), "silero_vad_v5.onnx")],
    [
      "vad.worklet.bundle.min.js",
      resolve(dirname(vadEntry), "vad.worklet.bundle.min.js"),
    ],
    [
      "ort-wasm-simd-threaded.mjs",
      resolve(dirname(ortEntry), "ort-wasm-simd-threaded.mjs"),
    ],
    [
      "ort-wasm-simd-threaded.wasm",
      resolve(dirname(ortEntry), "ort-wasm-simd-threaded.wasm"),
    ],
  ]);
  return {
    name: "local-conversation-vad",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const name = req.url?.split("?")[0]?.replace(/^\/voice-assets\//, "");
        const path = name && files.get(name);
        if (!path || !req.url?.startsWith("/voice-assets/")) return next();
        res.setHeader(
          "Content-Type",
          name.endsWith("wasm")
            ? "application/wasm"
            : name.endsWith("js")
              ? "text/javascript"
              : "application/octet-stream",
        );
        res.end(readFileSync(path));
      });
    },
    generateBundle() {
      for (const [name, path] of files) {
        this.emitFile({
          type: "asset",
          fileName: `voice-assets/${name}`,
          source: readFileSync(path),
        });
      }
    },
  };
}
