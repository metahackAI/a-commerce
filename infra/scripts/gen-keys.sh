#!/usr/bin/env bash
# Generates the gateway's Ed25519 key pair for local runs (Compose and the local Kubernetes
# overlay). The gateway signs internal tokens with the private key; commerce and agent verify
# them with the public key. Existing keys are kept unless --force is given.
set -euo pipefail

dir="$(cd "$(dirname "$0")/../secrets" && pwd)"
private="$dir/gateway_signing_key.pem"
public="$dir/gateway_public_key.pem"

if [[ -f "$private" && "${1:-}" != "--force" ]]; then
  echo "keys already exist in infra/secrets (use --force to replace them)"
  exit 0
fi

command -v openssl >/dev/null || { echo "openssl is required" >&2; exit 1; }
umask 077
openssl genpkey -algorithm ed25519 -out "$private"
openssl pkey -in "$private" -pubout -out "$public"
# Containers run as non-root users; make the files readable inside them.
chmod 644 "$private" "$public"
echo "wrote infra/secrets/gateway_signing_key.pem and gateway_public_key.pem"
