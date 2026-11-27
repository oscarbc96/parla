import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import satori from "satori";
import sharp from "sharp";
import { logoDarkDataUri } from "@/lib/logo";

/**
 * Build-time Open Graph images: satori lays out a card (own brand only, no
 * third-party marks) and sharp rasterises it to a 1200x630 PNG.
 */

const require = createRequire(import.meta.url);
const fontDir = require
  .resolve("@fontsource/inter/package.json")
  .replace(/package\.json$/, "files/");

async function loadFonts() {
  const files = [
    ["inter-latin-700-normal.woff", 700],
    ["inter-latin-ext-700-normal.woff", 700],
    ["inter-latin-400-normal.woff", 400],
    ["inter-latin-ext-400-normal.woff", 400],
  ] as const;
  return Promise.all(
    files.map(async ([file, weight]) => ({
      name: "Inter",
      data: await readFile(`${fontDir}${file}`),
      weight,
      style: "normal" as const,
    })),
  );
}

let fonts: ReturnType<typeof loadFonts> | undefined;

type Node = { type: string; props: Record<string, unknown> };

function el(
  type: string,
  style: Record<string, unknown>,
  children?: unknown,
  extra: Record<string, unknown> = {},
): Node {
  return { type, props: { style, children, ...extra } };
}

const mark = logoDarkDataUri;

export interface OgCard {
  locale: string;
  title: string;
  description: string;
}

/** The card as a PNG buffer. */
export async function renderOgImage({
  title,
  description,
}: OgCard): Promise<Buffer> {
  fonts ??= loadFonts();
  const headline = title.replace(/\s*\|\s*Parla$/, "");
  const tree = el(
    "div",
    {
      width: 1200,
      height: 630,
      display: "flex",
      flexDirection: "column",
      justifyContent: "space-between",
      boxSizing: "border-box",
      padding: 72,
      background: "#15132f",
      color: "#ecebf7",
      fontFamily: "Inter",
    },
    [
      el("div", { display: "flex", alignItems: "center" }, [
        el("img", { width: 72, height: 72, marginRight: 24 }, undefined, {
          src: mark,
          width: 72,
          height: 72,
        }),
        el(
          "div",
          {
            fontSize: 56,
            fontWeight: 700,
            letterSpacing: -2,
            color: "#ff7a45",
          },
          "Parla",
        ),
      ]),
      el("div", { display: "flex", flexDirection: "column" }, [
        el(
          "div",
          {
            fontSize: headline.length > 60 ? 54 : 68,
            fontWeight: 700,
            lineHeight: 1.1,
            letterSpacing: -1.5,
          },
          headline,
        ),
        el(
          "div",
          {
            fontSize: 30,
            lineHeight: 1.4,
            marginTop: 28,
            color: "#a8a5c8",
            fontWeight: 400,
          },
          description,
        ),
      ]),
    ],
  );
  const svg = await satori(tree as never, {
    width: 1200,
    height: 630,
    fonts: await fonts,
  });
  return sharp(Buffer.from(svg))
    .png({ compressionLevel: 9, palette: true })
    .toBuffer();
}
