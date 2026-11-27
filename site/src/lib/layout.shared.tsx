import { Star } from "lucide-react";
import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { GithubRowIcon } from "@/components/SidebarLinks";
import { defaultLocale, getUi, localePath } from "@/i18n";
import { logoMarkup } from "@/lib/logo";

const repoUrl = "https://github.com/oscarbc96/parla";

/**
 * `stars` is the compact star count fetched at build time. All site links are
 * top-bar only (`on: "nav"`); inside the docs sidebar they live in the footer
 * (see `SidebarLinks`), next to the language and theme controls.
 */
export function baseOptions(
  locale: string = defaultLocale,
  stars?: string,
): BaseLayoutProps {
  const ui = getUi(locale);
  return {
    nav: {
      title: (
        <span className="parla-wordmark inline-flex items-center gap-2">
          <span
            className="size-6"
            // The bundled brand mark, a trusted build-time string.
            dangerouslySetInnerHTML={{ __html: logoMarkup }}
          />
          Parla
        </span>
      ),
      url: localePath(locale, "/"),
    },
    // Light, dark and system; the provider defaults to system (see provider.tsx).
    themeSwitch: { mode: "light-dark-system" },
    links: [
      {
        text: ui.nav.docs,
        url: localePath(locale, "/docs/"),
        active: "nested-url",
        on: "nav",
      },
      {
        text: ui.nav.download,
        url: localePath(locale, "/downloads/"),
        active: "nested-url",
        on: "nav",
      },
      {
        text: ui.nav.changelog,
        url: localePath(locale, "/changelog/"),
        active: "nested-url",
        on: "nav",
      },
      {
        type: "button" as const,
        text: stars ? (
          <span className="inline-flex items-center gap-1">
            {ui.nav.github}
            <Star size={16} fill="currentColor" aria-hidden="true" />
            {stars}
          </span>
        ) : (
          ui.nav.github
        ),
        url: repoUrl,
        external: true,
        secondary: true,
        on: "nav",
      },
      // Docs sidebar only: the repo link sits left of the theme switch in the footer row.
      {
        type: "icon" as const,
        label: stars ? ui.starsLabel.replace("{count}", stars) : ui.nav.github,
        text: ui.nav.github,
        url: repoUrl,
        external: true,
        on: "menu" as const,
        icon: <GithubRowIcon label={ui.nav.github} stars={stars} />,
      },
    ],
  };
}
