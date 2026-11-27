/**
 * Which docs version the search dialog covers, from the page pathname. Pure,
 * no Astro imports, so the client bundle and the tests can both use it.
 */

/** Locale codes that prefix a path (`/es/docs/...`), besides the default. */
const LOCALE = /^[a-z]{2,3}(?:-[A-Za-z0-9]+)?$/;

/**
 * The version URL segment (`next`, `v0.1`) of a docs page, or `null` for the
 * default version and for pages outside the docs. `base` is the site base
 * path without a trailing slash (`/parla`).
 */
export function searchSegment(pathname: string, base = ""): string | null {
  const rest =
    base && pathname.startsWith(base) ? pathname.slice(base.length) : pathname;
  const parts = rest.split("/").filter(Boolean);
  if (parts[0] !== "docs" && parts[0] !== undefined && LOCALE.test(parts[0])) {
    parts.shift();
  }
  if (parts[0] !== "docs") return null;
  const segment = parts[1];
  if (segment === "next" || (segment !== undefined && /^v\d/.test(segment))) {
    return segment;
  }
  return null;
}
