import { docsVersions, getVersionSource, type DocsVersion } from "@/lib/source";

/** A docs page of one version in one locale, with its `[...slug]` route param. */
export interface DocsEntry {
  version: DocsVersion;
  locale: string;
  page: ReturnType<ReturnType<typeof getVersionSource>["getPages"]>[number];
  /** Value of the `slug` param: version segment plus page slugs, `undefined` for `/docs/`. */
  slug: string | undefined;
  /** Same, for the `.md` routes, where the index page is `index`. */
  mdSlug: string;
}

/** Every docs page of every version in `locales`, for the static routes. */
export function docsEntries(locales: string[]): DocsEntry[] {
  return docsVersions.flatMap((version) =>
    locales.flatMap((locale) =>
      getVersionSource(version)
        .getPages(locale)
        .map((page) => {
          const parts = [
            ...(version.segment ? [version.segment] : []),
            ...page.slugs.map((s) => decodeURI(s)),
          ];
          return {
            version,
            locale,
            page,
            slug: parts.join("/") || undefined,
            mdSlug:
              page.slugs.length > 0
                ? parts.join("/")
                : [...parts, "index"].join("/"),
          };
        }),
    ),
  );
}
