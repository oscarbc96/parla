import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type { GitHubChangelogEntry } from "@fumadocs/changelog/github";
import { fromCommits, parseReleasePr, type Unreleased } from "./unreleased";

/**
 * Build-time GitHub data (releases, stars). Nothing here may fail the build:
 * every failure logs a warning and falls back to "no data".
 */

export const owner = "oscarbc96";
export const repo = "parla";
export const repoUrl = `https://github.com/${owner}/${repo}`;
export const perPage = 30;

const token = process.env.GITHUB_TOKEN || undefined;

function warn(message: string, error?: unknown) {
  const detail = error instanceof Error ? `: ${error.message}` : "";
  console.warn(`[parla-site] ${message}${detail}`);
}

export interface RawRelease {
  id?: number | string;
  tag_name?: string | null;
  name?: string | null;
  body?: string | null;
  html_url?: string | null;
  published_at?: string | null;
  created_at?: string | null;
  draft?: boolean;
  prerelease?: boolean;
  title?: string;
  content?: string;
  url?: string | null;
  date?: string;
}

export function toEntry(raw: RawRelease): GitHubChangelogEntry | undefined {
  if ("title" in raw || "content" in raw) {
    const date = raw.date ? new Date(raw.date) : new Date(0);
    return {
      id: String(raw.id ?? raw.title ?? ""),
      title: raw.title ?? String(raw.tag_name ?? raw.name ?? ""),
      date,
      url: raw.url ?? raw.html_url ?? "",
      content: raw.content ?? raw.body ?? "",
      prerelease: Boolean(raw.prerelease),
    };
  }
  if (raw.draft) return undefined;
  const title = (raw.name || raw.tag_name || "").trim();
  if (!title) return undefined;
  const dateText = raw.published_at ?? raw.created_at ?? "";
  return {
    id: String(raw.tag_name ?? raw.id ?? title),
    title,
    date: dateText ? new Date(dateText) : new Date(0),
    url: raw.html_url ?? "",
    content: raw.body ?? "",
    prerelease: Boolean(raw.prerelease),
  };
}

async function readReleasesFile(
  file: string,
): Promise<GitHubChangelogEntry[] | undefined> {
  try {
    const raw = JSON.parse(await readFile(file, "utf8")) as unknown;
    if (!Array.isArray(raw)) return undefined;
    // `gh api --paginate --slurp` writes one array per page.
    return (raw.flat() as RawRelease[])
      .map(toEntry)
      .filter((entry): entry is GitHubChangelogEntry => entry !== undefined);
  } catch (error) {
    const code = (error as { code?: string } | null)?.code;
    if (code !== "ENOENT") warn("could not read the releases data", error);
    return undefined;
  }
}

function chunk(entries: GitHubChangelogEntry[]): GitHubChangelogEntry[][] {
  const pages: GitHubChangelogEntry[][] = [];
  for (let i = 0; i < entries.length; i += perPage) {
    pages.push(entries.slice(i, i + perPage));
  }
  return pages;
}

async function fetchReleases(): Promise<GitHubChangelogEntry[][]> {
  const fixture = process.env.PARLA_CHANGELOG_FIXTURE;
  if (fixture) {
    const all = await readReleasesFile(fixture);
    return all ? chunk(all) : [];
  }

  const file =
    process.env.PARLA_RELEASES_FILE ?? join(process.cwd(), ".releases.json");
  const all = await readReleasesFile(file);
  return all ? chunk(all) : [];
}

let releases: Promise<GitHubChangelogEntry[][]> | undefined;

/** Release pages, newest first. Empty when there are no releases or on any failure. */
export function getReleasePages(): Promise<GitHubChangelogEntry[][]> {
  releases ??= fetchReleases().catch((error) => {
    warn("could not load releases", error);
    return [];
  });
  return releases.then((pages) => {
    if (pages.length === 0)
      warn("no releases found, rendering the empty changelog");
    return pages;
  });
}

/** `1234` as `1.2k`, `999` as `999`. */
export function formatStars(count: number): string {
  if (count < 1000) return String(count);
  const value = count / 1000;
  const text =
    value < 10
      ? value.toFixed(1).replace(/\.0$/, "")
      : String(Math.round(value));
  return `${text}k`;
}

async function fetchStars(): Promise<number | undefined> {
  const fixture = process.env.PARLA_STARS_FIXTURE;
  if (fixture !== undefined && fixture !== "") {
    const count = Number(fixture);
    return Number.isFinite(count) && count >= 0 ? count : undefined;
  }
  if (fixture === "") return undefined;
  try {
    const headers = new Headers({ accept: "application/vnd.github+json" });
    if (token) headers.set("authorization", `Bearer ${token}`);
    const res = await fetch(`https://api.github.com/repos/${owner}/${repo}`, {
      headers,
    });
    if (!res.ok) {
      warn(`star count unavailable (HTTP ${res.status})`);
      return undefined;
    }
    const data = (await res.json()) as { stargazers_count?: unknown };
    return typeof data.stargazers_count === "number"
      ? data.stargazers_count
      : undefined;
  } catch (error) {
    warn("could not fetch the star count", error);
    return undefined;
  }
}

let stars: Promise<string | undefined> | undefined;

/** Compact star count (`1.2k`), `undefined` when unknown. */
export function getStars(): Promise<string | undefined> {
  stars ??= fetchStars().then((count) =>
    count === undefined ? undefined : formatStars(count),
  );
  return stars;
}

interface UnreleasedFixture {
  version?: string;
  subjects?: string[];
}

function apiHeaders(): Headers {
  const headers = new Headers({ accept: "application/vnd.github+json" });
  if (token) headers.set("authorization", `Bearer ${token}`);
  return headers;
}

async function api<T>(path: string): Promise<T | undefined> {
  const res = await fetch(
    `https://api.github.com/repos/${owner}/${repo}${path}`,
    { headers: apiHeaders() },
  );
  if (!res.ok) {
    warn(`${path} unavailable (HTTP ${res.status})`);
    return undefined;
  }
  return (await res.json()) as T;
}

async function fetchUnreleased(): Promise<Unreleased | undefined> {
  const fixture = process.env.PARLA_UNRELEASED_FIXTURE;
  if (fixture) {
    const raw = JSON.parse(
      await readFile(fixture, "utf8"),
    ) as UnreleasedFixture;
    const found = fromCommits(raw.subjects ?? [], repoUrl);
    return found ? { ...found, version: raw.version } : undefined;
  }
  // Other fixtures mean an offline, deterministic build.
  if (process.env.PARLA_CHANGELOG_FIXTURE || process.env.PARLA_STARS_FIXTURE)
    return undefined;

  // Same gate as the releases list: the workflow writes the releases file
  // only while the repository is public, so nothing shows before launch.
  const file =
    process.env.PARLA_RELEASES_FILE ?? join(process.cwd(), ".releases.json");
  const released = await readReleasesFile(file);
  if (!released) return undefined;

  // 1. The open release-please PR carries the proposed version and notes.
  const pending = await api<
    {
      title?: string;
      body?: string;
      html_url?: string;
      pull_request?: unknown;
    }[]
  >("/issues?state=open&labels=autorelease%3A%20pending&per_page=5");
  const pr = pending?.find((issue) => issue.pull_request);
  if (pr) {
    const parsed = parseReleasePr(pr);
    if (parsed) return parsed;
  }

  // 2. Otherwise the Conventional Commits on main since the latest release.
  const latest = released.find((release) => release.id);
  if (!latest?.id) return undefined;
  const compare = await api<{ commits?: { commit: { message: string } }[] }>(
    `/compare/${encodeURIComponent(latest.id)}...main`,
  );
  const subjects = (compare?.commits ?? []).map(
    (c) => c.commit.message.split("\n")[0] ?? "",
  );
  return fromCommits(subjects, repoUrl);
}

let unreleased: Promise<Unreleased | undefined> | undefined;

/** Pending changes for the top of the changelog; `undefined` when none or on any failure. */
export function getUnreleased(): Promise<Unreleased | undefined> {
  unreleased ??= fetchUnreleased().catch((error) => {
    warn("could not load the unreleased changes", error);
    return undefined;
  });
  return unreleased;
}
