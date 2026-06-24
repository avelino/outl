/**
 * Pure ticket → QR-SVG-string helper, shared by every client.
 *
 * Wraps the `qrcode` npm package's `toString(..., { type: "svg" })`
 * API so the matrix-generation logic lives in one place. The output is
 * a self-contained `<svg>…</svg>` string the {@link import("./PairingQR").PairingQR}
 * component drops straight into the DOM via `innerHTML`.
 *
 * Kept separate from the component so it stays testable without a DOM
 * and so a non-Solid caller (a future CLI preview, a test) can reuse it.
 */

import QRCode from "qrcode";

/** Options for {@link ticketToSvg}. All optional with sane defaults. */
export interface TicketQrOptions {
  /**
   * Quiet-zone margin in modules around the symbol. `qrcode` defaults
   * to 4; we keep that so the code stays scannable when rendered small.
   */
  margin?: number;
  /**
   * Error-correction level. `"M"` (~15% recovery) is the `qrcode`
   * default and a good balance for a screen-displayed pairing ticket
   * (no print smudging to recover from, but tolerant of a glare/blur).
   */
  errorCorrectionLevel?: "L" | "M" | "Q" | "H";
}

/**
 * Render `ticket` to a standalone SVG string (no `<?xml?>` prolog, just
 * the `<svg>` element). The SVG carries no fixed `width`/`height` — it
 * keeps the intrinsic `viewBox` so the consuming component can size it
 * with CSS. Rejects when `ticket` is empty or `qrcode` fails to encode.
 */
export function ticketToSvg(ticket: string, opts: TicketQrOptions = {}): Promise<string> {
  if (ticket.length === 0) {
    return Promise.reject(new Error("cannot render an empty pairing ticket as a QR code"));
  }
  return QRCode.toString(ticket, {
    type: "svg",
    margin: opts.margin ?? 4,
    errorCorrectionLevel: opts.errorCorrectionLevel ?? "M",
  });
}
