# infra/secrets

Local secrets for Docker Compose and the local Kubernetes overlay. Files here are git-ignored.

`make keys` (infra/scripts/gen-keys.sh) writes:

- `gateway_signing_key.pem`: Ed25519 private key; only the gateway signs internal tokens with it
- `gateway_public_key.pem`: the matching public key; commerce and agent verify tokens with it

Production keys come from the platform's secret store, never from this folder.
