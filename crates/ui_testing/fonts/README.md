# Bundled test fonts

Test-only, never packaged. All SIL OFL 1.1.

- `Inter-Regular.ttf`, `Inter-Italic.ttf` (both from the Inter 4.1 release, `extras/ttf`), `JetBrainsMono-Regular.ttf`: unmodified.
- `NotoEmoji-Regular-subset.ttf`: Noto Emoji (monochrome), instanced at weight 400 and cut
  down with `pyftsubset` to the emoji the visual suites draw (smileys, a few gestures,
  skin-tone swatches, lock, pencil). U+270E (the sidebar's draft mark) is mapped to the
  pencil glyph U+270F so it does not render as a missing-glyph box. Add code points here
  when a scenario needs more.
