import type { APIRoute, GetStaticPaths } from "astro";
import { createFromSource } from "fumadocs-core/search/server";
import {
  defaultVersion,
  docsVersions,
  getVersionSource,
  type DocsVersion,
} from "@/lib/source";

export const prerender = true;

/** One static index per version besides the default, which `/api/search.json` serves. */
export const getStaticPaths = (() =>
  docsVersions
    .filter((v) => v !== defaultVersion)
    .map((version) => ({
      params: { version: version.segment! },
      props: { version },
    }))) satisfies GetStaticPaths;

export const GET: APIRoute<{ version: DocsVersion }> = ({ props }) =>
  createFromSource(getVersionSource(props.version), {
    localeMap: { en: "english", es: "spanish" },
  }).staticGET();
