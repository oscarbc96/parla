import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  dmgFileName,
  latestRelease,
  releaseChecksum,
  releaseHistory,
} from "../src/lib/release";
import { buildSite } from "./build";

const releaseOut = "dist-tests/release";
const noReleaseOut = "dist-tests/release-none";

let dir = "";

beforeAll(async () => {
  dir = await mkdtemp(join(tmpdir(), "parla-release-"));
  rmSyncDirs();
  await Promise.all([
    buildSite(releaseOut, {
      PARLA_RELEASE_FILE: "tests/fixtures/release.json",
      PARLA_CHANGELOG_FIXTURE: "tests/fixtures/no-releases.json",
      PARLA_STARS_FIXTURE: "none",
    }),
    buildSite(noReleaseOut, {
      PARLA_RELEASE_FILE: join(dir, "missing.json"),
      PARLA_CHANGELOG_FIXTURE: "tests/fixtures/no-releases.json",
      PARLA_STARS_FIXTURE: "none",
    }),
  ]);
}, 240_000);

afterAll(async () => {
  if (dir) await rm(dir, { recursive: true, force: true });
});

function rmSyncDirs() {
  // cleaned by caller context via buildSite targets living under dist-tests
}

const read = (out: string, file: string) => Bun.file(`${out}/${file}`).text();

describe("latestRelease", () => {
  test("reads the fixture", async () => {
    const release = await latestRelease("tests/fixtures/release.json");
    expect(release).not.toBeNull();
    expect(release!.version).toBe("v0.1.0");
    expect(release!.dmgUrl).toContain("Parla-0.1.0-macos-universal.dmg");
    expect(release!.sumsUrl).toContain("SHA256SUMS");
    expect(release!.date).toBe("2031-06-01T12:00:00Z");
  });

  test("is null for prerelease, draft or missing asset", async () => {
    const base = JSON.parse(
      await Bun.file("tests/fixtures/release.json").text(),
    ) as Record<string, unknown>;

    const prerelease = join(dir, "prerelease.json");
    await writeFile(prerelease, JSON.stringify({ ...base, prerelease: true }));
    expect(await latestRelease(prerelease)).toBeNull();

    const draft = join(dir, "draft.json");
    await writeFile(draft, JSON.stringify({ ...base, draft: true }));
    expect(await latestRelease(draft)).toBeNull();

    const noAsset = join(dir, "no-asset.json");
    await writeFile(
      noAsset,
      JSON.stringify({
        ...base,
        assets: [
          {
            name: "Parla-0.1.0-linux.tar.gz",
            browser_download_url: "https://example.test/linux.tar.gz",
          },
        ],
      }),
    );
    expect(await latestRelease(noAsset)).toBeNull();
  });

  test("is null when the file is missing", async () => {
    expect(await latestRelease(join(dir, "absent.json"))).toBeNull();
  });
});

describe("releaseHistory", () => {
  test("reads the changelog fixture and skips prereleases", async () => {
    const entries = await releaseHistory("tests/fixtures/releases.json");
    expect(entries.map((entry) => entry.version)).toEqual(["v0.1.0"]);
    expect(entries[0].url).toContain("/releases/tag/v0.1.0");
    expect(entries[0].dmgUrl).toBeUndefined();
  });

  test("keeps the dmg and checksum assets, newest first", async () => {
    const file = join(dir, "history.json");
    const older = {
      tag_name: "v0.1.0",
      published_at: "2030-01-01T00:00:00Z",
      assets: [
        {
          name: "Parla-0.1.0-macos-universal.dmg",
          browser_download_url: "https://example.test/Parla-0.1.0.dmg",
        },
        {
          name: "SHA256SUMS",
          browser_download_url: "https://example.test/sums",
        },
      ],
    };
    const newer = {
      tag_name: "v0.2.0",
      published_at: "2031-01-01T00:00:00Z",
      assets: [
        {
          name: "Parla-0.2.0-macos-universal.dmg",
          browser_download_url: "https://example.test/Parla-0.2.0.dmg",
        },
      ],
    };
    await writeFile(file, JSON.stringify([[newer], [older]]));
    const entries = await releaseHistory(file);
    expect(entries.map((entry) => entry.version)).toEqual(["v0.2.0", "v0.1.0"]);
    expect(entries[0].dmgUrl).toBe("https://example.test/Parla-0.2.0.dmg");
    expect(entries[1].sumsUrl).toBe("https://example.test/sums");
  });

  test("is empty when the file is missing", async () => {
    expect(await releaseHistory(join(dir, "absent-history.json"))).toEqual([]);
  });
});

describe("releaseChecksum", () => {
  const hash = "a".repeat(64);
  const release = (sumsUrl: string) => ({
    version: "v0.1.0",
    dmgUrl: "https://example.test/download/Parla-0.1.0-macos-universal.dmg",
    sumsUrl,
    date: "",
  });

  test("reads the hash of the dmg from SHA256SUMS", async () => {
    const server = Bun.serve({
      port: 0,
      fetch: () =>
        new Response(
          `${"b".repeat(64)}  Parla-0.9.0-macos-universal.dmg\n${hash}  Parla-0.1.0-macos-universal.dmg\n`,
        ),
    });
    try {
      expect(
        await releaseChecksum(
          release(`http://localhost:${server.port}/SHA256SUMS`),
        ),
      ).toBe(hash);
    } finally {
      server.stop(true);
    }
  });

  test("is undefined without the asset or on an HTTP error", async () => {
    expect(await releaseChecksum(release(""))).toBeUndefined();
    const server = Bun.serve({
      port: 0,
      fetch: () => new Response("missing", { status: 404 }),
    });
    try {
      expect(
        await releaseChecksum(
          release(`http://localhost:${server.port}/SHA256SUMS`),
        ),
      ).toBeUndefined();
    } finally {
      server.stop(true);
    }
  });

  test("dmgFileName reads the asset name", () => {
    expect(
      dmgFileName(
        "https://example.test/download/Parla-0.1.0-macos-universal.dmg",
      ),
    ).toBe("Parla-0.1.0-macos-universal.dmg");
  });
});

describe("release file discovery", () => {
  test("builds with site/.release.json present and no PARLA_RELEASE_FILE and sees the download button", async () => {
    const releaseFile = ".release.json";
    const out = "dist-tests/release-default-file";
    const raw = await Bun.file("tests/fixtures/release.json").text();
    await writeFile(releaseFile, raw);
    try {
      await buildSite(out, {
        PARLA_CHANGELOG_FIXTURE: "tests/fixtures/no-releases.json",
        PARLA_STARS_FIXTURE: "none",
      });
      const html = await read(out, "downloads/index.html");
      expect(html).toContain("Download for macOS");
      expect(html).toContain("v0.1.0");
      expect(html).toContain("Parla-0.1.0-macos-universal.dmg");
      expect(html).not.toContain('data-testid="no-release"');
    } finally {
      await rm(releaseFile, { force: true });
    }
  }, 240_000);
});

describe("downloads and home pages", () => {
  test("show the download button with version when a release exists", async () => {
    const html = await read(releaseOut, "downloads/index.html");
    expect(html).toContain("Download for macOS");
    expect(html).toContain("v0.1.0");
    expect(html).toContain("Parla-0.1.0-macos-universal.dmg");
    expect(html).toContain("macOS 14 or later, Apple silicon and Intel");
    expect(html).toContain("SHA256SUMS");
    expect(html).toContain("gh attestation verify");
    expect(html).not.toContain('data-testid="no-release"');

    const home = await read(releaseOut, "index.html");
    expect(home).toContain("Download for macOS");
    expect(home).toContain("v0.1.0");
  });

  test("show no-release text without a release", async () => {
    const html = await read(noReleaseOut, "downloads/index.html");
    expect(html).toContain('data-testid="no-release"');
    expect(html).toContain("Nothing published yet");
    expect(html).toContain("https://github.com/oscarbc96/parla/releases");
    expect(html).not.toContain("Download for macOS");

    const home = await read(noReleaseOut, "index.html");
    expect(home).toContain("Nothing published yet");
  });
});
