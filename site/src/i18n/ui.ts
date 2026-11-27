/** Shape of the UI strings every locale provides. */
export interface Ui {
  nav: {
    main: string;
    docs: string;
    changelog: string;
    download: string;
    github: string;
  };
  footer: {
    license: string;
    source: string;
    privacy: string;
    llms: string;
    /** Colophon line: what the site itself is built with. */
    colophon: string;
  };
  docsTitle: string;
  language: string;
  home: {
    title: string;
    description: string;
    kicker: string;
    heading: string;
    headingAccent: string;
    lede: string;
    docsButton: string;
    sourceButton: string;
    /** Short status label next to the hero buttons ("In development"). */
    status: string;
    /** One line under the status label, about the first release. */
    statusText: string;
    /** Link text of `statusText`, pointing at the install guide. */
    statusLink: string;
    /** Platforms and what to do on the ones without an installer. */
    platform: string;
    /** Line under the hero screenshot. */
    heroCaption: string;
    featuresLabel: string;
    /** Heading of the feature grid. */
    featuresHeading: string;
    /**
     * Cards of the landing page grid, in the same order as `cards` in
     * `components/Home.astro` (which holds the icon and the link). A card
     * without `linkText` has no link.
     */
    features: { title: string; text: string; linkText?: string }[];
    /** Theme section: `themesTextBefore`, the `themes/` code span, then `themesTextAfter`. */
    themesLabel: string;
    themesTitle: string;
    themesTextBefore: string;
    themesTextAfter: string;
    themesLight: string;
    themesDark: string;
    themesDefault: string;
    themesPresets: string;
    themesFile: string;
    themesLink: string;
    /** The dark band: plugins, the local MCP server and the keymap. */
    extLabel: string;
    extTitle: string;
    extText: string;
    extPluginsTitle: string;
    extPluginsText: string;
    extMcpTitle: string;
    extMcpText: string;
    extKeymapTitle: string;
    extKeymapText: string;
    /** Heading of the closing card (the design's “Try Parla on your Mac.”). */
    downloadHeading: string;
    downloadText: string;
    buildButton: string;
    releasesButton: string;
    changelogButton: string;
    allDownloads: string;
    /** Alt text of the hero screenshot (a thread beside its conversation). */
    heroAlt: string;
    galleryLabel: string;
    gallery: { shot: string; title: string; text: string; alt: string }[];
    connectLabel: string;
    /** Big heading; the caps label above it says “How it connects”. */
    connectHeading: string;
    connectText: string;
    connectBullet1: string;
    connectBullet2: string;
    connectBullet3: string;
    connectSignIn: string;
    connectPrivacy: string;
  };
  changelog: {
    title: string;
    description: string;
    heading: string;
    /** Release notes are always English. */
    notesNote: string;
    empty: string;
    emptyText: string;
    prerelease: string;
    unreleased: {
      heading: string;
      badge: string;
      /** `{version}` is replaced by the proposed version. */
      version: string;
      note: string;
    };
    newer: string;
    older: string;
    /** BCP 47 tag used to format dates. */
    dateLocale: string;
  };
  downloads: {
    title: string;
    description: string;
    heading: string;
    /** Line under the page heading. */
    lede: string;
    macTitle: string;
    downloadMac: string;
    macRequirements: string;
    /** Tag next to the macOS version. */
    latest: string;
    /** Badge for the self-signed build. */
    signed: string;
    /** Label of the checksum field. */
    shaLabel: string;
    /** Copy button of the checksum field. */
    copy: string;
    /** Shown on the copy button for a moment after a copy. */
    copied: string;
    checksumLink: string;
    attestationLabel: string;
    installTitle: string;
    installStep1: { title: string; text: string };
    installStep2: { title: string; text: string };
    installStep3: { title: string; text: string };
    installStep4: { title: string; text: string };
    installLink: string;
    otherPlatformsTitle: string;
    otherPlatformsText: string;
    /** Tag next to Windows and Linux. */
    later: string;
    noRelease: string;
    noReleaseText: string;
    releasesLink: string;
    buildTitle: string;
    buildText: string;
    buildLink: string;
    /** `{version}` is the latest version. */
    whatsNew: string;
    notesText: string;
    fullChangelog: string;
    olderTitle: string;
    tableVersion: string;
    tableDate: string;
    tableDownload: string;
    tableChecksum: string;
    allReleases: string;
  };
  notFound: {
    title: string;
    description: string;
    heading: string;
    text: string;
    home: string;
  };
  /** Docs version switcher and banners; only shown when snapshots exist. */
  docsVersions: {
    /** Accessible label of the version switcher. */
    label: string;
    /** Placeholder and accessible name of the version search box. */
    search: string;
    /** Shown when no version matches the search. */
    noMatch: string;
    /** Tag next to the newest release line. */
    latest: string;
    /** Accessible label of the version tag in the search dialog; `{version}` is its label. */
    searching: string;
    /** Label of the working docs. */
    unreleased: string;
    /** Banner on an old line; `{version}` is `X.Y`. */
    oldText: string;
    /** Link text of that banner, pointing at the latest version. */
    oldLink: string;
    /** Banner on the working docs. */
    nextText: string;
    /** Link text of that banner, pointing at the latest release. */
    nextLink: string;
  };
  /** `{count}` is the compact star count. */
  starsLabel: string;
  /** Fumadocs UI strings, keyed by their English text. */
  translations: Record<string, string>;
}
