# One entry point for people, coding agents, and CI.
.PHONY: setup check test serve reset

setup:
	cd server && uv sync

test:
	cd server && uv run pytest -q

# Everything that must pass before a PR merges.
check: test

serve:
	cd server && uv run uvicorn a_commerce.api:app --reload --port 8000
