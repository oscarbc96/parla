import type { ReactNode } from "react";
import { RootProvider } from "fumadocs-ui/provider/astro";
import DefaultSearchDialog, {
  SearchScopeContext,
  type SearchScope,
} from "@/components/search";
import { defaultLocale, getUi, locales, switchLocalePath } from "@/i18n";

export interface ProviderProps {
  /** `Astro.url.pathname` */
  pathname: string;
  /** `Astro.params` */
  params: Record<string, string | undefined>;
  /** Locale code of the page. */
  locale?: string;
  /** Version tags of the search dialog; only with several docs versions. */
  searchScope?: Omit<SearchScope, "pathname">;
  /** Compact GitHub star count, when known. */
  stars?: string;
  children?: ReactNode;
}

export function Provider({
  pathname,
  params,
  locale = defaultLocale,
  searchScope,
  children,
}: ProviderProps) {
  return (
    <SearchScopeContext.Provider
      value={searchScope ? { ...searchScope, pathname } : null}
    >
      <RootProvider
        pathname={pathname}
        params={params}
        search={{ SearchDialog: DefaultSearchDialog }}
        theme={{ defaultTheme: "system", enableSystem: true }}
        i18n={{
          locale,
          locales: locales.map((item) => ({
            locale: item.code,
            name: item.name,
          })),
          translations: getUi(locale).translations,
          onLocaleChange: (target) => {
            window.location.assign(switchLocalePath(pathname, target));
          },
        }}
      >
        {children}
      </RootProvider>
    </SearchScopeContext.Provider>
  );
}
