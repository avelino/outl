/**
 * Pure derivations over peer reachability, shared by every GUI client so
 * the sync indicator reads identically on mobile and desktop.
 */

import type { PeerStatusDto } from "../api/types";

/**
 * `true` when at least one paired peer is currently reachable over iroh.
 *
 * This is the single source of truth for "is the P2P mesh up right now?"
 * that both clients feed into their sync dot: a green/synced dot means
 * `peersOnline(statuses) === true`. When it's `false` there is nothing to
 * sync with (no peers paired, or every paired peer is unreachable), so the
 * dot reads offline regardless of the device's own internet connection —
 * `navigator.onLine` says the phone has WiFi, not that a peer answered.
 *
 * Accepts either the raw `peerStatus()` array or the `Map<node_id, …>`
 * the desktop's `SyncPanel` already builds, so neither client has to
 * reshape its data to call it.
 */
export function peersOnline(
  statuses: readonly PeerStatusDto[] | ReadonlyMap<string, PeerStatusDto>,
): boolean {
  const list = statuses instanceof Map ? [...statuses.values()] : (statuses as readonly PeerStatusDto[]);
  return list.some((s) => s.online === true);
}
