import type { APIRoute, GetStaticPaths, InferGetStaticPropsType } from "astro";
import { defaultLocale } from "@/i18n";
import { docsEntries } from "@/lib/docs-routes";
import { getDocsLlms } from "@/lib/source";

export const prerender = true;

export const getStaticPaths = (() =>
  docsEntries([defaultLocale]).map(({ mdSlug, page, version }) => ({
    params: { slug: mdSlug },
    props: { page, version },
  }))) satisfies GetStaticPaths;

export const GET: APIRoute<
  InferGetStaticPropsType<typeof getStaticPaths>
> = async ({ props: { page, version } }) =>
  new Response(await getDocsLlms(version).page(page), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
