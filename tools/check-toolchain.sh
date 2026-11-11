#!/usr/bin/env bash
# Verify the local toolchain matches the pinned versions (task 1.1): Rust from rust-toolchain.toml, Node from
# .node-version, pnpm from .pnpm-version. Exit 1 listing every mismatch.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
want_rust="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
got_rustc="$(rustc --version | cut -d' ' -f2)"
got_cargo="$(cargo --version | cut -d' ' -f2)"
want_node="$(tr -d ' \n' < .node-version)"
got_node="$(node --version 2>/dev/null | tr -d 'v' || echo missing)"
want_pnpm="$(tr -d ' \n' < .pnpm-version)"
got_pnpm="$(pnpm --version 2>/dev/null || echo missing)"

check() { # name want got
  if [ "$2" = "$3" ]; then echo "ok    $1 $3"; else echo "FAIL  $1: want $2, got $3"; fail=1; fi
}
check rustc "$want_rust" "$got_rustc"
check cargo "$want_rust" "$got_cargo"
check node "$want_node" "$got_node"
check pnpm "$want_pnpm" "$got_pnpm"
exit $fail
