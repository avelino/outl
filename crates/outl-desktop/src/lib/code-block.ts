/**
 * Detect a fenced code block in a block's raw text.
 *
 * Mirrors the Rust-side `outl_exec::extract_fence` contract loosely
 * enough for UI purposes: we only need to know "is this a code block
 * I can render and run?" and which language. The backend re-detects
 * on `run_code_block`, so a false-positive here is harmless — the
 * backend bounces it.
 *
 * Lives in the desktop client (not `@outl/shared`) because mobile
 * doesn't surface code execution. Promote on the day a second client
 * does.
 */
export interface FenceParts {
  /** Info string after the opener backticks (`python`, `lisp`, …). */
  language: string;
  /** Inner source between opener and closer, with no trailing `\n`. */
  body: string;
}

export function detectFence(text: string): FenceParts | null {
  const m = text.match(/^```([A-Za-z0-9_+\-]*)\n([\s\S]*?)\n```\s*$/);
  if (!m) return null;
  return {
    language: m[1] || "text",
    body: m[2],
  };
}
