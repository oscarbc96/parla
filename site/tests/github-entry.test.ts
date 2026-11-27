import { describe, expect, test } from "bun:test";
import { toEntry } from "../src/lib/github";

describe("toEntry (GitHub API shape)", () => {
  test("an unnamed release falls back to its tag", () => {
    const e = toEntry({
      tag_name: "v0.1.0",
      name: "",
      body: "notes",
      html_url: "https://example.test/r/1",
      published_at: "2026-11-01T00:00:00Z",
    });
    expect(e?.title).toBe("v0.1.0");
    expect(e?.id).toBe("v0.1.0");
    expect(e?.date.toISOString()).toBe("2026-11-01T00:00:00.000Z");
  });

  test("drafts are dropped", () => {
    expect(
      toEntry({ tag_name: "v0.2.0", name: "x", draft: true }),
    ).toBeUndefined();
  });

  test("prerelease flag is kept", () => {
    expect(
      toEntry({ tag_name: "v0.2.0-rc.1", prerelease: true })?.prerelease,
    ).toBe(true);
  });
});
