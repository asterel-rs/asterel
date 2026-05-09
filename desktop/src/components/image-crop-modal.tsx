import { useCallback, useEffect, useRef, useState } from "react";
import Cropper from "react-easy-crop";
import type { Area } from "react-easy-crop";
import { Check, Crop, X, ZoomIn, ZoomOut } from "lucide-react";
import { useI18n } from "@/lib/i18n";

/* ━━━ Canvas crop extraction ━━━ */

function createImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.addEventListener("load", () => resolve(img));
    img.addEventListener("error", (err) =>
      reject(err instanceof Error ? err : new Error("Image load failed")),
    );
    img.src = src;
  });
}

async function cropImageToFile(
  imageSrc: string,
  pixelCrop: Area,
  originalName: string,
): Promise<File> {
  const image = await createImage(imageSrc);
  const { width, height } = pixelCrop;

  let blob: Blob;

  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas context unavailable");
    ctx.drawImage(image, pixelCrop.x, pixelCrop.y, width, height, 0, 0, width, height);
    blob = await canvas.convertToBlob({ type: "image/webp", quality: 0.92 });
  } else {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas context unavailable");
    ctx.drawImage(image, pixelCrop.x, pixelCrop.y, width, height, 0, 0, width, height);
    blob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error("toBlob produced null"))),
        "image/webp",
        0.92,
      );
    });
  }

  const webpName = /\.[^.]+$/.test(originalName)
    ? originalName.replace(/\.[^.]+$/, ".webp")
    : `${originalName}.webp`;

  return new File([blob], webpName, { type: "image/webp" });
}

/* ━━━ ImageCropModal ━━━ */

interface ImageCropModalProps {
  imageSrc: string;
  fileName: string;
  onConfirm: (file: File) => void;
  onCancel: () => void;
}

export function ImageCropModal({ imageSrc, fileName, onConfirm, onCancel }: ImageCropModalProps) {
  const { t } = useI18n();
  const [crop, setCrop] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [croppedAreaPixels, setCroppedAreaPixels] = useState<Area | null>(null);
  const [processing, setProcessing] = useState(false);
  const [imageAspect, setImageAspect] = useState(4 / 3);
  const modalRef = useRef<HTMLDivElement>(null);

  const handleMediaLoaded = useCallback(
    (mediaSize: { naturalWidth: number; naturalHeight: number }) => {
      if (mediaSize.naturalWidth > 0 && mediaSize.naturalHeight > 0) {
        setImageAspect(mediaSize.naturalWidth / mediaSize.naturalHeight);
      }
    },
    [],
  );

  const handleCropComplete = useCallback((_: Area, pixels: Area) => {
    setCroppedAreaPixels(pixels);
  }, []);

  const handleConfirm = useCallback(async () => {
    if (!croppedAreaPixels) return;
    setProcessing(true);
    try {
      const croppedFile = await cropImageToFile(imageSrc, croppedAreaPixels, fileName);
      onConfirm(croppedFile);
    } catch {
      onCancel();
    } finally {
      setProcessing(false);
    }
  }, [croppedAreaPixels, imageSrc, fileName, onConfirm, onCancel]);

  useEffect(() => {
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handleEsc);
    return () => window.removeEventListener("keydown", handleEsc);
  }, [onCancel]);

  useEffect(() => {
    modalRef.current?.focus();
  }, []);

  return (
    <div
      ref={modalRef}
      tabIndex={-1}
      className="fixed inset-0 z-50 flex items-center justify-center outline-none"
      style={{ background: "color-mix(in oklch, var(--bg) 72%, transparent)" }}
      role="dialog"
      aria-modal="true"
      aria-label={t("Crop image")}
    >
      <button
        type="button"
        className="absolute inset-0 w-full h-full appearance-none border-none bg-transparent cursor-default"
        onClick={onCancel}
        aria-label={t("Dismiss")}
        tabIndex={-1}
      />
      <div
        className="relative flex flex-col"
        style={{
          width: "min(92vw, 640px)",
          maxHeight: "min(88vh, 720px)",
          background: "var(--bg-panel)",
          border: "1px solid var(--border)",
          boxShadow: "var(--shadow-lg)",
          borderRadius: "2px",
        }}
      >
        {/* ── Header ── */}
        <div
          className="flex items-center justify-between shrink-0"
          style={{
            padding: "10px 16px",
            borderBottom: "1px solid var(--border)",
          }}
        >
          <div className="flex items-center gap-2">
            <Crop size={13} strokeWidth={1.5} style={{ color: "var(--fg-muted)" }} />
            <span
              style={{
                fontSize: "11px",
                fontWeight: 600,
                letterSpacing: "0.03em",
                color: "var(--fg-soft)",
              }}
            >
              {t("Crop image")}
            </span>
          </div>
          <button
            type="button"
            onClick={onCancel}
            className="ui-button ui-button-ink"
            aria-label={t("Dismiss")}
          >
            <X size={14} strokeWidth={1.5} />
          </button>
        </div>

        {/* ── Crop area ── */}
        <div className="relative flex-1" style={{ minHeight: "340px", background: "var(--bg)" }}>
          <Cropper
            image={imageSrc}
            crop={crop}
            zoom={zoom}
            aspect={imageAspect}
            onCropChange={setCrop}
            onZoomChange={setZoom}
            onCropComplete={handleCropComplete}
            onMediaLoaded={handleMediaLoaded}
            minZoom={1}
            maxZoom={4}
            showGrid
            style={{
              containerStyle: {
                background: "var(--bg)",
              },
              cropAreaStyle: {
                border: "1.5px dashed oklch(0.56 0.1 158 / 0.5)",
                borderRadius: "0px",
                color: "oklch(0.16 0.01 74 / 0.55)",
              },
            }}
          />
        </div>

        {/* ── Footer: zoom + actions ── */}
        <div
          className="shrink-0"
          style={{
            padding: "12px 16px",
            borderTop: "1px solid var(--border)",
            background: "var(--bg-panel)",
          }}
        >
          {/* Zoom slider */}
          <div className="flex items-center gap-3 mb-3">
            <ZoomOut
              size={13}
              strokeWidth={1.5}
              style={{ color: "var(--fg-muted)", flexShrink: 0 }}
            />
            <input
              type="range"
              min={1}
              max={4}
              step={0.05}
              value={zoom}
              onChange={(e) => setZoom(Number(e.target.value))}
              aria-label={t("Zoom")}
              className="flex-1"
              style={{
                height: "2px",
                accentColor: "var(--accent)",
                cursor: "pointer",
              }}
            />
            <ZoomIn
              size={13}
              strokeWidth={1.5}
              style={{ color: "var(--fg-muted)", flexShrink: 0 }}
            />
            <span
              className="font-mono"
              style={{
                fontSize: "10px",
                fontWeight: 600,
                color: "var(--fg-muted)",
                letterSpacing: "0.03em",
                minWidth: "32px",
                textAlign: "right",
              }}
            >
              {zoom.toFixed(1)}x
            </span>
          </div>

          {/* Action buttons */}
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onCancel}
              className="ui-button ui-button-muted"
              style={{ padding: "7px 14px" }}
            >
              {t("Skip")}
            </button>
            <button
              type="button"
              onClick={handleConfirm}
              disabled={processing || !croppedAreaPixels}
              className="ui-button ui-button-accent-fill"
              style={{ padding: "7px 16px" }}
            >
              <span className="flex items-center gap-1.5">
                <Check size={13} strokeWidth={1.5} />
                {processing ? "..." : t("Crop & attach")}
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
