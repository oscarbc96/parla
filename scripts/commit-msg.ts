#!/usr/bin/env bun
/**
 * hk commit-msg step: the first line of a commit message must be a
 * Conventional Commit, with the same rules lint:pr-body applies to a PR title.
 * Messages git writes itself (merge, revert, fixup and
 * squash) pass.
 *
 *   bun scripts/commit-msg.ts <message-file>
 */
import { readFileSync } from "node:fs";
import { TITLE } from "./pr-body";

const GIT_MADE = /^(Merge |Revert "|fixup! |squash! |amend! )/;

/** Error text for a bad subject line, or null when the message is fine. */
export function checkCommitMessage(message: string): string | null {
  const first =
    message
      .split("\n")
      .find((l) => l.trim() !== "" && !l.startsWith("#"))
      ?.trim() ?? "";
  if (GIT_MADE.test(first) || TITLE.test(first)) return null;
  return (
    "commit-msg: the first line is not a Conventional Commit `type(scope): summary` " +
    "(types: feat, fix, perf, docs, refactor, test, ci, build, chore, revert)"
  );
}

if (import.meta.main) {
  const file = process.argv[2];
  if (!file) {
    console.error("usage: commit-msg.ts <message-file>");
    process.exit(2);
  }
  const err = checkCommitMessage(readFileSync(file, "utf8"));
  if (err) {
    console.error(err);
    process.exit(1);
  }
}
