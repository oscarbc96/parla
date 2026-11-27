import { describe, expect, test } from "bun:test";
import { searchSegment } from "../src/lib/docs-search-scope";

describe("searchSegment", () => {
  const base = "/parla";
  test.each([
    ["/parla/docs/next/install/", "next"],
    ["/parla/docs/next", "next"],
    ["/parla/es/docs/v0.1/old-only/", "v0.1"],
    ["/parla/ca/docs/v0.10/", "v0.10"],
    ["/parla/docs/v12.3/a/b/", "v12.3"],
    ["/docs/next/", "next"],
  ])("%s -> %s", (pathname, expected) => {
    expect(searchSegment(pathname, base)).toBe(expected);
  });

  test.each([
    "/parla/",
    "/parla/downloads/",
    "/parla/changelog/",
    "/parla/docs/",
    "/parla/docs/install/",
    "/parla/es/docs/install/",
    "/parla/docs/nextish/",
    "/parla/docs/vision/",
    "/parla/docs/install/next/",
    "/parla/es/",
  ])("%s -> default", (pathname) => {
    expect(searchSegment(pathname, base)).toBeNull();
  });
});
