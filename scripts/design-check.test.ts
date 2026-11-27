import { describe, expect, test } from "bun:test";
import {
  addedLines,
  checkDiff,
  checkLine,
  inScope,
  loadScale,
} from "./design-check";

const scale = loadScale();
const diffOf = (file: string, ...lines: string[]) =>
  `diff --git a/${file} b/${file}\n--- a/${file}\n+++ b/${file}\n@@ -1,0 +10,${lines.length} @@\n${lines.map((l) => `+${l}`).join("\n")}\n`;
const rules = (l: string, prev = "") =>
  checkLine(l, prev, scale).map((r) => r.rule);

describe("design-check", () => {
  test("scale comes from the token file and includes layout", () => {
    expect(scale.get(8)).toContain("SPACE_4");
    expect(scale.get(264)).toEqual(["LAYOUT_SIDEBAR"]);
  });
  test("colour literals are errors", () => {
    for (const l of [
      "let c = rgb(0xff0000);",
      "let c = rgba(0xff000080);",
      "let c = hsla(0.5, 1., 0.5, 1.);",
      "let c = rgb_to_hsla(rgb(0x123456));",
      "let c = Hsla { h: 0., s: 0., l: 0., a: 1. };",
      'let c = "#FFA611";',
      "let c = gpui::white();",
      "let c = white();",
      "let c = gpui::black();",
      "let c = gpui::rgb(v);",
      "let c = gpui::rgba(v);",
      "let c = gpui::hsla(a, b, c, d);",
    ]) {
      expect(rules(l), l).toContain("colour");
    }
  });
  test("theme colours pass", () => {
    expect(rules("let c = theme.colors().text;")).toEqual([]);
    expect(rules("let c = hsla(h, s, l, a);")).toEqual([]);
    expect(rules("let c = hsla_to_rgba(x); let d = Rgba::from_rgba(y);")).toEqual(
      [],
    );
    expect(rules("let c = is_white(x);")).toEqual([]);
  });
  test("px in the scale, 0 and 1 pass; others fail with a hint", () => {
    expect(rules(".h(px(8.0)).p(px(0.)).border(px(1.0)).t(px(-1.0))")).toEqual(
      [],
    );
    const r = checkLine(".h(px(29.0))", "", scale);
    expect(r).toHaveLength(1);
    expect(r[0].message).toContain("px(29.0)");
    expect(r[0].message).toContain("themes::metrics::");
  });
  test("a bare literal equal to a layout value must use the const", () => {
    const r = checkLine(".w(px(264.))", "", scale);
    expect(r).toHaveLength(1);
    expect(r[0].message).toContain("LAYOUT_SIDEBAR");
    expect(rules(".w(px(-264.))")).toEqual(["px"]);
    expect(rules(".w(themes::metrics::LAYOUT_SIDEBAR)")).toEqual([]);
  });
  test("hint maps a near value to its const", () => {
    expect(checkLine(".h(px(9.0))", "", scale)[0].message).toContain("SPACE_4");
  });
  test("escape hatch needs a reason, on the line or the one above", () => {
    expect(rules(".h(px(29.0)) // design: allow icon grid")).toEqual([]);
    expect(rules(".h(px(29.0))", "// design: allow icon grid")).toEqual([]);
    expect(rules(".h(px(29.0)) // design: allow")).toEqual(["px"]);
  });
  test("only added lines in components and parla src count", () => {
    expect(
      checkDiff(diffOf("crates/components/src/a.rs", ".h(px(29.0))"), scale),
    ).toHaveLength(1);
    expect(
      checkDiff(diffOf("crates/parla/src/a.rs", "rgb(0x1)"), scale)[0].line,
    ).toBe(10);
    expect(
      checkDiff(diffOf("crates/components/tests/a.rs", ".h(px(29.0))"), scale),
    ).toEqual([]);
    expect(
      checkDiff(diffOf("crates/store/src/a.rs", ".h(px(29.0))"), scale),
    ).toEqual([]);
    expect(inScope("crates/parla/src/x/y.rs")).toBe(true);
  });
  test("line numbers and previous line follow the hunk", () => {
    const a = addedLines(
      diffOf("crates/parla/src/a.rs", "// design: allow x", ".h(px(29.0))"),
    );
    expect(a[1]).toMatchObject({ line: 11, prev: "// design: allow x" });
    expect(
      checkDiff(
        diffOf("crates/parla/src/a.rs", "// design: allow x", ".h(px(29.0))"),
        scale,
      ),
    ).toEqual([]);
  });
});
