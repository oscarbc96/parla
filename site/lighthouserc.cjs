/**
 * Lighthouse CI: runs against the built `dist/` served under the `/parla` base.
 * Budgets live here.
 */
const port = 4173;
const origin = `http://localhost:${port}`;
const locales = ["", "es/", "ca/"]; // keep in step with src/i18n/config.json
const pages = ["", "docs/install/", "changelog/"];

const urls = locales.flatMap((l) =>
    pages.map((p) => `${origin}/parla/${l}${p}`),
);

const score = (min) => ["error", { minScore: min }];
const bytes = (max) => ["error", { maxNumericValue: max }];

const common = {
    "categories:performance": score(0.95),
    "categories:accessibility": score(0.95),
    "categories:best-practices": score(0.95),
    "categories:seo": score(1),
    "cumulative-layout-shift": ["error", { maxNumericValue: 0.02 }],
};

module.exports = {
    ci: {
        collect: {
            url: urls,
            numberOfRuns: 3,
            startServerCommand: `bun scripts/serve-dist.ts ${port}`,
            startServerReadyPattern: "serving dist",
            settings: { chromeFlags: "--no-sandbox --headless=new" },
        },
        assert: {
            assertMatrix: [
                // Static pages: no JavaScript files (only small inline snippets).
                {
                    matchingUrlPattern: "/parla/((es|ca)/)?(changelog/)?$",
                    assertions: {
                        ...common,
                        "resource-summary:script:size": bytes(0),
                        "resource-summary:stylesheet:size": bytes(20000),
                    },
                },
                // Docs: the Fumadocs island is the only JavaScript.
                {
                    matchingUrlPattern: "/parla/((es|ca)/)?docs/",
                    assertions: {
                        ...common,
                        "resource-summary:script:size": bytes(230000),
                        "resource-summary:stylesheet:size": bytes(20000),
                    },
                },
            ],
        },
        upload: { target: "filesystem", outputDir: ".lighthouseci" },
    },
};
