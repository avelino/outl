import { describe, expect, it } from "vitest";

import { propertyChips, remindRule } from "./properties";

describe("propertyChips", () => {
  it("gives a known key its glyph and hides the key name", () => {
    const [chip] = propertyChips([["remind", "3pm every 1h"]]);
    expect(chip.icon).toBe("⏰");
    expect(chip.known).toBe(true);
    expect(chip.value).toBe("3pm every 1h");
  });

  it("leaves a user's own property as key + value", () => {
    // `priority:: high` is theirs, not ours to interpret.
    const [chip] = propertyChips([["priority", "high"]]);
    expect(chip.icon).toBeUndefined();
    expect(chip.known).toBe(false);
    expect(chip.key).toBe("priority");
  });

  it("matches the key case-insensitively", () => {
    expect(propertyChips([["Remind", "10am"]])[0].icon).toBe("⏰");
  });

  it("hides outl's own bookkeeping", () => {
    // The user never typed these and can't act on them.
    expect(propertyChips([["id", "01ABC"], ["from-template", "daily"]])).toEqual(
      [],
    );
  });

  it("keeps the backend's order so two clients agree", () => {
    const chips = propertyChips([
      ["auto-run", "true"],
      ["priority", "high"],
      ["remind", "9am"],
    ]);
    expect(chips.map((c) => c.key)).toEqual(["auto-run", "priority", "remind"]);
  });

  it("is empty for a block with no properties", () => {
    expect(propertyChips([])).toEqual([]);
    expect(propertyChips(undefined)).toEqual([]);
  });
});

describe("remindRule", () => {
  it("finds the rule whatever the key casing", () => {
    expect(remindRule([["REMIND", "3pm"]])).toBe("3pm");
  });

  it("is null when the block has no reminder", () => {
    expect(remindRule([["priority", "high"]])).toBeNull();
    expect(remindRule(undefined)).toBeNull();
  });
});
