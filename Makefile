# A-Commerce: one place for the commands people and coding agents run.
# `make help` lists them. Serving architecture: infra/README.md.

SHELL := /bin/bash
.DEFAULT_GOAL := help

COMPOSE     := docker compose -f infra/compose/compose.yaml --env-file .env
COMPOSE_DBG := $(COMPOSE) -f infra/compose/compose.debug.yaml
TAG         ?= local
IMAGES      := gateway commerce agent

.PHONY: help env keys up up-debug up-models up-observability down clean ps logs migrate reseed \
        images image-web check check-rust check-web check-infra k8s-render kind-up kind-down

help: ## List the commands
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[1m%-18s\033[0m %s\n", $$1, $$2}'

# --- setup --------------------------------------------------------------------------------------

env: ## Create .env from .env.example (keeps an existing one)
	@test -f .env && echo ".env exists" || (cp .env.example .env && echo "wrote .env")

keys: ## Generate the gateway's local signing keys into infra/secrets/
	@infra/scripts/gen-keys.sh

# --- Docker Compose (same topology as production) -----------------------------------------------

up: env keys ## Build images and start everything at http://localhost:8080
	$(COMPOSE) up -d --build
	@echo "→ http://localhost:$${ACOM_HTTP_PORT:-8080}  (storefront /, back office /bo)"

up-debug: env keys ## Like `up`, plus Postgres and each service on its own port, readable logs
	$(COMPOSE_DBG) up -d --build

up-models: env keys ## Also run the worker model on a local GPU (profile `models`)
	$(COMPOSE) --profile models up -d --build

up-observability: env keys ## Also run Prometheus (:9090) and Grafana (:3001)
	$(COMPOSE) --profile observability up -d --build

down: ## Stop everything (keeps the database volume)
	$(COMPOSE) --profile models --profile observability down

clean: ## Stop everything and delete volumes (database, model cache)
	$(COMPOSE) --profile models --profile observability down -v

ps: ## Show containers and health
	$(COMPOSE) ps

logs: ## Follow logs (SERVICE=commerce for one service)
	$(COMPOSE) logs -f $(SERVICE)

migrate: ## Run migrations again (the stack runs them on start)
	$(COMPOSE) run --rm migrate

reseed: ## Empty the store and load the demo store again (admin / admin123)
	$(COMPOSE) run --rm migrate seed --reset

# --- images -------------------------------------------------------------------------------------

images: image-web ## Build every image (acom/acom-*:$(TAG))
	@for t in $(IMAGES); do \
	  docker build -f infra/docker/rust.Dockerfile --target $$t -t acom/acom-$$t:$(TAG) . || exit 1; \
	done

image-web: ## Build the web image
	docker build -f infra/docker/web.Dockerfile -t acom/acom-web:$(TAG) .

# --- checks (no database, no running services) --------------------------------------------------

check: check-rust check-web check-infra ## Everything CI runs

check-rust: ## fmt, clippy, tests, docs/schema/seed/layout/coverage checks (as CI)
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	cargo xtask check-docs
	cargo xtask check-schema
	cargo xtask check-seed
	cargo xtask check-layout
	cargo xtask check-coverage

check-web: ## Type check and build the web app
	cd web && npm ci --no-audit --no-fund && npm run typecheck && npm run build

check-infra: ## Validate Compose files and render + validate Kubernetes manifests
	docker compose -f infra/compose/compose.yaml -f infra/compose/compose.debug.yaml --env-file .env.example \
	  --profile models --profile observability config --quiet
	infra/k8s/validate.sh

# --- Kubernetes ---------------------------------------------------------------------------------

k8s-render: ## Print the local overlay (OVERLAY=production for the other)
	kubectl kustomize infra/k8s/overlays/$(or $(OVERLAY),local)

kind-up: keys ## Local Kubernetes: kind cluster, images, the local overlay (then port-forward svc/gateway → http://localhost:8080)
	infra/scripts/kind-up.sh

kind-down: ## Delete the local kind cluster
	infra/scripts/kind-down.sh
