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

/**
 * Image file extensions the renderer treats as an inline `<img>`.
 *
 * Mirror of `IMAGE_EXTENSIONS` in `crates/outl-md/src/wikilink.rs` (the
 * Rust source of truth for "is this asset an image"). Keep the two in
 * sync — a divergence means a `.png` renders as a file chip on one
 * surface and an image on another.
 */
const IMAGE_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "svg",
  "bmp",
  "avif",
  "ico",
  "tiff",
  "tif",
];

/**
 * Whether an `![alt](href)` target is an inline-renderable image (by
 * extension). Everything else — pdf, unknown types — renders as a
 * click-to-open file chip, so the renderer only needs this boolean.
 *
 * Extension-only (no network probe): a query string / fragment is
 * stripped first, then the last `.`-segment is matched against
 * `IMAGE_EXTENSIONS`.
 */
export function isImagePath(href: string): boolean {
  const clean = href.split(/[?#]/)[0];
  const dot = clean.lastIndexOf(".");
  const ext = dot >= 0 ? clean.slice(dot + 1).toLowerCase() : "";
  return IMAGE_EXTENSIONS.includes(ext);
}

/**
 * Best-effort file name from a link target, for the file-chip label.
 * Strips a query/fragment and returns the last path segment.
 */
export function assetFileName(href: string): string {
  const clean = href.split(/[?#]/)[0];
  const segments = clean.split("/");
  return segments[segments.length - 1] || href;
}
