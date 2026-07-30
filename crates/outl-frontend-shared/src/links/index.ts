/**
 * Link classification shared by every client's link-click handler.
 *
 * Pure logic, no `invoke` — mirrors `outl_md::is_asset_link` so the
 * frontend routes a clicked `[label](url)` the same way the backend
 * would: a workspace asset (`assets/…`) opens via the `open_asset`
 * command (OS default app), everything else (`http(s)`, `mailto`) goes
 * through `openExternalUrl`.
 */

/**
 * True when a markdown link URL points at a workspace asset rather than
 * an external resource.
 *
 * Mirror of `outl_md::asset::is_asset_link`: matches the canonical
 * `assets/…` form plus the `./assets/…` and `/assets/…` variants a
 * hand-typed or imported link might carry. Any URL with a scheme
 * (`http://`, `mailto:`) never matches — the `://` guard also rejects an
 * `http://assets/…` lookalike. Keep this in sync with the Rust owner.
 */
export function isAssetLink(url: string): boolean {
  if (url.includes("://")) {
    return false;
  }
  let trimmed = url.startsWith("./") ? url.slice(2) : url;
  trimmed = trimmed.startsWith("/") ? trimmed.slice(1) : trimmed;
  return trimmed.startsWith("assets/");
}
