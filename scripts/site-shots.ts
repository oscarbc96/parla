/**
 * Marketing and docs screenshots for the site, rendered from visual baselines
 * (FakeProvider scenarios, never a real account).
 *
 *   bun scripts/site-shots.ts            write site/public/shots/*.webp + manifest.json
 *   bun scripts/site-shots.ts --check    fail when a listed baseline changed
 *
 * The list lives in `scripts/site-shots.config.json`: per shot an output
 * name, a scenario, the appearances and an optional crop in baseline pixels.
 * `manifest.json` records the SHA-256 of every source baseline; `--check`
 * (`mise run site:check-shots`, part of `mise run lint`) compares it, so a
 * UI change that moves a baseline cannot leave stale site images behind.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { has, inRepo } from "./lib";

export interface Crop {
  left: number;
  top: number;
  width: number;
  height: number;
}
export interface Shot {
  output: string;
  scenario: string;
  appearances: ("light" | "dark")[];
  crop: Crop | null;
}
export interface ShotConfig {
  platform: string;
  shots: Shot[];
}
export interface ManifestEntry {
  scenario: string;
  appearance: string;
  source: string;
  hash: string;
  bytes?: number;
}
export interface Manifest {
  shots: Record<string, ManifestEntry>;
}

export const MAX_BYTES = 150 * 1024;
const MAX_WIDTH = 1600;
const FIX = "run `mise run site:shots` and commit site/public/shots";

export function outputsOf(config: ShotConfig) {
  return config.shots.flatMap((s) =>
    s.appearances.map((a) => ({
      file: `${s.output}-${a}.webp`,
      shot: s,
      appearance: a,
    })),
  );
}

export const sourceOf = (platform: string) => (scenario: string, appearance: string) =>
  `crates/ui_testing/baselines/${platform}/components/${scenario}@2x-${appearance}.png`;

/** Problems between the config, the manifest and the baselines on disk (empty = in sync). */
export function checkManifest(
  config: ShotConfig,
  manifest: Manifest,
  hashOf: (path: string) => string | null,
  source: (scenario: string, appearance: string) => string,
): string[] {
  const errors: string[] = [];
  const expected = new Set<string>();
  for (const { file, shot, appearance } of outputsOf(config)) {
    expected.add(file);
    const src = source(shot.scenario, appearance);
    const now = hashOf(src);
    const entry = manifest.shots[file];
    if (now === null) {
      errors.push(`${file}: baseline ${src} is missing; fix site-shots.config.json or the scenario`);
    } else if (!entry) {
      errors.push(`${file}: not in the manifest; ${FIX}`);
    } else if (entry.hash !== now) {
      errors.push(`${file}: baseline ${src} changed since the image was made; ${FIX}`);
    }
  }
  for (const file of Object.keys(manifest.shots)) {
    if (!expected.has(file)) errors.push(`${file}: in the manifest but not in site-shots.config.json; ${FIX}`);
  }
  return errors;
}

const sha = (path: string) =>
  new Bun.CryptoHasher("sha256").update(readFileSync(path)).digest("hex");

function loadConfig(): ShotConfig {
  return JSON.parse(readFileSync(inRepo("scripts/site-shots.config.json"), "utf8"));
}

const manifestPath = () => inRepo("site/public/shots/manifest.json");

function check(): number {
  const config = loadConfig();
  const mp = manifestPath();
  const manifest: Manifest = existsSync(mp)
    ? JSON.parse(readFileSync(mp, "utf8"))
    : { shots: {} };
  const hashOf = (p: string) => (existsSync(inRepo(p)) ? sha(inRepo(p)) : null);
  const errors = checkManifest(config, manifest, hashOf, sourceOf(config.platform));
  for (const f of outputsOf(config)) {
    if (!existsSync(inRepo("site/public/shots", f.file))) {
      errors.push(`${f.file}: image missing; ${FIX}`);
    }
  }
  for (const e of errors) console.error(`site-shots: ${e}`);
  if (errors.length === 0) console.log("site-shots: ok");
  return errors.length ? 1 : 0;
}

async function render(): Promise<number> {
  const config = loadConfig();
  let sharp: typeof import("sharp");
  try {
    sharp = createRequire(inRepo("site/package.json"))("sharp");
  } catch {
    console.error("site-shots: sharp is missing; run `mise run site:install`");
    return 1;
  }
  const dir = inRepo("site/public/shots");
  mkdirSync(dir, { recursive: true });
  const source = sourceOf(config.platform);
  const manifest: Manifest = { shots: {} };
  for (const { file, shot, appearance } of outputsOf(config)) {
    const src = source(shot.scenario, appearance);
    if (!existsSync(inRepo(src))) {
      console.error(`site-shots: baseline ${src} is missing`);
      return 1;
    }
    let img = sharp(inRepo(src));
    if (shot.crop) img = img.extract(shot.crop);
    img = img.resize({ width: MAX_WIDTH, withoutEnlargement: true });
    let buf: Buffer = Buffer.alloc(0);
    for (let q = 84; q >= 40; q -= 6) {
      buf = await img.clone().webp({ quality: q, effort: 6 }).toBuffer();
      if (buf.length <= MAX_BYTES) break;
    }
    if (buf.length > MAX_BYTES) {
      console.error(`site-shots: ${file} is ${buf.length} bytes, over ${MAX_BYTES}`);
      return 1;
    }
    writeFileSync(join(dir, file), buf);
    manifest.shots[file] = {
      scenario: shot.scenario,
      appearance,
      source: src,
      hash: sha(inRepo(src)),
      bytes: buf.length,
    };
    console.log(`site-shots: ${file} ${(buf.length / 1024).toFixed(0)} KB`);
  }
  writeFileSync(manifestPath(), `${JSON.stringify(manifest, null, 2)}\n`);
  return 0;
}

if (import.meta.main) {
  const argv = process.argv.slice(2);
  process.exit(has(argv, "--check") ? check() : await render());
}
