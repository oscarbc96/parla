import { ChevronsUpDown } from "lucide-react";
import { useId, useRef, useState, type KeyboardEvent } from "react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "fumadocs-ui/components/ui/popover";

/** One entry of the docs version switcher. */
export interface VersionLink {
  id: string;
  /** Text of the entry, `0.2` or "Unreleased". */
  label: string;
  /** Extra tag, such as "latest". */
  tag?: string;
  href: string;
  current: boolean;
}

export interface DocsBannerProps {
  text: string;
  linkText: string;
  href: string;
}

/** Strings of the switcher besides its accessible label. */
export interface VersionSwitcherText {
  /** Placeholder and accessible name of the search box. */
  search: string;
  /** Shown when no version matches the search. */
  empty: string;
}

const optionClass = (link: VersionLink, active: boolean) =>
  `flex items-center justify-between gap-2 rounded-md px-2 py-1.5 no-underline transition-colors ${
    active ? "bg-fd-accent text-fd-accent-foreground" : ""
  } ${
    link.current ? "font-medium text-fd-primary" : "text-fd-muted-foreground"
  }`;

/**
 * Version combobox at the top of the docs sidebar: a button with the current
 * version opens a popover with a search box and the matching versions (WAI-ARIA
 * combobox with a listbox popup). Without JavaScript a `<noscript>` list of the
 * same links stands in, since the popover only exists once hydrated.
 */
export function VersionSwitcher({
  label,
  text,
  links,
}: {
  label: string;
  text: VersionSwitcherText;
  links: VersionLink[];
}) {
  const current = links.find((link) => link.current) ?? links[0]!;
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const baseId = useId();
  const listId = `${baseId}-list`;
  const optionId = (index: number) => `${baseId}-option-${index}`;

  const needle = query.trim().toLowerCase();
  const matches = needle
    ? links.filter(
        (link) =>
          link.label.toLowerCase().includes(needle) ||
          link.id.toLowerCase().includes(needle),
      )
    : links;
  const activeLink = matches[active];

  const onOpenChange = (next: boolean) => {
    setOpen(next);
    setQuery("");
    setActive(Math.max(0, links.indexOf(current)));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (matches.length === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((active + step + matches.length) % matches.length);
    } else if (event.key === "Enter" && activeLink) {
      event.preventDefault();
      window.location.assign(activeLink.href);
    }
  };

  return (
    <nav aria-label={label} className="parla-version-switcher">
      <Popover open={open} onOpenChange={onOpenChange}>
        <PopoverTrigger
          aria-label={label}
          className="flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg border bg-fd-secondary/50 px-2.5 py-2 text-sm text-fd-foreground hover:bg-fd-accent hover:text-fd-accent-foreground"
        >
          <span className="flex items-center gap-2">
            <span>{current.label}</span>
            {current.tag && (
              <span className="rounded-md bg-fd-primary/10 px-1.5 py-0.5 text-xs text-fd-primary">
                {current.tag}
              </span>
            )}
          </span>
          <ChevronsUpDown
            size={14}
            aria-hidden="true"
            className="text-fd-muted-foreground"
          />
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-(--radix-popover-trigger-width) min-w-48 p-1 text-sm"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            inputRef.current?.focus();
          }}
        >
          <input
            ref={inputRef}
            type="text"
            role="combobox"
            aria-label={text.search}
            aria-expanded={open}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={activeLink ? optionId(active) : undefined}
            autoComplete="off"
            spellCheck={false}
            placeholder={text.search}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            className="mb-1 w-full rounded-md border bg-fd-background px-2 py-1.5 text-fd-foreground placeholder:text-fd-muted-foreground focus-visible:outline-2 focus-visible:outline-fd-ring"
          />
          <ul
            id={listId}
            role="listbox"
            aria-label={label}
            className="m-0 flex max-h-64 list-none flex-col gap-0.5 overflow-y-auto p-0"
          >
            {matches.map((link, index) => (
              <li
                key={link.id}
                id={optionId(index)}
                role="option"
                aria-selected={index === active}
              >
                <a
                  href={link.href}
                  tabIndex={-1}
                  aria-current={link.current ? "page" : undefined}
                  onMouseMove={() => setActive(index)}
                  className={optionClass(link, index === active)}
                >
                  <span>{link.label}</span>
                  {link.tag && <span className="text-xs">{link.tag}</span>}
                </a>
              </li>
            ))}
          </ul>
          {matches.length === 0 && (
            <p
              role="status"
              className="m-0 px-2 py-3 text-center text-fd-muted-foreground"
            >
              {text.empty}
            </p>
          )}
        </PopoverContent>
      </Popover>
      <noscript>
        <ul className="m-0 mt-1 flex list-none flex-col gap-0.5 p-0 text-sm">
          {links.map((link) => (
            <li key={link.id}>
              <a
                href={link.href}
                aria-current={link.current ? "page" : undefined}
                className={optionClass(link, false)}
              >
                <span>{link.label}</span>
                {link.tag && <span className="text-xs">{link.tag}</span>}
              </a>
            </li>
          ))}
        </ul>
      </noscript>
    </nav>
  );
}

/** Notice above the title of a page of an old line or of the working docs. */
export function DocsBanner({ text, linkText, href }: DocsBannerProps) {
  return (
    <p
      role="note"
      className="mb-4 rounded-lg border border-fd-primary/30 bg-fd-primary/10 px-3 py-2 text-sm text-fd-foreground"
    >
      {text}{" "}
      <a href={href} className="font-medium underline underline-offset-2">
        {linkText}
      </a>
    </p>
  );
}
