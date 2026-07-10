import { describe, expect, it } from "vitest";

import { detectFence } from "./fence";

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

  it("accepts the `c#` info-string (special char in alias)", () => {
    expect(detectFence("```c#\nprint();\n```")).toEqual({
      language: "c#",
      body: "print();",
    });
  });

  it("detects a callable-template fence (`call:<name>`) as a code block", () => {
    expect(detectFence("```call:calc-salary\npedido: 10\nproposta: 19\n```")).toEqual({
      language: "call:calc-salary",
      body: "pedido: 10\nproposta: 19",
    });
  });
});
