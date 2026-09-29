/**
 * Screenshots for a feedback report: turn whatever was pasted or picked into
 * a small image that is still readable.
 *
 * Every screenshot is committed to the feedback repository, so size matters
 * more than fidelity — but a bug report whose text you can't read is useless.
 * The balance: cap the long edge at 1920 px (a full 1080p screen, a 4K one
 * halved, where UI text is still legible), encode as WebP (JPEG where the
 * WebView cannot write WebP, which is WebKit), and walk quality and then
 * size down until the file is under ~400 KB. A typical screenshot lands
 * between 80 and 300 KB; a 4K PNG of several megabytes comes out at a tenth.
 *
 * Re-encoding also strips metadata, so a photo's EXIF (location included)
 * never leaves the machine.
 *
 * The search is a pure function over an injected encoder, so the policy is
 * tested without a canvas; `compressScreenshot` wires it to the real one.
 */

export type ScreenshotType = "image/png" | "image/jpeg" | "image/webp";

export interface Screenshot {
  mediaType: ScreenshotType;
  /** Plain base64, no `data:` prefix. */
  data: string;
  bytes: number;
  width: number;
  height: number;
}

/** Same caps as `src-tauri/src/feedback.rs` and the Worker. */
export const MAX_SCREENSHOTS = 3;
export const MAX_SCREENSHOT_BYTES = 1024 * 1024;

export const MAX_EDGE = 1920;
export const TARGET_BYTES = 400 * 1024;
/** Refuse to even decode anything larger than this. */
export const MAX_INPUT_BYTES = 40 * 1024 * 1024;
const QUALITIES = [0.85, 0.75, 0.65];
const SHRINK = 0.8;
const MIN_EDGE = 640;

export function fitWithin(
  width: number,
  height: number,
  maxEdge: number,
): { width: number; height: number } {
  const long = Math.max(width, height);
  if (long <= maxEdge) return { width, height };
  const scale = maxEdge / long;
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

export interface Encoded {
  type: string;
  bytes: number;
  blob: Blob;
}

/** Draws the source at a size and encodes it; the only part needing a canvas. */
export type Encoder = (
  width: number,
  height: number,
  type: ScreenshotType,
  quality: number,
) => Promise<Encoded>;

export interface Chosen {
  encoded: Encoded;
  width: number;
  height: number;
}

/**
 * Try quality steps at the capped size, then shrink and try again, until the
 * result is under `TARGET_BYTES`. If nothing gets there before the size floor,
 * the smallest attempt wins; the caller still enforces the hard cap.
 */
export async function chooseEncoding(
  sourceWidth: number,
  sourceHeight: number,
  encode: Encoder,
): Promise<Chosen> {
  let { width, height } = fitWithin(sourceWidth, sourceHeight, MAX_EDGE);
  let type: ScreenshotType = "image/webp";
  let best: Chosen | null = null;

  for (;;) {
    for (let step = 0; step < QUALITIES.length; step++) {
      const encoded = await encode(width, height, type, QUALITIES[step]);
      // WebKit silently writes PNG when asked for WebP. Lossy PNG does not
      // exist, so switch to JPEG and redo this step.
      if (type === "image/webp" && encoded.type !== "image/webp") {
        type = "image/jpeg";
        step--;
        continue;
      }
      if (!best || encoded.bytes < best.encoded.bytes) {
        best = { encoded, width, height };
      }
      if (encoded.bytes <= TARGET_BYTES) return best;
    }
    if (Math.max(width, height) * SHRINK < MIN_EDGE) return best!;
    width = Math.max(1, Math.round(width * SHRINK));
    height = Math.max(1, Math.round(height * SHRINK));
  }
}

export type ScreenshotError = "notImage" | "tooLarge" | "unreadable";

export class ScreenshotRejected extends Error {
  constructor(readonly reason: ScreenshotError) {
    super(reason);
  }
}

/** Human-sized bytes for a thumbnail's tooltip: "184 KB", "1.2 MB". */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function blobToBase64(blob: Blob): Promise<string> {
  const buffer = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  for (let i = 0; i < buffer.length; i += 0x8000) {
    binary += String.fromCharCode(...buffer.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/** The images a paste carries, if any (a screenshot, or copied image files). */
export function imagesFromClipboard(data: DataTransfer | null): File[] {
  if (!data) return [];
  const files: File[] = [];
  for (const item of Array.from(data.items)) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (file) files.push(file);
    }
  }
  return files;
}

export async function compressScreenshot(file: Blob): Promise<Screenshot> {
  if (!file.type.startsWith("image/")) throw new ScreenshotRejected("notImage");
  if (file.size > MAX_INPUT_BYTES) throw new ScreenshotRejected("tooLarge");

  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new ScreenshotRejected("unreadable");
  }

  try {
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d");
    if (!context) throw new ScreenshotRejected("unreadable");

    const encode: Encoder = (width, height, type, quality) => {
      canvas.width = width;
      canvas.height = height;
      // Screenshots with transparent regions would turn black as JPEG.
      context.fillStyle = "#ffffff";
      context.fillRect(0, 0, width, height);
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = "high";
      context.drawImage(bitmap, 0, 0, width, height);
      return new Promise((resolve, reject) =>
        canvas.toBlob(
          (blob) =>
            blob
              ? resolve({ type: blob.type, bytes: blob.size, blob })
              : reject(new ScreenshotRejected("unreadable")),
          type,
          quality,
        ),
      );
    };

    const chosen = await chooseEncoding(bitmap.width, bitmap.height, encode);
    const blob = chosen.encoded.blob;
    if (blob.size > MAX_SCREENSHOT_BYTES)
      throw new ScreenshotRejected("tooLarge");
    return {
      mediaType: chosen.encoded.type as ScreenshotType,
      data: await blobToBase64(blob),
      bytes: blob.size,
      width: chosen.width,
      height: chosen.height,
    };
  } finally {
    bitmap.close();
  }
}
