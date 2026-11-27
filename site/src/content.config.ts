import { defineCollection } from "astro:content";
import { metaLoader, pageLoader } from "@fumadocs/astro/loaders";
import { loadVersions, metaCollectionName } from "@/lib/docs-versions";

/**
 * One page and one meta collection per docs version: `docs`/`meta` for the
 * working docs, `v<X>_<Y>`/`v<X>_<Y>_meta` for each frozen snapshot.
 */
export const collections = Object.fromEntries(
  loadVersions().flatMap((version) => [
    [
      version.collection,
      defineCollection({ loader: pageLoader(version.collection) }),
    ],
    [
      metaCollectionName(version),
      defineCollection({ loader: metaLoader(version.collection) }),
    ],
  ]),
);
