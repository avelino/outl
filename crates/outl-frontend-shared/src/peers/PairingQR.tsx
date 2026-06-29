/**
 * `<PairingQR ticket=… />` — renders a pairing ticket as a scannable
 * QR code, shared by mobile + desktop.
 *
 * The component is **almost** pure: it takes a ticket string and owns
 * the async QR encoding (via {@link ticketToSvg}) as a local render
 * concern — it never calls a Tauri command itself. The host fetches the
 * ticket (e.g. `peerPairHost()` from `@outl/shared/api/commands`) and
 * passes it down; this component just draws it.
 *
 * Markup is intentionally minimal (a single wrapper that holds the SVG)
 * so each client styles the frame, size and surrounding chrome. Import
 * `@outl/shared/peers/styles` for the neutral baseline.
 */

import { createResource, Show, type JSX } from "solid-js";

import { ticketToSvg, type TicketQrOptions } from "./qr";

interface PairingQRProps {
  /** The pairing ticket string to encode (from `peerPairHost()`). */
  ticket: string;
  /** Forwarded to {@link ticketToSvg} (margin, error-correction). */
  options?: TicketQrOptions;
  /**
   * Rendered while the SVG is being generated. Defaults to a neutral
   * "Generating…" placeholder. Pass `null` to render nothing.
   */
  fallback?: JSX.Element;
  /**
   * Rendered if encoding fails (empty/oversized ticket). Receives the
   * error. Defaults to a neutral inline error message.
   */
  errorRender?: (err: unknown) => JSX.Element;
}

export function PairingQR(props: PairingQRProps): JSX.Element {
  const [svg] = createResource(
    // Track both inputs so a ticket or option change re-encodes.
    () => ({ ticket: props.ticket, options: props.options }),
    ({ ticket, options }) => ticketToSvg(ticket, options),
  );

  return (
    <div class="outl-pairing-qr" role="img" aria-label="Pairing QR code">
      <Show
        when={svg()}
        fallback={
          <Show when={svg.error} fallback={props.fallback ?? <DefaultFallback />}>
            {props.errorRender ? props.errorRender(svg.error) : <DefaultError />}
          </Show>
        }
      >
        {/* `qrcode` returns a trusted, self-generated SVG string (no user
            HTML), so `innerHTML` is safe here — the ticket is encoded as
            QR modules, never interpolated into the markup. */}
        {(markup) => <div class="outl-pairing-qr__svg" innerHTML={markup()} />}
      </Show>
    </div>
  );
}

function DefaultFallback(): JSX.Element {
  return <div class="outl-pairing-qr__loading">Generating QR…</div>;
}

function DefaultError(): JSX.Element {
  return <div class="outl-pairing-qr__error">Could not render QR code</div>;
}
