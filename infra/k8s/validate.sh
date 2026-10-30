#!/usr/bin/env bash
# Render every overlay and check the result against the Kubernetes API schemas. No cluster needed.
#
#   infra/k8s/validate.sh
#   KUSTOMIZE=/path/to/kustomize KUBECONFORM=/path/to/kubeconform infra/k8s/validate.sh
#
# Environment:
#   KUSTOMIZE    kustomize v5 binary            (default: kustomize on PATH)
#   KUBECONFORM  kubeconform binary             (default: kubeconform on PATH)
#   K8S_VERSION  schema version to check against (default: 1.31.0)
#   CRD_SCHEMAS  extra kubeconform schema location for CRDs such as ServiceMonitor
#                (default: the public CRDs catalog; set to "" to skip CRDs, offline for example)
#   OUT_DIR      if set, the rendered YAML of each target is written there
#
# The renders run on a temporary copy of infra/k8s and infra/postgres in which the git-ignored
# local secret files are filled from the *.example files (and placeholder PEM files), so this
# works on a fresh clone and never reads the real local secrets.
set -euo pipefail

KUSTOMIZE="${KUSTOMIZE:-kustomize}"
KUBECONFORM="${KUBECONFORM:-kubeconform}"
K8S_VERSION="${K8S_VERSION:-1.31.0}"
crd_catalog='https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{.Group}}/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json'
CRD_SCHEMAS="${CRD_SCHEMAS-$crd_catalog}"
OUT_DIR="${OUT_DIR:-}"

for tool in "$KUSTOMIZE" "$KUBECONFORM"; do
  command -v "$tool" >/dev/null 2>&1 || { echo "validate.sh: '$tool' not found (set KUSTOMIZE / KUBECONFORM)" >&2; exit 2; }
done

k8s_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
infra_dir="$(dirname "$k8s_dir")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Copy the manifests, leaving out whatever is in the local secrets folder.
mkdir -p "$work/infra"
tar -C "$infra_dir" --exclude='k8s/overlays/local/secrets/*' -cf - k8s \
  | tar -C "$work/infra" -xf -
mkdir -p "$work/infra/postgres"
cp "$infra_dir/postgres/kustomization.yaml" "$infra_dir/postgres/postgres.yaml" "$work/infra/postgres/"
if [[ -f "$infra_dir/postgres/init.sql" ]]; then
  cp "$infra_dir/postgres/init.sql" "$work/infra/postgres/"
else
  echo "validate.sh: infra/postgres/init.sql is missing; using an empty placeholder" >&2
  echo "-- placeholder for validation" > "$work/infra/postgres/init.sql"
fi

# Local secret files from the examples.
secrets="$work/infra/k8s/overlays/local/secrets"
for example in "$k8s_dir"/overlays/local/secrets/*.example; do
  cp "$example" "$secrets/$(basename "$example" .example)"
done
for pem in gateway_signing_key.pem gateway_public_key.pem; do
  printf 'placeholder for validation only\n' > "$secrets/$pem"
done

# Local overlay plus the optional vLLM component.
mkdir -p "$work/infra/k8s/overlays/local-models-vllm"
cat > "$work/infra/k8s/overlays/local-models-vllm/kustomization.yaml" <<'EOF'
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: a-commerce
resources:
  - ../local
components:
  - ../../components/models-vllm
EOF

# Production with the Gateway API in place of the Ingress.
mkdir -p "$work/infra/k8s/overlays/production-gateway-api"
cat > "$work/infra/k8s/overlays/production-gateway-api/kustomization.yaml" <<'EOF'
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: a-commerce
resources:
  - ../production
components:
  - ../../components/gateway-api
patches:
  - patch: |-
      $patch: delete
      apiVersion: networking.k8s.io/v1
      kind: Ingress
      metadata:
        name: a-commerce
EOF

conform_args=(-strict -summary -ignore-missing-schemas -kubernetes-version "$K8S_VERSION" -schema-location default)
if [[ -n "$CRD_SCHEMAS" ]]; then
  conform_args+=(-schema-location "$CRD_SCHEMAS")
fi

mkdir -p "$work/rendered"
status=0
check() {
  local name="$1" dir="$2" out="$work/rendered/$1.yaml"
  echo "== $name"
  if ! "$KUSTOMIZE" build "$dir" > "$out"; then
    echo "   kustomize build failed" >&2
    status=1
    return
  fi
  echo "   $(grep -c '^kind:' "$out") objects:" \
    "$(grep '^kind:' "$out" | sort | uniq -c | awk '{printf "%s %s, ", $3, $1}' | sed 's/, $//')"
  if ! "$KUBECONFORM" "${conform_args[@]}" "$out"; then
    status=1
  fi
  if [[ -n "$OUT_DIR" ]]; then
    mkdir -p "$OUT_DIR" && cp "$out" "$OUT_DIR/$name.yaml"
  fi
}

overlays="$work/infra/k8s/overlays"
check base "$work/infra/k8s/base"
check local "$overlays/local"
check production "$overlays/production"
check local-models-vllm "$overlays/local-models-vllm"
check production-gateway-api "$overlays/production-gateway-api"

echo "== secrets.example.yaml"
"$KUBECONFORM" "${conform_args[@]}" "$k8s_dir/secrets.example.yaml" || status=1

if [[ $status -eq 0 ]]; then echo "OK"; else echo "FAILED" >&2; fi
exit $status
