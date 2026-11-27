import { getCollection } from "astro:content";
import { llms, loader } from "fumadocs-core/source";
import {
  toFumadocsSource,
  type MetaEntry,
  type PageEntry,
} from "@fumadocs/astro/source";
import { defaultLocale, i18n, localePath } from "@/i18n";
import {
  loadVersions,
  metaCollectionName,
  type DocsVersion,
} from "@/lib/docs-versions";

export type { DocsVersion };

/** Every docs version: the working docs first, then the release lines newest first. */
export const docsVersions: DocsVersion[] = loadVersions();

/** True when snapshots exist, so the switcher and banners apply. */
export const hasVersions = docsVersions.length > 1;

/** The version served at `/docs/`: the newest release line, or the working docs. */
export const defaultVersion = docsVersions.find((v) => v.segment === null)!;

/** Locale-free docs path prefix of a version: `/docs` or `/docs/v0.1`. */
export function versionBase(version: DocsVersion): string {
  return version.segment ? `/docs/${version.segment}` : "/docs";
}

/** Locale-free path of a docs page of a version, ending in `/`. */
export function versionedDocsPath(
  version: DocsVersion,
  slugs: string[],
): string {
  const base = versionBase(version);
  return slugs.length > 0 ? `${base}/${slugs.join("/")}/` : `${base}/`;
}

/** A docs page entry; the collection schema makes `title` required. */
type DocsPageEntry = PageEntry & { data: { title: string } };

function createSource(
  version: DocsVersion,
  pages: DocsPageEntry[],
  metas: MetaEntry[],
) {
  const base = versionBase(version);
  return loader({
    source: toFumadocsSource(pages, metas),
    baseUrl: localePath(defaultLocale, base),
    i18n,
    url: (slugs, locale) =>
      localePath(
        locale ?? i18n.defaultLanguage,
        `${base}/${slugs.join("/")}`,
      ).replace(/\/$/, "") || "/",
  });
}

type DocsSource = ReturnType<typeof createSource>;

const sources = new Map<string, DocsSource>(
  await Promise.all(
    docsVersions.map(async (version) => {
      // Collections are built per version at runtime, so Astro cannot type
      // them; they are page and meta entries by construction (content.config.ts).
      const pages = (await getCollection(
        version.collection as never,
      )) as unknown as DocsPageEntry[];
      const metas = (await getCollection(
        metaCollectionName(version) as never,
      )) as unknown as MetaEntry[];
      return [version.id, createSource(version, pages, metas)] as const;
    }),
  ),
);

/** The Fumadocs source of one docs version. */
export function getVersionSource(version: DocsVersion): DocsSource {
  return sources.get(version.id)!;
}

/** The version a docs page belongs to, from its first URL segment, or the default. */
export function versionFromSegment(segment: string | undefined) {
  return (
    docsVersions.find((v) => v.segment !== null && v.segment === segment) ??
    defaultVersion
  );
}

/**
 * Source of the default version (served at `/docs/`). Search, `llms*.txt`,
 * the sitemap and Open Graph images cover only this version.
 */
export const source = getVersionSource(defaultVersion);

// `next` and `v<digit>...` are URL segments of the versions, not page slugs.
for (const page of getVersionSource(
  docsVersions.find((v) => v.id === "next")!,
).getPages(defaultLocale)) {
  const first = page.slugs[0];
  if (first !== undefined && (first === "next" || /^v\d/.test(first))) {
    throw new Error(
      `docs page "${page.path}" has the reserved top-level slug "${first}" ` +
        `("next" and "v<digit>..." are docs version URL segments)`,
    );
  }
}

/** URL of the Markdown form of a docs page, served next to its HTML page. */
export function getPageMarkdownUrl(
  page: { slugs: string[]; locale?: string },
  version: DocsVersion = defaultVersion,
) {
  const segments = page.slugs.length > 0 ? [...page.slugs] : ["index"];
  segments[segments.length - 1] += ".md";
  return localePath(
    page.locale ?? defaultLocale,
    `${versionBase(version)}/${segments.join("/")}`,
  );
}

function createLlms(version: DocsVersion) {
  const src = getVersionSource(version);
  return llms(src, {
    renderUrl: (item, { lang }) => {
      const page = src.getNodePage(item, lang);
      return page ? getPageMarkdownUrl(page, version) : item.url;
    },
    renderPage: async (page) => `# ${page.data.title} (${page.url})

${page.data.description ? `${page.data.description}\n\n` : ""}${await page.data.getText("processed")}`,
  });
}

const llmsByVersion = new Map(docsVersions.map((v) => [v.id, createLlms(v)]));

/** `llms.txt` helpers of one docs version. */
export function getDocsLlms(version: DocsVersion) {
  return llmsByVersion.get(version.id)!;
}

/** `llms.txt` helpers of the default version. */
export const docsLlms = getDocsLlms(defaultVersion);
