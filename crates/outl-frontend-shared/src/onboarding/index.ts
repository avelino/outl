/**
 * First-run onboarding copy — the one piece both clients render
 * identically.
 *
 * The *chrome* of onboarding is client-specific (mobile shows a
 * full-screen sequence of bottom-sheet-styled cards with haptics;
 * desktop wraps the existing `<WorkspacePicker />` + a centered card),
 * so the components stay in each client. What does NOT differ is the
 * **words** — the honest, no-account explanation of where notes live
 * and how peer-to-peer sync works. Keeping the strings here means a
 * copy edit (or a future i18n pass) lands in one place instead of
 * drifting between iOS and desktop.
 *
 * Nothing here calls `invoke()` or touches Tauri — it is plain data.
 * The client decides *how* to present it.
 */

/**
 * Headline + body for the "where do your notes live?" step.
 *
 * Mobile renders the two storage choices (`local` / `iCloud`) below
 * this copy; desktop renders the folder picker. The copy is the same
 * framing either way: your notes are files, you choose where.
 */
export const STORAGE_STEP = {
  /** Short title shown at the top of the storage step. */
  title: "Where do your notes live?",
  /**
   * One honest sentence. outl notes are plain Markdown files in a
   * folder you control — no proprietary database, no lock-in.
   */
  body: "Your notes are plain Markdown files in a folder you control. No account, no proprietary database.",
} as const;

/**
 * Headline + body + bullet points for the optional "sync with another
 * device?" step.
 *
 * The promise we must keep honest: sync is peer-to-peer, needs no
 * account and no cloud, and a single device works fine. This is the
 * step the user can always skip.
 */
export const SYNC_STEP = {
  /** Short title shown at the top of the sync step. */
  title: "Sync with another device?",
  /**
   * The core reassurance: P2P, no account, no server, optional.
   */
  body: "outl syncs directly between your devices, peer-to-peer. No account, no cloud server, nothing to sign up for.",
  /**
   * Short reassurances rendered as a bulleted list under {@link body}.
   * Each is a complete, self-contained claim we can stand behind.
   */
  bullets: [
    "One device works perfectly on its own.",
    "Pairing is end-to-end between your devices only.",
    "You can pair a device any time later from settings.",
  ],
  /** Label for the button that opens the existing pairing flow. */
  pairCta: "Pair a device",
  /** Label for the button that skips pairing and finishes onboarding. */
  skipCta: "Skip for now",
} as const;

/** Label for the final "open my notes" button that lands on today's journal. */
export const FINISH_CTA = "Start writing";
