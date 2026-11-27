# Sounds

`parla-chime.wav` is the ringtone Parla plays for an incoming huddle. It is a
two-note chime synthesised from sine waves by `scripts/gen_chime.ts`, so it is
original work released under CC0 1.0. Regenerate it with `mise run
assets:chime`; the output is deterministic. The file is compiled into the
binary (`include_bytes!`), not read from disk.
