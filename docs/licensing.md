# Licensing

What is under which licence, what ships inside the app, and how to check it.
The short version: the app is MPL-2.0, the plugin API and SDK are
Apache-2.0 OR MIT, the docs are CC-BY-4.0, all bundled third-party code is
permissive, and every bundled asset is original or carries its licence next to
it.

## Repository licence map

| What | Licence | Where it is stated |
| --- | --- | --- |
| App and host-side crates (`crates/`, 24 crates) | `MPL-2.0` | `LICENSE`, each `Cargo.toml` (`license.workspace = true`) |
| Plugin API (`crates/parla_plugin_api`) | `Apache-2.0 OR MIT` | its `Cargo.toml`, `LICENSE-APACHE`, `LICENSE-MIT` |
| Plugin SDK (`plugins/sdk-rust`) | `Apache-2.0 OR MIT` | its `Cargo.toml`, `LICENSE-APACHE`, `LICENSE-MIT` |
| Example plugin (`plugins/examples/hello-command`) | `Apache-2.0 OR MIT` | its `Cargo.toml`, `LICENSE-APACHE`, `LICENSE-MIT` |
| Test-only plugin guests (`plugins/examples/test-guests/*`) | `MPL-2.0` | guest workspace `Cargo.toml` |
| Documentation (`docs/`, `site/content/docs/`, prose files) | `CC-BY-4.0` | `LICENSE-DOCS`, docs index, `NOTICE` |
| Original artwork, themes, keymaps, settings templates, screenshots | `MPL-2.0` | `NOTICE` |
| Incoming huddle chime | `CC0-1.0` | `assets/sounds/README.md`, `NOTICE` |
| Bundled test fonts | `SIL OFL 1.1` | licence text beside each font in `crates/ui_testing/fonts/` |
| Third-party crates and native libraries | permissive licences only | `deny.toml`, `NOTICE`, release `NOTICE` |

Both Cargo workspaces (`/` and `plugins/`) inherit `license = "MPL-2.0"`;
only the three permissive plugin crates override it. No crate is missing a
licence field.

## Bundled assets

| Path | What it is | Origin | Licence | Licence location |
| --- | --- | --- | --- | --- |
| `assets/brand/logo*.svg` | Parla logo | Original | MPL-2.0 | `NOTICE` |
| `assets/icons/app/*.png` | App icons | Generated from the logo | MPL-2.0 | `NOTICE` |
| `assets/icons/tray/*` | Tray icons | Generated from the logo | MPL-2.0 | `NOTICE` |
| `assets/icons/ui/*.svg` | 102 line icons | Original, drawn for Parla | MPL-2.0 | `NOTICE` |
| `assets/themes/*.json` | Built-in themes | Original | MPL-2.0 | `NOTICE` |
| `assets/keymaps/*.json` | Default keymaps | Original, Zed file format | MPL-2.0 | `NOTICE` |
| `assets/settings/*` | Settings templates | Original | MPL-2.0 | `NOTICE` |
| `assets/sounds/parla-chime.wav` | Huddle chime | Synthesised by `scripts/gen_chime.ts` | CC0-1.0 | `assets/sounds/README.md` |
| `crates/ui_testing/fonts/*.ttf` | Inter, JetBrains Mono, Noto Emoji subset | Upstream releases | OFL-1.1 | OFL text beside each family |
| `crates/ui_testing/baselines/**` | 398 app screenshots | Rendered from `FakeProvider` | MPL-2.0 | `NOTICE` |
| `site/public/favicon.svg` | Website icon | Parla logo | MPL-2.0 | `NOTICE` |
| `site/public/shots/*.webp` | Website screenshots | Rendered from the baselines | MPL-2.0 | `NOTICE` |
| `crates/domain/src/test_support/{solved.png,party.gif}` | Test emoji | Generated 32 px images | MPL-2.0 | `NOTICE` |
| `crates/providers/*/tests/fixtures/**` | 259 API samples | Synthetic or redacted; no real accounts | MPL-2.0 | `NOTICE` |

No Slack artwork, sound, font, icon, colour or screenshot is bundled. The icon
and chime provenance is asserted by the commits that introduced them (the icon
set and the chime generator); the licensing check in `scripts/licensing.test.ts`
fails on a Slack path or name under `assets/` or `site/public/`.

## Rust dependencies

`cargo deny check` passes `advisories`, `bans`, `licenses` and `sources` for
the root workspace and for the `plugins/` workspace (the latter reads the root
`deny.toml`, which cargo-deny finds by walking up).

The root graph has 1,110 packages: 24 local crates and 1,086 from crates.io.
Every licence is in `deny.toml`'s allow list; no allow entry is unused by the
root graph.

| Licence family | Packages | Notes |
| --- | --- | --- |
| MIT / Apache-2.0 | 923 only, 1,003 include | The bulk of the graph |
| BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0 | 35 only, 92 include | Permissive |
| Unicode-3.0 | 44 | ICU4X, `unicode-ident`, `unicode-properties`, `emojis`; attribution in `NOTICE` |
| MPL-2.0 | 7 external + 23 local | `cbindgen`, `cssparser`, `cssparser-macros`, `dtoa-short`, `dwrote`, `option-ext`, `selectors`; unmodified, source on crates.io |
| Unlicense | 14 | `Unlicense OR MIT` in practice |
| CC0-1.0 / MIT-0 / CDLA-Permissive-2.0 | 3 / 2 / 1 | `dunce`, `notify`, `aws-lc-sys`, `webpki-root-certs`; CDLA text in `NOTICE` |
| WTFPL, blessing | 1 | SQLCipher's bundled libtomcrypt only |

Two packages offer a copyleft option, and the permissive branch is used:
`self_cell` is `Apache-2.0 OR GPL-2.0-only`, and `r-efi` is
`MIT OR Apache-2.0 OR LGPL-2.1-or-later`. There is no GPL-only, AGPL or SSPL
dependency.

The app statically links four native libraries. Their versions are pinned by
the lockfile and their licences are recorded in `NOTICE`; `cargo about`
includes the licence texts in the bundled `NOTICE`.

| Native library | Crate | Licence |
| --- | --- | --- |
| AWS-LC (BoringSSL/OpenSSL heritage) | `aws-lc-sys` 0.47.0 | ISC and Apache-2.0 |
| OpenSSL 3.6.3 | `openssl-src` 300.6.1+3.6.3 | Apache-2.0 |
| SQLCipher 4.20.0, SQLite 3.54.1, libtomcrypt 1.18.2 | `sqlcipher-src` 420.0.0 | BSD-3-Clause, public domain, WTFPL/blessing |
| libopus 1.6.1 | `opusic-sys` 0.7.7 | BSD-3-Clause plus its patent licences |

Vendored source is covered by its own notice: the Chime signalling protobuf
in `crates/chime_signal` (Apache-2.0, `crates/chime_signal/NOTICE`) and the
gpui-ce text system in `crates/ui_testing` (Apache-2.0,
`crates/ui_testing/NOTICE`).

## JavaScript dependencies

`site/` has 768 installed packages (`bun install --frozen-lockfile`). All are
permissive except the four build-only cases below, none of which is shipped in
the site output.

| Licence | Packages | Notes |
| --- | --- | --- |
| MIT | 641 | Includes Astro, React, Fumadocs, Tailwind, Turbo, TypeScript |
| ISC | 40 | Includes Lucide icons, which do appear in the built site |
| Apache-2.0 | 27 | Includes Lighthouse |
| BSD-2-Clause / BSD-3-Clause | 32 | |
| BlueOak-1.0.0 | 12 | |
| MPL-2.0 | 4 | `axe-core`, `lightningcss`, its darwin binary, `satori` |
| OFL-1.1 | 3 | `@fontsource/inter`, `@fontsource-variable/inter`, `@fontsource-variable/jetbrains-mono`; the built site serves these fonts |
| CC0-1.0, 0BSD, PSF-2.0, MIT OR CC0-1.0 | 7 | Permissive |
| FSL-1.1-Apache-2.0 | 1 | `sentry` CLI, dev-only through `@lhci/cli` -> Lighthouse; never run by the site build |
| LGPL-3.0-or-later | 1 | `@img/sharp-libvips-darwin-arm64`, build-only through `sharp`; not in the output |

The website's third-party content is therefore Inter and JetBrains Mono
(OFL-1.1) and Lucide icons (ISC), both named in `NOTICE`.

## How to check

```sh
# Rust: policy for both workspaces, then the licence inventory
mise exec -- cargo deny check
mise exec -- cargo deny list
(cd plugins && mise exec -- cargo deny check && mise exec -- cargo deny list)

# The bundled NOTICE: project notice plus generated crate licences
mise run release:notice        # writes target/release-assets/NOTICE

# JavaScript: install exactly the lockfile, then read every package.json
(cd site && bun install --frozen-lockfile)
# Then read `license` from each site/node_modules/**/package.json; a package
# without a permissive licence is a finding, not a silent pass.

# Repo guard: licence files, NOTICE coverage, asset provenance, Cargo fields
bun test scripts/licensing.test.ts
```

`deny.toml` and `about.toml` must stay in step: a new licence in the graph is
added to both (and documented here), and an allow entry no crate uses any more
is removed. `cargo deny check` warns about unused allowances.
