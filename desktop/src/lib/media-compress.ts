/**
 * Client-side media compression before upload.
 *
 * - Images: Canvas API resize + quality reduction (JPEG/WebP)
 * - Video/Audio: passthrough (MediaBunny integration point for future)
 */

const MAX_IMAGE_DIMENSION = 2048;
const JPEG_QUALITY = 0.8;
const COMPRESS_THRESHOLD_BYTES = 512 * 1024; // 512KB — skip tiny images

/**
 * Compress an image File using Canvas API.
 * Returns a smaller File (or the original if already small enough).
 */
export async function compressImage(
  file: File,
  opts?: {
    maxDimension?: number;
    quality?: number;
  },
): Promise<File> {
  // Skip non-images
  if (!file.type.startsWith("image/")) return file;
  // Skip small files
  if (file.size < COMPRESS_THRESHOLD_BYTES) return file;
  // Skip SVGs and GIFs (lossy compression would break them)
  if (file.type === "image/svg+xml" || file.type === "image/gif") return file;

  const maxDim = opts?.maxDimension ?? MAX_IMAGE_DIMENSION;
  const quality = opts?.quality ?? JPEG_QUALITY;

  try {
    const bitmap = await createImageBitmap(file);
    const { width, height } = bitmap;

    // Calculate target dimensions preserving aspect ratio
    let targetW = width;
    let targetH = height;
    if (width > maxDim || height > maxDim) {
      const ratio = Math.min(maxDim / width, maxDim / height);
      targetW = Math.round(width * ratio);
      targetH = Math.round(height * ratio);
    }

    // If no resize needed and file is reasonably small, skip
    if (targetW === width && targetH === height && file.size < 2 * 1024 * 1024) {
      bitmap.close();
      return file;
    }

    // Draw to offscreen canvas
    const canvas = new OffscreenCanvas(targetW, targetH);
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      bitmap.close();
      return file;
    }
    ctx.drawImage(bitmap, 0, 0, targetW, targetH);
    bitmap.close();

    // Encode — prefer WebP, fallback to JPEG
    const outputType = supportsWebP() ? "image/webp" : "image/jpeg";
    const blob = await canvas.convertToBlob({
      type: outputType,
      quality,
    });

    // Only use compressed version if it's actually smaller
    if (blob.size >= file.size) return file;

    const ext = outputType === "image/webp" ? ".webp" : ".jpg";
    const compressedName = file.name.replace(/\.[^.]+$/, ext);

    return new File([blob], compressedName, {
      type: outputType,
      lastModified: file.lastModified,
    });
  } catch (error) {
    console.warn("[media-compress] image compression failed, using original", error);
    return file;
  }
}

/**
 * Compress a media file before upload.
 * Routes to appropriate compressor based on MIME type.
 */
export async function compressMedia(file: File): Promise<File> {
  if (file.type.startsWith("image/")) {
    return compressImage(file);
  }

  // Video/audio: passthrough for now
  // Future: integrate MediaBunny for video transcoding
  return file;
}

let _webpSupported: boolean | null = null;

function supportsWebP(): boolean {
  if (_webpSupported !== null) return _webpSupported;
  try {
    const canvas = new OffscreenCanvas(1, 1);
    // OffscreenCanvas.convertToBlob supports webp in modern browsers
    _webpSupported = typeof canvas.convertToBlob === "function";
  } catch {
    _webpSupported = false;
  }
  return _webpSupported;
}
