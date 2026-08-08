/**
 * Banner rendered above the outline when a page has stopped syncing
 * because its `.md` holds content the op log never recorded.
 *
 * Sits in the same slot as `<ParseWarningsBanner />` and shares its
 * neutral chrome, but is deliberately louder (red, not amber): a parser
 * warning says "outl kept something odd", this one says "this page is
 * no longer converging with your other devices, and here's what to do".
 *
 * Contract:
 * - `info` mirrors `PageView.md_ahead_of_log`. Pass `undefined` (the
 *   healthy case) and the component renders nothing.
 * - `client` picks the recovery sentence only — see `aheadOfLogNotice`,
 *   which owns every word shown here.
 * - Not dismissable on purpose. The condition does not clear on its own
 *   and the banner disappears the moment a reconcile fixes it; a dismiss
 *   button would let a user hide a page that is silently not syncing,
 *   which is the exact failure this replaces.
 */

import { Show } from "solid-js";

import type { MdAheadOfLog } from "../api/types";
import { aheadOfLogNotice, type AheadOfLogClient } from "./ahead-of-log";

interface PageAheadOfLogBannerProps {
  info?: MdAheadOfLog;
  client: AheadOfLogClient;
}

export function PageAheadOfLogBanner(props: PageAheadOfLogBannerProps) {
  return (
    <Show when={props.info}>
      {(info) => {
        const notice = () => aheadOfLogNotice(info(), props.client);
        return (
          <div role="alert" aria-label="Page not syncing" class="outl-ahead-of-log-banner">
            <div class="outl-ahead-of-log-banner__header">
              <span class="outl-ahead-of-log-banner__icon" aria-hidden="true">
                ⚠
              </span>
              <span class="outl-ahead-of-log-banner__title">{notice().title}</span>
            </div>
            <p class="outl-ahead-of-log-banner__body">{notice().body}</p>
            <p class="outl-ahead-of-log-banner__sample">e.g. {notice().sample}</p>
            <p class="outl-ahead-of-log-banner__action">{notice().action}</p>
            <code class="outl-ahead-of-log-banner__command">{notice().command}</code>
            <p class="outl-ahead-of-log-banner__caution">{notice().caution}</p>
            <p class="outl-ahead-of-log-banner__path" title={notice().path}>
              {notice().path}
            </p>
          </div>
        );
      }}
    </Show>
  );
}
