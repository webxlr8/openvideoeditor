#!/usr/bin/env bash
# Type-checks the golden projection corpus against the generated typings (timeline-projection task 2.2).
# Usage: tools/check-projection-types.sh [path/to/tsc]   (default: npx tsc)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tsc="${1:-npx --yes -p typescript@5 tsc}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp "$root/docs/spec/timeline.d.ts" "$work/timeline.d.ts"
status=0
for f in "$root"/crates/ove-projection/tests/corpus/valid/*.tsx; do
  cp "$f" "$work/timeline.tsx"
  printf '{"compilerOptions":{"jsx":"preserve","strict":true,"noEmit":true,"types":[],"lib":["es2022"]},"files":["timeline.d.ts","timeline.tsx"]}\n' > "$work/tsconfig.json"
  if (cd "$work" && $tsc -p tsconfig.json); then echo "ok   $(basename "$f")"; else echo "FAIL $(basename "$f")"; status=1; fi
done
exit $status
