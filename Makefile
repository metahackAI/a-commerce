# One entry point for people, coding agents, and CI.
.PHONY: setup check test web-build e2e serve web demo try-model compare

setup:
	cd server && uv sync
	cd web && npm ci && npx playwright install chromium

test:
	cd server && uv run pytest -q

web-build:
	cd web && npm run build

# The demo in a real browser (Playwright starts its own API and web server).
e2e:
	cd web && npx playwright test

# Everything that must pass before a PR merges (same script as CI).
check:
	./scripts/check.sh

serve:
	cd server && uv run uvicorn a_commerce.api:app --reload --port 8000

# Back office at http://localhost:5173 (needs make serve running).
web:
	cd web && npm run dev

# Both at once: API on :8000, app on http://localhost:5173 (Ctrl-C stops both).
demo:
	$(MAKE) -j2 serve web

# Run the demo request through the model in .env and judge the change set (exit 0 = demo works).
try-model:
	cd server && set -a && [ -f ../.env ] && . ../.env; set +a; uv run python -m a_commerce.harness.acceptance --runs $(or $(RUNS),1)

# Same model, same tasks: A-Commerce tools vs the screen-shaped facade (docs/design.md §7).
compare:
	cd server && set -a && [ -f ../.env ] && . ../.env; set +a; uv run python -m a_commerce.harness.compare --runs $(or $(RUNS),1)
