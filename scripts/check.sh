#!/usr/bin/env sh
# Everything that must pass before a PR merges. `make check` and CI both run this file.
set -eu
cd "$(dirname "$0")/.."
(cd server && uv run pytest -q)
(cd web && npm run build)
(cd web && npx playwright test)
