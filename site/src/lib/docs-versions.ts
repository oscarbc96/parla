/**
 * Docs versions: pure, no Astro imports, so the Astro config, the content
 * config, `src/lib`, scripts and tests can all import it.
 *
 * `content/docs` is the working docs ("next"). `content/versions.json` lists
 * the frozen release lines (`X.Y`, newest first), each snapshotted in
 * `content/versions/<X.Y>/`. With no lines, `/docs/` is the working docs; with
 * lines, `/docs/` is the newest line, `/docs/next/` the working docs and
 * `/docs/v<X.Y>/` each older line.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/** Env var that points (relative to `site/`) at a fixture dir for tests. */
export const FIXTURE_ENV = "PARLA_DOCS_VERSIONS_FIXTURE";

/** Working docs, relative to `site/`. */
export const WORKING_DIR = "content/docs";
/** Where snapshots live, relative to `site/`. */
export const SNAPSHOT_DIR = "content/versions";
/** The version list, relative to `site/`. */
export const VERSIONS_FILE = "content/versions.json";

const ID = /^\d+\.\d+$/;

export interface DocsVersion {
  /** `"next"` or `"X.Y"`. */
  id: string;
  /** URL segment after `/docs/`; `null` = served at `/docs`. */
  segment: string | null;
  /** Fumadocs collection name. */
  collection: string;
  /** Content directory, relative to `site/`. */
  dir: string;
  /** Served at `/docs` as the newest release (or the only docs). */
  latest: boolean;
  /** Working docs next to released lines. */
  unreleased: boolean;
}

function parts(id: string): [number, number] {
  const [major, minor] = id.split(".").map(Number);
  return [major!, minor!];
}

/** Collection name of a snapshot: `0.10` is `v0_10`. */
function snapshotCollection(id: string): string {
  return `v${id.replace(".", "_")}`;
}

/** Name of the Astro content collection that holds a version's meta files. */
export function metaCollectionName(version: DocsVersion): string {
  return version.collection === "docs" ? "meta" : `${version.collection}_meta`;
}

/**
 * Validates the release lines and resolves every docs version, working docs first.
 * `ids` are the release lines, newest first; `fixtureDir` (relative to `site/`)
 * replaces the snapshot root.
 */
export function resolveVersions(
  ids: string[],
  fixtureDir?: string,
): DocsVersion[] {
  const seen = new Set<string>();
  ids.forEach((id, i) => {
    if (typeof id !== "string" || !ID.test(id)) {
      throw new Error(
        `invalid docs version ${JSON.stringify(id)}: expected "X.Y" (for example "0.2")`,
      );
    }
    if (seen.has(id)) throw new Error(`duplicate docs version "${id}"`);
    seen.add(id);
    if (i > 0) {
      const [pa, pb] = parts(ids[i - 1]!);
      const [a, b] = parts(id);
      if (a > pa || (a === pa && b >= pb)) {
        throw new Error(
          `docs versions must be sorted newest first: "${id}" comes after "${ids[i - 1]}"`,
        );
      }
    }
  });

  const working: DocsVersion = {
    id: "next",
    segment: ids.length > 0 ? "next" : null,
    collection: "docs",
    dir: WORKING_DIR,
    latest: ids.length === 0,
    unreleased: ids.length > 0,
  };
  const root = fixtureDir ? fixtureDir.replace(/\/+$/, "") : SNAPSHOT_DIR;
  return [
    working,
    ...ids.map((id, i) => ({
      id,
      segment: i === 0 ? null : `v${id}`,
      collection: snapshotCollection(id),
      dir: `${root}/${id}`,
      latest: i === 0,
      unreleased: false,
    })),
  ];
}

/**
 * Reads the version list (from the fixture dir when the env var is set) and
 * resolves it. `root` is the `site/` directory.
 */
export function loadVersions(
  root: string = process.cwd(),
  env: Record<string, string | undefined> = process.env,
): DocsVersion[] {
  const fixture = env[FIXTURE_ENV] || undefined;
  const file = fixture
    ? join(root, fixture, "versions.json")
    : join(root, VERSIONS_FILE);
  let ids: string[] = [];
  if (existsSync(file)) {
    let data: unknown;
    try {
      data = JSON.parse(readFileSync(file, "utf8"));
    } catch (error) {
      throw new Error(`${file}: invalid JSON (${(error as Error).message})`);
    }
    const list = Array.isArray(data)
      ? data
      : (data as { versions?: unknown } | null)?.versions;
    if (!Array.isArray(list)) {
      throw new Error(`${file}: expected { "versions": ["X.Y", ...] }`);
    }
    ids = list;
  }
  return resolveVersions(ids, fixture);
}
