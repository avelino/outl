/**
 * `@outl/shared/toolbar` — catalog + MFU ordering for the mobile edit
 * toolbar, shared across every web-rendered client bar (Android today,
 * iOS once the native bar retires). The rendering (icons, capsule,
 * keyboard docking) stays in client chrome; only the pure logic lives
 * here.
 */
export {
  DEFAULT_ORDER,
  PINNED_FIRST,
  PINNED_LAST,
  TOOLBAR_META,
  type ToolbarAction,
  type ToolbarActionMeta,
  type ToolbarStyle,
} from "./actions";
export {
  MFU_STORAGE_KEY,
  orderedMiddleActions,
  orderedMiddleFromStore,
  parseCounts,
  readCountsFromStore,
  record,
  recordToStore,
  type ToolbarCounts,
} from "./mfu";
