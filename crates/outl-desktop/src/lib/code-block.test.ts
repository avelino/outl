import { describe, expect, it } from "vitest";

import { detectFence } from "./code-block";

describe("detectFence", () => {
  it("extracts language and body from a clean fence", () => {
    const out = detectFence("```python\nprint('hi')\n```");
    expect(out).toEqual({ language: "python", body: "print('hi')" });
  });

  it("treats a missing info string as text", () => {
    const out = detectFence("```\nplain\n```");
    expect(out).toEqual({ language: "text", body: "plain" });
  });

  it("preserves multi-line bodies verbatim", () => {
    const src = "```js\nconst x = 1;\nconst y = 2;\n```";
    expect(detectFence(src)).toEqual({
      language: "js",
      body: "const x = 1;\nconst y = 2;",
    });
  });

  it("tolerates trailing whitespace after the closer", () => {
    const out = detectFence("```lisp\n(+ 1 2)\n```   ");
    expect(out).toEqual({ language: "lisp", body: "(+ 1 2)" });
  });

  it("returns null when the block has no closer", () => {
    expect(detectFence("```python\nprint('hi')")).toBeNull();
  });

  it("returns null when the opener is indented (not a top-level fence)", () => {
    expect(detectFence("  ```python\nprint('hi')\n```")).toBeNull();
  });

  it("returns null for non-fenced text", () => {
    expect(detectFence("just a plain block")).toBeNull();
  });

  it("returns null for the empty string", () => {
    expect(detectFence("")).toBeNull();
  });

  it("accepts info strings with dashes, plus and underscore", () => {
    expect(detectFence("```c++\nint main(){}\n```")?.language).toBe("c++");
    expect(detectFence("```scheme-r7rs\n(display 1)\n```")?.language).toBe(
      "scheme-r7rs",
    );
    expect(detectFence("```snake_case\nx=1\n```")?.language).toBe("snake_case");
  });
});
