/**
 * Banner rendered above the outline when the page's `.md` has
 * non-fatal parser recoveries. Every client (mobile, desktop)
 * wraps this — the chrome is intentionally neutral so the same
 * markup looks at home on both shells.
 *
 * Contract:
 * - `warnings` mirrors `outl_md::ParseWarning[]` (line, raw, kind).
 *   Empty array → the component renders nothing (zero layout cost).
 * - `onDismiss` is optional. When omitted, the banner stays sticky
 *   (UX: it should clear naturally when the user fixes the file).
 * - `onWarningClick(index)` is optional. Clients can use it to
 *   scroll to / open the raw view at the corresponding line.
 *
 * Mirrors the TUI's `view/warnings_banner.rs` banner. Source of
 * truth for the user-facing copy lives in `docs/clients.md`
 * § "Surfacing parser warnings on every client".
 */

import { For, Show } from "solid-js";

import type { ParseWarning } from "../api/types";

interface ParseWarningsBannerProps {
  warnings: ParseWarning[];
  onDismiss?: () => void;
  onWarningClick?: (index: number) => void;
}

const MAX_PREVIEW_CHARS = 60;

function truncate(raw: string): string {
  if (raw.length <= MAX_PREVIEW_CHARS) {
    return raw;
  }
  return `${raw.slice(0, MAX_PREVIEW_CHARS)}…`;
}

export function ParseWarningsBanner(props: ParseWarningsBannerProps) {
  return (
    <Show when={props.warnings.length > 0}>
      <div
        role="alert"
        aria-label="Parser recovery warnings"
        class="outl-parse-warnings-banner"
        data-warning-count={props.warnings.length}
      >
        <div class="outl-parse-warnings-banner__header">
          <span class="outl-parse-warnings-banner__icon" aria-hidden="true">
            ⚠
          </span>
          <span class="outl-parse-warnings-banner__title">
            {props.warnings.length} line(s) outside outl dialect — preserved as blocks
          </span>
          <Show when={props.onDismiss}>
            <button
              type="button"
              class="outl-parse-warnings-banner__dismiss"
              aria-label="Dismiss banner"
              onClick={() => props.onDismiss?.()}
            >
              ×
            </button>
          </Show>
        </div>
        <ul class="outl-parse-warnings-banner__list">
          <For each={props.warnings}>
            {(w, idx) => (
              <li class="outl-parse-warnings-banner__item">
                <button
                  type="button"
                  class="outl-parse-warnings-banner__row"
                  // `idx()` is Solid's reactive accessor — evaluated at
                  // click time, so it always returns the position the
                  // item currently occupies in `props.warnings`. Caller
                  // contract: do NOT mutate `props.warnings` from
                  // inside `onWarningClick` itself (a reentrant mutation
                  // would invalidate the index before the host handler
                  // could act on it).
                  onClick={() => props.onWarningClick?.(idx())}
                  disabled={!props.onWarningClick}
                >
                  <span class="outl-parse-warnings-banner__line">line {w.line}</span>
                  <span class="outl-parse-warnings-banner__raw">{truncate(w.raw)}</span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Show>
  );
}
