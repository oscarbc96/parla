import { describe, expect, test } from "bun:test";
import { parseRefs, qualifying } from "./push-refs";

const A = "a".repeat(40);
const B = "b".repeat(40);
const Z = "0".repeat(40);
const line = (l: string, sha: string, r: string, rsha = Z) => `${l} ${sha} ${r} ${rsha}`;

describe("push refs", () => {
  test("parses git's pre-push lines", () => {
    expect(parseRefs(`${line("refs/heads/x", A, "refs/heads/x")}\n\n`)).toEqual([
      { localSha: A, remoteRef: "refs/heads/x" },
    ]);
  });
  test("keeps branches that share history with main", () => {
    const refs = parseRefs(line("HEAD", A, "refs/heads/feat/x"));
    expect(qualifying(refs, () => true)).toEqual([A]);
  });
  test("skips an unrelated branch", () => {
    const refs = parseRefs(line("HEAD", A, "refs/heads/unrelated") + "\n" + line("HEAD", B, "refs/heads/feat/y"));
    expect(qualifying(refs, (s) => s === B)).toEqual([B]);
  });
  test("skips tags and deletions", () => {
    const refs = parseRefs(line("refs/tags/v1", A, "refs/tags/v1") + "\n" + line("(delete)", Z, "refs/heads/old"));
    expect(qualifying(refs, () => true)).toEqual([]);
  });
});
