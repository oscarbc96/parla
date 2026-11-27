import mark from "../../../assets/brand/logo.svg?raw";

/**
 * The Parla mark from `assets/brand/logo.svg`, inlined so CSS can colour it:
 * `.parla-logo .bubble { fill: var(--parla-logo-bubble) }` and so on (see
 * `styles/global.css`, which follows the site's light/dark/System switch).
 * Without the custom properties the SVG's own light defaults apply.
 */
export const logoMarkup = mark
  .replace(/<title>.*?<\/title>\s*/s, "")
  .replace(/<!--.*?-->\s*/gs, "")
  .replace(
    /<svg [^>]*>/,
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" class="parla-logo" aria-hidden="true" focusable="false">',
  );

/** The mark with the dark appearance colours, as a data URI (social images). */
export const logoDarkDataUri = `data:image/svg+xml;utf8,${encodeURIComponent(
  logoMarkup
    .replace("#211E4C", "#211F47")
    .replace('stroke="none"', 'stroke="#3A3A78"')
    .replace("#FFA611", "#FEA604")
    .replace("#FE6129", "#F85E22")
    .replace("#FD3459", "#FD2E57"),
)}`;
