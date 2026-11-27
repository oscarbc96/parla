import {
  type SerializedPageTree,
  useFumadocsLoader,
} from "fumadocs-core/source/client";
import type { TOCItemType } from "fumadocs-core/toc";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import {
  DocsDescription,
  DocsPage,
  DocsTitle,
  MarkdownCopyButton,
  ViewOptionsPopover,
} from "fumadocs-ui/layouts/docs/page";
import { Provider, type ProviderProps } from "@/components/provider";
import { languageSelectSlot } from "@/components/SidebarLinks";
import {
  DocsBanner,
  type DocsBannerProps,
  VersionSwitcher,
  type VersionLink,
  type VersionSwitcherText,
} from "@/components/VersionSwitcher";
import { baseOptions } from "@/lib/layout.shared";

export interface DocsProps extends ProviderProps {
  tree: SerializedPageTree;
  title: string;
  description?: string;
  toc: TOCItemType[];
  /** URL of the page as Markdown, for the page actions. */
  markdownUrl: string;
  /** URL of the page source on GitHub. */
  githubUrl: string;
  /** Version menu entries; only passed when the site has more than one version. */
  versionLinks?: VersionLink[];
  /** Accessible label of the version menu. */
  versionLabel?: string;
  /** Search and empty-state strings of the version menu. */
  versionText?: VersionSwitcherText;
  /** Notice above the title on old lines and on the working docs. */
  banner?: DocsBannerProps;
}

/** The React island of docs pages, the content is passed as children. */
export function Docs({
  tree,
  title,
  description,
  toc,
  markdownUrl,
  githubUrl,
  versionLinks,
  versionLabel = "",
  versionText = { search: "", empty: "" },
  banner,
  children,
  ...props
}: DocsProps) {
  const data = useFumadocsLoader({ tree });

  return (
    <Provider {...props}>
      <DocsLayout
        {...baseOptions(props.locale, props.stars)}
        tree={data.tree}
        slots={{ languageSelect: languageSelectSlot }}
        sidebar={
          versionLinks
            ? {
                banner: (
                  <VersionSwitcher
                    label={versionLabel}
                    text={versionText}
                    links={versionLinks}
                  />
                ),
              }
            : undefined
        }
      >
        <DocsPage toc={toc}>
          {banner && <DocsBanner {...banner} />}
          <DocsTitle>{title}</DocsTitle>
          <DocsDescription>{description}</DocsDescription>
          <div className="flex flex-row items-center gap-2 border-b pt-2 pb-6">
            <MarkdownCopyButton markdownUrl={markdownUrl} />
            <ViewOptionsPopover
              markdownUrl={markdownUrl}
              githubUrl={githubUrl}
            />
          </div>
          {children}
        </DocsPage>
      </DocsLayout>
    </Provider>
  );
}
