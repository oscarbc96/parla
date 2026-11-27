#!/bin/sh
# Sourced by mise (mise.toml [env] _.source). Opt-in compile cache: with
# PARLA_SCCACHE=1 and sccache on PATH, cargo uses it as the rustc wrapper.
# Otherwise nothing changes. See CONTRIBUTING.md.
if [ "${PARLA_SCCACHE:-}" = "1" ] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
fi
