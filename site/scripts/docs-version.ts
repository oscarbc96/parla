/**
 * Freezes the working docs as a release line.
 *
 *   mise run site:docs-version -- <X.Y> [--ref <git-ref>]
 *
 * Copies `content/docs` (or, with `--ref`, the docs of that git ref) into
 * `content/versions/<X.Y>/`, replacing an existing snapshot of the same line
 * (how a patch release refreshes it), and inserts the line into
 * `content/versions.json`, newest first. Run it after a Release PR for a new
 * or patched line merges, for stable releases only.
 */
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { run } from "../../scripts/lib";
import {
  resolveVersions,
  SNAPSHOT_DIR,
  VERSIONS_FILE,
  WORKING_DIR,
} from "../src/lib/docs-versions";

const ID = /^\d+\.\d+$/;
const DOCS_FILE = /\.mdx$|^meta(\.[A-Za-z-]+)?\.json$/;

/** Inserts `id` into `ids` (newest first) unless it is already listed. */
export function insertVersion(ids: string[], id: string): string[] {
  if (!ID.test(id)) {
    throw new Error(`invalid docs version "${id}": expected "X.Y"`);
  }
  if (ids.includes(id)) return [...ids];
  const key = (v: string) => v.split(".").map(Number) as [number, number];
  const [a, b] = key(id);
  const out = [...ids];
  const at = out.findIndex((other) => {
    const [oa, ob] = key(other);
    return oa < a || (oa === a && ob < b);
  });
  out.splice(at === -1 ? out.length : at, 0, id);
  return out;
}

/** Reads `content/versions.json` of the site at `root` (empty when absent). */
export function readVersionList(root: string): string[] {
  const file = join(root, VERSIONS_FILE);
  if (!existsSync(file)) return [];
  const data = JSON.parse(readFileSync(file, "utf8"));
  return Array.isArray(data) ? data : (data.versions ?? []);
}

/**
 * Snapshots the docs found in `docsDir` as line `id` of the site at `root`
 * and records the line. Returns the updated list.
 */
export function createSnapshot(
  root: string,
  id: string,
  docsDir: string = join(root, WORKING_DIR),
): string[] {
  if (!ID.test(id)) {
    throw new Error(`invalid docs version "${id}": expected "X.Y"`);
  }
  if (!existsSync(docsDir)) throw new Error(`${docsDir}: no docs to snapshot`);
  const ids = insertVersion(readVersionList(root), id);
  // Throws on a malformed or misordered list before anything is written.
  resolveVersions(ids);
  const target = join(root, SNAPSHOT_DIR, id);
  rmSync(target, { recursive: true, force: true });
  mkdirSync(target, { recursive: true });
  cpSync(docsDir, target, {
    recursive: true,
    filter: (src) =>
      src === docsDir || !existsSync(src) || DOCS_FILE.test(basename(src)),
  });
  writeFileSync(
    join(root, VERSIONS_FILE),
    `${JSON.stringify({ versions: ids }, null, 2)}\n`,
    "utf8",
  );
  return ids;
}

/**
 * Extracts `site/content/docs` of a git ref into a temp dir and returns the
 * path of the extracted docs. `root` is the site directory inside a checkout.
 */
export function docsFromRef(root: string, ref: string): string {
  const top = run(["git", "rev-parse", "--show-toplevel"], {
    cwd: root,
  }).trim();
  const dir = mkdtempSync(join(tmpdir(), "parla-docs-ref-"));
  const archive = join(dir, "docs.tar");
  run(["git", "archive", "-o", archive, ref, `site/${WORKING_DIR}`], {
    cwd: top,
  });
  run(["tar", "-xf", archive, "-C", dir], { cwd: dir });
  return join(dir, "site", WORKING_DIR);
}

export function main(argv: string[], root: string = process.cwd()): number {
  const args = [...argv];
  let ref: string | undefined;
  const at = args.indexOf("--ref");
  if (at !== -1) {
    ref = args[at + 1];
    if (!ref) {
      console.error("--ref needs a git ref");
      return 2;
    }
    args.splice(at, 2);
  }
  const [id, ...rest] = args;
  if (!id || rest.length > 0 || !ID.test(id)) {
    console.error("usage: docs-version <X.Y> [--ref <git-ref>]");
    return 2;
  }
  try {
    const docsDir = ref ? docsFromRef(root, ref) : undefined;
    const ids = createSnapshot(root, id, docsDir);
    console.log(
      `docs ${id} frozen${ref ? ` from ${ref}` : ""}; versions: ${ids.join(", ")}`,
    );
    return 0;
  } catch (error) {
    console.error((error as Error).message);
    return 1;
  }
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
