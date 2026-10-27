# One entry point for people, coding agents, and CI.
.PHONY: setup check test serve try-model

setup:
	cd server && uv sync

test:
	cd server && uv run pytest -q

# Everything that must pass before a PR merges.
check: test

serve:
	cd server && uv run uvicorn a_commerce.api:app --reload --port 8000

# Run the demo request through the model in .env and judge the change set (exit 0 = demo works).
try-model:
	cd server && set -a && [ -f ../.env ] && . ../.env; set +a; uv run python -m a_commerce.harness.acceptance
