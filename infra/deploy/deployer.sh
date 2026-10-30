#!/usr/bin/env bash
# Pull-based deployer for the demo server. Polls the GitHub deployments that
# .github/workflows/deploy.yml records, and for each new one: checks out that sha in a deploy
# clone, pulls its images, restarts what changed, waits for health, and reports the result back
# as a deployment status (visible in the repository's Environments panel).
#
# The server only makes outbound calls (GitHub API, GHCR); it needs no inbound port and GitHub
# holds no credentials for it. Setup and operation: infra/deploy/README.md.
#
# Runs from an installed copy (not from the deploy clone, which it checks out under itself).
#
#   acom-deployer.sh            poll forever
#   acom-deployer.sh --once     handle the newest deployment once and exit
set -euo pipefail

REPO=${ACOM_DEPLOY_REPO:-metahackAI/a-commerce}
ENVIRONMENT=${ACOM_DEPLOY_ENVIRONMENT:-demo}
DIR=${ACOM_DEPLOY_DIR:-$HOME/deploy/a-commerce}        # deploy clone with .env and infra/secrets
STATE=${ACOM_DEPLOY_STATE:-$HOME/deploy/state}
PROJECT=${ACOM_DEPLOY_PROJECT:-acom-demo}               # compose project name
PUBLIC_URL=${ACOM_PUBLIC_URL:-}                         # shown as the environment URL
READY_URL=${ACOM_READY_URL:-http://127.0.0.1:8080/readyz}
INTERVAL=${ACOM_DEPLOY_INTERVAL:-20}
HEALTH_TIMEOUT=${ACOM_DEPLOY_HEALTH_TIMEOUT:-240}

mkdir -p "$STATE/logs"
log() { printf '%s %s\n' "$(date -Is)" "$*"; }

status() { # <deployment id> <state> <description>
  local args=(-f state="$2" -f description="${3:0:140}" -F auto_inactive=true)
  [[ -n $PUBLIC_URL ]] && args+=(-f environment_url="$PUBLIC_URL")
  gh api "repos/$REPO/deployments/$1/statuses" "${args[@]}" --silent || log "could not report $2 for $1"
}

compose() {
  docker compose -p "$PROJECT" -f infra/compose/compose.yaml -f infra/compose/compose.deploy.yaml --env-file .env "$@"
}

healthy() { # every long-running service is healthy and the gateway is ready
  local unhealthy
  unhealthy=$(compose ps --format '{{.Service}} {{.State}} {{.Health}}' \
    | awk '$1 != "migrate" && !($2 == "running" && $3 == "healthy")')
  [[ -z $unhealthy ]] && curl -fsS -m 3 "$READY_URL" >/dev/null
}

deploy() { # <deployment id> <sha>
  local id=$1 sha=$2 out="$STATE/logs/$1.log"
  log "deployment $id: $sha"
  status "$id" in_progress "pulling $sha"
  # A subshell: `cd` and ACOM_TAG stay local. Errors are checked explicitly because errexit is
  # off inside `if deploy ...`.
  (
    cd "$DIR" || exit 1
    git fetch --quiet origin "$sha" || exit 1
    git -c advice.detachedHead=false checkout --quiet --detach "$sha" || exit 1
    export ACOM_TAG=$sha
    compose pull --quiet || { echo "image pull failed for $sha"; exit 1; }
    compose up -d --no-build --remove-orphans || { echo "compose up failed"; exit 1; }
    waited=0
    until healthy; do
      if (( waited >= HEALTH_TIMEOUT )); then compose ps; echo "not healthy after ${HEALTH_TIMEOUT}s"; exit 1; fi
      sleep 5; waited=$((waited + 5))
    done
    compose ps
  ) >"$out" 2>&1
}

handle_newest() {
  local line id sha
  line=$(gh api "repos/$REPO/deployments?environment=$ENVIRONMENT&per_page=1" --jq '.[0] | "\(.id) \(.sha)"' 2>/dev/null) || return 0
  [[ -z $line || $line == "null null" ]] && return 0
  read -r id sha <<<"$line"
  [[ $id == "$(cat "$STATE/last" 2>/dev/null)" ]] && return 0
  if deploy "$id" "$sha"; then
    status "$id" success "running ${sha:0:12}"
    log "deployment $id: success"
  else
    status "$id" failure "$(tail -n 1 "$STATE/logs/$id.log") (server log $STATE/logs/$id.log)"
    log "deployment $id: failure, see $STATE/logs/$id.log"
  fi
  echo "$id" >"$STATE/last"
}

if [[ ${1:-} == --once ]]; then handle_newest; exit 0; fi
log "watching $REPO environment $ENVIRONMENT every ${INTERVAL}s"
while true; do handle_newest; sleep "$INTERVAL"; done
