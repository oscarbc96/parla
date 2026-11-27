import type { GitHubChangelogEntry } from "@fumadocs/changelog/github";
import Link from "fumadocs-core/link";
import { createMarkdownRenderer } from "fumadocs-core/content/md";
import { remarkGfm } from "fumadocs-core/mdx-plugins/remark-gfm";
import { rehypeCode } from "fumadocs-core/mdx-plugins/rehype-code";
import defaultMdxComponents from "fumadocs-ui/mdx";
import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";
import { type ComponentProps, type ReactNode, Suspense } from "react";
import { cn } from "../lib/cn";

export interface ChangelogTimelineEntry extends Omit<
  GitHubChangelogEntry,
  "prerelease"
> {
  prerelease?: boolean;
  /** rendered content, replaces `content` */
  body?: ReactNode;
}

const { Markdown } = createMarkdownRenderer({
  remarkPlugins: [remarkGfm],
  // `tab` wraps code blocks in MDX elements, which Markdown can't render
  rehypePlugins: [[rehypeCode, { fallbackLanguage: "plaintext", tab: false }]],
});

const mdxComponents = {
  ...defaultMdxComponents,
  img: "img" as const,
};

export interface ChangelogLabels {
  /** BCP 47 tag used to format dates */
  dateLocale: string;
  prerelease: string;
  newer: string;
  older: string;
}

function formats(locale: string) {
  return {
    month: new Intl.DateTimeFormat(locale, {
      month: "long",
      year: "numeric",
      timeZone: "UTC",
    }),
    day: new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
      timeZone: "UTC",
    }),
  };
}

export function ChangelogTimeline({
  entries,
  labels,
}: {
  entries: ChangelogTimelineEntry[];
  labels: ChangelogLabels;
}) {
  const { month: monthFormat } = formats(labels.dateLocale);
  const months = new Map<string, ChangelogTimelineEntry[]>();
  for (const entry of [...entries].sort(
    (a, b) => b.date.getTime() - a.date.getTime(),
  )) {
    const month = monthFormat.format(entry.date);
    const items = months.get(month);
    if (items) items.push(entry);
    else months.set(month, [entry]);
  }

  return (
    <div className="flex flex-col gap-8">
      {Array.from(months, ([month, items]) => (
        <section key={month}>
          <h2 className="sticky top-14 z-1 bg-parla-app/80 py-4 text-sm font-medium text-parla-muted backdrop-blur-sm">
            {month}
          </h2>
          <ol className="ms-3 border-s border-parla-line ps-6">
            {items.map((entry) => (
              <ChangelogItem key={entry.id} entry={entry} labels={labels} />
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}

function ChangelogItem({
  entry,
  labels,
}: {
  entry: ChangelogTimelineEntry;
  labels: ChangelogLabels;
}) {
  const dayFormat = formats(labels.dateLocale).day;
  const versions = entry.versions?.filter(
    ({ name, version }) => `${name}@${version}` !== entry.title,
  );

  return (
    <li id={entry.id} className="scroll-mt-28 pb-10 last:pb-0">
      <div className="mb-2 flex flex-wrap items-center gap-1.5">
        <time
          dateTime={entry.date.toISOString()}
          className="relative pe-2 text-xs font-semibold text-parla-signal-text tabular-nums"
        >
          <span className="absolute -start-6 top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-parla-signal rtl:translate-x-1/2" />
          {dayFormat.format(entry.date)}
        </time>
        {versions?.map(({ name, version }) => (
          <span
            key={name}
            className="rounded-md border border-parla-line bg-parla-sunken px-1.5 py-0.5 font-mono text-xs text-parla-muted"
          >
            {name}@{version}
          </span>
        ))}
        {entry.prerelease && (
          <span className="rounded-md border border-parla-signal/40 bg-parla-signal-soft px-1.5 py-0.5 text-xs font-medium text-parla-signal-text">
            {labels.prerelease}
          </span>
        )}
      </div>
      <h3 className="parla-h2 mb-3">
        {entry.url ? (
          <a href={entry.url} className="no-underline hover:text-parla-link">
            {entry.title}
          </a>
        ) : (
          entry.title
        )}
      </h3>
      <div className="prose prose-sm text-parla-muted">
        {entry.body ??
          (entry.content && (
            <Suspense>
              <Markdown async components={mdxComponents}>
                {entry.content}
              </Markdown>
            </Suspense>
          ))}
      </div>
    </li>
  );
}

/** links to the pages of newer and older entries */
export function ChangelogPagination({
  newer,
  older,
  labels,
  className,
  ...props
}: ComponentProps<"nav"> & {
  newer?: string;
  older?: string;
  labels: ChangelogLabels;
}) {
  if (!newer && !older) return;
  const linkClass =
    "inline-flex items-center gap-1.5 rounded-lg border border-parla-line bg-parla-surface px-3 py-1.5 text-sm font-medium text-parla-text no-underline transition-colors hover:bg-parla-hover";

  return (
    <nav {...props} className={cn("flex flex-row", className)}>
      {newer && (
        <Link href={newer} className={linkClass}>
          <ChevronLeftIcon className="size-4 rtl:rotate-180" />
          {labels.newer}
        </Link>
      )}
      {older && (
        <Link href={older} className={cn(linkClass, "ms-auto")}>
          {labels.older}
          <ChevronRightIcon className="size-4 rtl:rotate-180" />
        </Link>
      )}
    </nav>
  );
}

export interface UnreleasedLabels {
  heading: string;
  badge: string;
  version: string;
  note: string;
}

/** Pending changes, clearly marked as not released yet. */
export function UnreleasedSection({
  notes,
  version,
  url,
  labels,
}: {
  notes: string;
  version?: string;
  url?: string;
  labels: UnreleasedLabels;
}) {
  return (
    <section
      data-testid="changelog-unreleased"
      className="rounded-xl border border-dashed border-parla-signal/40 bg-parla-surface p-6"
    >
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <span className="rounded-md border border-parla-signal/40 bg-parla-signal-soft px-1.5 py-0.5 text-xs font-medium text-parla-signal-text">
          {labels.badge}
        </span>
        {version && (
          <span className="rounded-md border border-parla-line bg-parla-sunken px-1.5 py-0.5 font-mono text-xs text-parla-muted">
            {labels.version.replace("{version}", version)}
          </span>
        )}
      </div>
      <h2 className="parla-h2">
        {url ? (
          <a href={url} className="no-underline hover:text-parla-link">
            {labels.heading}
          </a>
        ) : (
          labels.heading
        )}
      </h2>
      <p className="mt-1 mb-4 text-sm text-parla-muted">{labels.note}</p>
      <div className="prose prose-sm text-parla-muted">
        <Suspense>
          <Markdown async components={mdxComponents}>
            {notes}
          </Markdown>
        </Suspense>
      </div>
    </section>
  );
}
