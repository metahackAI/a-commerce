#!/usr/bin/env bash
# Deletes the local kind cluster created by kind-up.sh.
set -euo pipefail
kind delete cluster --name "${KIND_CLUSTER:-acom}"
