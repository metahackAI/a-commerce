# One entry point for people, coding agents, and CI.
.PHONY: setup check test web-build serve web try-model

setup:
	cd server && uv sync
	cd web && npm ci

test:
	cd server && uv run pytest -q

web-build:
	cd web && npm run build

# Everything that must pass before a PR merges.
check: test web-build

serve:
	cd server && uv run uvicorn a_commerce.api:app --reload --port 8000

# Back office at http://localhost:5173 (needs make serve running).
web:
	cd web && npm run dev

# Run the demo request through the model in .env and judge the change set (exit 0 = demo works).
try-model:
	cd server && set -a && [ -f ../.env ] && . ../.env; set +a; uv run python -m a_commerce.harness.acceptance
