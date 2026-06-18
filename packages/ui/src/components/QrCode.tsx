import { QRCodeSVG } from "qrcode.react";
import { cn } from "../lib/cn";

export interface QrCodeProps {
  value: string;
  size?: number;
  /** caption rendered below the QR (e.g. truncated URL) */
  caption?: string;
  className?: string;
}

/** Renders a QR on a white tile (so it scans in dark theme too). */
export function QrCode({ value, size = 220, caption, className }: QrCodeProps) {
  return (
    <div className={cn("inline-flex flex-col items-center gap-3", className)}>
      <div className="rounded-lg border border-border bg-white p-4">
        <QRCodeSVG value={value} size={size} level="M" includeMargin={false} />
      </div>
      {caption && <span className="max-w-[260px] break-all text-center text-xs text-muted">{caption}</span>}
    </div>
  );
}
