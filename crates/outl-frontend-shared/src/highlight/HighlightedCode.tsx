import { JSX, Show, createMemo, createResource } from "solid-js";

import { canonical } from "./aliases";

/**
 * Render a fenced code block with syntax highlighting via
 * `highlight.js`. The grammar set is loaded lazily — the full
 * `highlight.js/lib/common` bundle (~30 popular languages) is
 * imported on the first render and cached for the rest of the
 * session.
 *
 * Unknown / empty languages fall back to a plain `<pre><code>`
 * with the brand background. We never highlight as "auto" — the
 * misclassification cost (Bash highlighted as Perl) is worse than
 * the visual flatness of plaintext.
 *
 * The host client supplies the visual theme by importing
 * `@outl/shared/highlight/styles` once in its global stylesheet
 * — see `styles.css` next to this file for what tokens look like
 * (it follows highlight.js's `.hljs-*` selectors).
 */
interface HighlightedCodeProps {
  /** Language tag as it appears after the opening backticks. May be
   *  an alias (`rs`, `javascript`); the renderer canonicalizes it. */
  language: string | null | undefined;
  /** The code body (no fence lines). */
  code: string;
  /** Optional extra classes on the outer `<pre>` (e.g. for layout). */
  class?: string;
}

interface HljsLike {
  highlight(code: string, opts: { language: string }): { value: string };
  getLanguage(name: string): unknown;
}

let cachedHljs: HljsLike | null = null;

async function loadHljs(): Promise<HljsLike> {
  if (cachedHljs) return cachedHljs;
  // `common` is the ~30-language bundle highlight.js ships for
  // exactly this use case — JS / TS / Python / Rust / Go / Bash /
  // YAML / JSON / Markdown / etc. Cheaper to ship than the full
  // grammar set (~500KB → ~80KB), still covers what users actually
  // write in code fences.
  const mod = (await import("highlight.js/lib/common")) as {
    default: HljsLike;
  };
  cachedHljs = mod.default;
  return cachedHljs;
}

export function HighlightedCode(props: HighlightedCodeProps): JSX.Element {
  // Resolve the alias once per render. Memoized so children that
  // observe both `language` and `code` re-render together.
  //
  // A callable-template fence (`call:<name>`) has no grammar of its
  // own, but its body is `key: value` params — highlight it as YAML so
  // the keys/values get colour instead of rendering flat. The display
  // chip still shows the raw `call:<name>` (CodeFenceView uses the raw
  // language, not this).
  const resolvedLang = createMemo(() => {
    const lang = props.language;
    if (lang && lang.toLowerCase().startsWith("call:")) return "yaml";
    return canonical(lang);
  });

  // Trigger the lazy `highlight.js` import. Solid's `createResource`
  // does the suspending; while the bundle is loading we show plain
  // text so the user never sees a blank box.
  const [hljs] = createResource(() => loadHljs());

  const rendered = createMemo<string | null>(() => {
    const lib = hljs();
    const lang = resolvedLang();
    if (!lib || !lang) return null;
    if (!lib.getLanguage(lang)) return null;
    try {
      return lib.highlight(props.code, { language: lang }).value;
    } catch {
      // Grammar mismatch / `highlight.js` internal — fall back to
      // plain text rather than throwing past the renderer.
      return null;
    }
  });

  return (
    <pre
      class={`outl-codeblock ${props.class ?? ""}`}
      data-lang={resolvedLang() ?? "plain"}
    >
      <Show
        when={rendered()}
        fallback={<code class="hljs language-plain">{props.code}</code>}
      >
        <code
          class={`hljs language-${resolvedLang()}`}
          // `rendered()` is highlight.js output, which is HTML —
          // pre-escaped by the library, safe to drop into innerHTML.
          innerHTML={rendered() ?? ""}
        />
      </Show>
    </pre>
  );
}
