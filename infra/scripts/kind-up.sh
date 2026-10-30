#!/usr/bin/env bash
# Local Kubernetes with kind: the same images and manifests as production, through the local
# overlay (infra/k8s/overlays/local). Needs docker, kind, kubectl.
#
#   infra/scripts/kind-up.sh        (or: make kind-up)
#   kubectl -n a-commerce port-forward svc/gateway 8080:7000   → http://localhost:8080
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cluster="${KIND_CLUSTER:-acom}"
namespace="a-commerce"
secrets="$root/infra/k8s/overlays/local/secrets"

for tool in docker kind kubectl; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done

if ! kind get clusters 2>/dev/null | grep -qx "$cluster"; then
  kind create cluster --name "$cluster" --config "$root/infra/k8s/kind/cluster.yaml"
fi
kubectl config use-context "kind-$cluster" >/dev/null

echo "building images"
make -C "$root" images TAG=local
for image in gateway commerce agent web; do
  kind load docker-image "acom/acom-$image:local" --name "$cluster"
done

echo "preparing local secrets (git-ignored)"
"$root/infra/scripts/gen-keys.sh"
for f in database models postgres; do
  [[ -f "$secrets/$f.env" ]] || cp "$secrets/$f.env.example" "$secrets/$f.env"
done
cp "$root/infra/secrets/gateway_signing_key.pem" "$root/infra/secrets/gateway_public_key.pem" "$secrets/"

echo "applying the local overlay"
kubectl apply -k "$root/infra/k8s/overlays/local"
kubectl -n "$namespace" rollout status statefulset/postgres --timeout=180s
kubectl -n "$namespace" wait --for=condition=complete job/migrate --timeout=300s
for d in commerce commerce-worker agent web gateway; do
  kubectl -n "$namespace" rollout status "deployment/$d" --timeout=180s
done

echo
echo "ready. open the store with:"
echo "  kubectl -n $namespace port-forward svc/gateway 8080:7000"
echo "  → http://localhost:8080  (storefront /, back office /bo)"
