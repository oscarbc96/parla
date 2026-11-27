import { readFile } from "node:fs/promises";
import { join } from "node:path";

export interface LatestRelease {
  version: string;
  dmgUrl: string;
  sumsUrl: string;
  date: string;
}

/** One published release in the downloads history table. */
export interface ReleaseEntry {
  version: string;
  date: string;
  /** The release page on GitHub, when the payload has one. */
  url?: string;
  /** Universal macOS DMG, when the release shipped one. */
  dmgUrl?: string;
  /** SHA256SUMS asset, when present. */
  sumsUrl?: string;
}

/** A release as the GitHub API returns it, or as the changelog fixture stores it. */
interface HistoryPayload extends ReleasePayload {
  id?: string;
  title?: string;
  content?: string;
  url?: string;
  date?: string;
}

/** File name of the universal DMG in `dmgUrl`. */
export function dmgFileName(dmgUrl: string): string {
  return decodeURIComponent(dmgUrl.split("/").pop() ?? "");
}

interface ReleaseAsset {
  name?: string;
  browser_download_url?: string;
}

interface ReleasePayload {
  draft?: boolean;
  prerelease?: boolean;
  tag_name?: string;
  name?: string;
  published_at?: string;
  created_at?: string;
  html_url?: string;
  assets?: ReleaseAsset[];
}

function warn(message: string, error?: unknown) {
  const detail = error instanceof Error ? `: ${error.message}` : "";
  console.warn(`[parla-site] ${message}${detail}`);
}

function pickAsset(
  assets: ReleaseAsset[],
  predicate: (name: string) => boolean,
): string | undefined {
  for (const asset of assets) {
    const name = asset.name ?? "";
    if (predicate(name) && asset.browser_download_url) {
      return asset.browser_download_url;
    }
  }
  return undefined;
}

/**
 * Every published release, newest first, from the release data the changelog
 * reads (`PARLA_RELEASES_FILE`, then `.releases.json`). Handles one array or
 * one array per page, as `gh api --paginate --slurp` writes them. Returns an
 * empty list for a missing file or any read error: the page then omits the
 * table.
 */
export async function releaseHistory(path?: string): Promise<ReleaseEntry[]> {
  const file =
    path ??
    process.env.PARLA_RELEASES_FILE ??
    join(process.cwd(), ".releases.json");
  try {
    const raw = JSON.parse(await readFile(file, "utf8")) as unknown;
    if (!Array.isArray(raw)) return [];
    const entries: ReleaseEntry[] = [];
    for (const entry of raw.flat() as HistoryPayload[]) {
      if (!entry || entry.draft || entry.prerelease) continue;
      const version = entry.tag_name ?? entry.name ?? entry.id ?? entry.title;
      if (!version) continue;
      const assets = Array.isArray(entry.assets) ? entry.assets : [];
      entries.push({
        version,
        date: entry.published_at ?? entry.created_at ?? entry.date ?? "",
        url: entry.html_url ?? entry.url,
        dmgUrl: pickAsset(assets, (name) =>
          name.endsWith("-macos-universal.dmg"),
        ),
        sumsUrl: pickAsset(
          assets,
          (name) => name === "SHA256SUMS" || name.endsWith("SHA256SUMS"),
        ),
      });
    }
    return entries.sort((a, b) =>
      a.date < b.date ? 1 : a.date > b.date ? -1 : 0,
    );
  } catch (error) {
    const code = (error as { code?: string } | null)?.code;
    if (code !== "ENOENT") warn("could not read the release history", error);
    return [];
  }
}

/**
 * Latest installable macOS release, or `null` when none is available.
 * Reads `PARLA_RELEASE_FILE`, then `.release.json` in the current working
 * directory. Returns `null` for a missing file, a draft or pre-release, or a
 * release without a universal DMG.
 */
export async function latestRelease(
  path?: string,
): Promise<LatestRelease | null> {
  const file =
    path ??
    process.env.PARLA_RELEASE_FILE ??
    join(process.cwd(), ".release.json");
  try {
    const raw = await readFile(file, "utf8");
    const data = JSON.parse(raw) as ReleasePayload;
    if (data.draft || data.prerelease) return null;
    const version = data.tag_name ?? data.name;
    if (!version) return null;
    const assets = Array.isArray(data.assets) ? data.assets : [];
    const dmgUrl = pickAsset(assets, (name) =>
      name.endsWith("-macos-universal.dmg"),
    );
    if (!dmgUrl) return null;
    const sumsUrl =
      pickAsset(
        assets,
        (name) => name === "SHA256SUMS" || name.endsWith("SHA256SUMS"),
      ) ?? "";
    const date = data.published_at ?? data.created_at ?? "";
    return { version, dmgUrl, sumsUrl, date };
  } catch (error) {
    const code = (error as { code?: string } | null)?.code;
    if (code !== "ENOENT") warn("could not read the release data", error);
    return null;
  }
}

/** One lookup per sums file: the page and the tests may ask twice. */
const checksums = new Map<string, Promise<string | undefined>>();

async function fetchChecksum(
  sumsUrl: string,
  dmgName: string,
): Promise<string | undefined> {
  try {
    const res = await fetch(sumsUrl, { signal: AbortSignal.timeout(4000) });
    if (!res.ok) {
      warn(`checksum unavailable (HTTP ${res.status})`);
      return undefined;
    }
    for (const line of (await res.text()).split("\n")) {
      const [hash, ...rest] = line.trim().split(/\s+/);
      const name = rest.join(" ").replace(/^\*/, "");
      if (hash && name === dmgName && /^[0-9a-f]{64}$/i.test(hash)) {
        return hash.toLowerCase();
      }
    }
    return undefined;
  } catch {
    // A missing checksum is not a reason to fail the build.
    return undefined;
  }
}

/**
 * SHA-256 of the universal DMG, read from the release's `SHA256SUMS` asset.
 * `undefined` without an asset or when the file cannot be read; the downloads
 * page then shows the attestation command alone.
 */
export function releaseChecksum(
  release: LatestRelease,
): Promise<string | undefined> {
  const name = dmgFileName(release.dmgUrl);
  if (!release.sumsUrl || !name) return Promise.resolve(undefined);
  const key = `${release.sumsUrl} ${name}`;
  let value = checksums.get(key);
  if (!value) {
    value = fetchChecksum(release.sumsUrl, name);
    checksums.set(key, value);
  }
  return value;
}
