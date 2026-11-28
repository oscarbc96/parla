# Bundled test fonts

Test-only, never packaged. JetBrains Mono and Noto Emoji are SIL OFL 1.1.

- `JetBrainsMono-Regular.ttf`: unmodified.
- `NotoEmoji-Regular-subset.ttf`: Noto Emoji (monochrome), instanced at weight 400 and cut
  down with `pyftsubset` to the emoji the visual suites draw (smileys, a few gestures,
  skin-tone swatches, lock, pencil). U+270E (the sidebar's draft mark) is mapped to the
  pencil glyph U+270F so it does not render as a missing-glyph box. Add code points here
  when a scenario needs more.
- Inter (Regular, Italic, Medium, SemiBold, Bold) comes from `assets/fonts`, shared with
  the app; `themes::fonts` embeds it and this crate re-exports the faces.
