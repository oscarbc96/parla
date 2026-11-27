import { describe, expect, test } from "bun:test";
import { checkCommitMessage } from "./commit-msg";

describe("commit-msg", () => {
  test("accepts Conventional Commit subjects, with scope and breaking mark", () => {
    for (const m of [
      "feat(site): rebuild the landing page",
      "fix: crash on empty channel\n\nbody",
      "chore(spec)!: drop the old layout",
      "# comment first\nbuild: pin tools",
    ]) {
      expect(checkCommitMessage(m), m).toBeNull();
    }
  });
  test("accepts messages git writes itself", () => {
    for (const m of ["Merge branch 'x'", 'Revert "feat: a"', "fixup! feat: a", "squash! fix: b"]) {
      expect(checkCommitMessage(m), m).toBeNull();
    }
  });
  test("rejects anything else with the same rule as the PR title", () => {
    for (const m of ["", "update stuff", "Feat: x", "feat:no space", "wip(site): x", "feat(Site): x"]) {
      expect(checkCommitMessage(m), m).not.toBeNull();
    }
  });
});
