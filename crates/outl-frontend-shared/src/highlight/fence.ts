/**
 * Detect a fenced code block in a block's raw text.
 *
 * Loose mirror of `outl_md::lang::extract_fence`-style logic in the
 * frontend: we only need "is this a code block I can render and
 * (maybe) run?" plus its language tag. The backend re-detects on
 * `run_code_block`, so a false-positive here is harmless — the
 * backend bounces it.
 *
 * Lives in `@outl/shared/highlight` so both clients consume the
 * same detector before handing the result to `<HighlightedCode />`.
 */
export interface FenceParts {
  /** Info string after the opener backticks (`python`, `lisp`, …). */
  language: string;
  /** Inner source between opener and closer, with no trailing `\n`. */
  body: string;
}

// `:` is allowed in the info string so a callable-template fence
// (` ```call:<name> `) is detected as a code block, not left as raw
// text. The backend's `extract_fence` takes the same first-token info
// string, so the two stay consistent.
const FENCE_RE = /^```([A-Za-z0-9_+\-#:]*)\n([\s\S]*?)\n```\s*$/;

export function detectFence(text: string): FenceParts | null {
  const m = text.match(FENCE_RE);
  if (!m) return null;
  return {
    language: m[1] || "text",
    body: m[2],
  };
}
