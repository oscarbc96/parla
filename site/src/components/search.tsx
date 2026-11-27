import {
  SearchDialog,
  SearchDialogClose,
  SearchDialogContent,
  SearchDialogHeader,
  SearchDialogIcon,
  SearchDialogInput,
  SearchDialogList,
  SearchDialogOverlay,
  type SharedProps,
} from "fumadocs-ui/components/dialog/search";
import { useStaticSearch } from "fumadocs-core/search/client";
import { useI18n } from "fumadocs-ui/contexts/i18n";
import { createContext, useContext } from "react";
import { searchSegment } from "@/lib/docs-search-scope";

const base = import.meta.env.BASE_URL.replace(/\/$/, "");

/** Version tags of the search dialog; only provided when the site has several versions. */
export interface SearchScope {
  /** Tag text per version segment; `""` is the default version. */
  labels: Record<string, string>;
  /** Accessible label of the tag, with `{version}`. */
  searching: string;
  /** `Astro.url.pathname` of the page. */
  pathname: string;
}

export const SearchScopeContext = createContext<SearchScope | null>(null);

export default function DefaultSearchDialog(props: SharedProps) {
  const { locale } = useI18n();
  const scope = useContext(SearchScopeContext);
  // Without versions there is one index and no scope.
  const segment = scope ? searchSegment(scope.pathname, base) : null;
  const search = useStaticSearch({
    from: segment
      ? `${base}/api/search/${segment}.json`
      : `${base}/api/search.json`,
    locale,
  });
  const tag = scope?.labels[segment ?? ""];

  return (
    <SearchDialog {...search} {...props}>
      <SearchDialogOverlay />
      <SearchDialogContent>
        <SearchDialogHeader>
          <SearchDialogIcon />
          <SearchDialogInput />
          {tag ? (
            <span
              className="shrink-0 rounded-md bg-fd-primary/10 px-1.5 py-0.5 text-xs text-fd-primary"
              aria-label={scope!.searching.replace("{version}", tag)}
              data-search-version
            >
              {tag}
            </span>
          ) : null}
          <SearchDialogClose />
        </SearchDialogHeader>
        <SearchDialogList />
      </SearchDialogContent>
    </SearchDialog>
  );
}
