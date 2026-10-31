# Instructions for coding agents

Read `docs/design.md` before writing code. It defines the tool design rules, domain scope, harness, and UI protocol. Then read `docs/architecture.md` (services and trust boundaries), `docs/backend/` (how the Rust code is organized), and `docs/api/conventions.md` (wire formats).

## Hackathon rules that affect code

- Everything in this repository must be written during the hackathon (Oct 27 – Nov 4, 2026). Do not paste code from other projects.
- The repository is public. Never commit secrets (`META_API_KEY` and others live in `.env`, which is git-ignored).

## Reference material

Team members may have reference material about an existing commerce platform on their machines, outside this repository. Use it only to understand how commerce businesses work (flows, rules, edge cases). Then design and write our own version:

- Do not copy code, SQL, table or column names, API paths, identifiers, sample data, or document text.
- Do not mention the product, its customers, or its internal names in code, comments, commits, or docs.
- Re-express each business rule in our own words and our own names, following `docs/design.md`.

## Tool checklist (every new tool)

- One business verb, `verb_noun` name in plain English
- Typed inputs, word enums, a description and example for every field
- Reads have no side effects and return items with the total
- Writes support `preview=true` and return rule checks and whether approval is needed
- Declares what it consumes and produces
- Errors include a code, a sentence, and a suggested next tool
- Business rules come from the policy file, not from code constants or prompts

## Back end (Rust)

The whole server side is one Cargo workspace: `crates/` (libraries), `services/` (gateway, commerce, agent), `xtask/`. Details: `docs/backend/README.md`.

- An operation exists in `docs/api/` first. Its tool name, address, kind, and REST path in code must match the spec; `cargo xtask check-docs` compares them.
- One operation is one type implementing `ReadOp` or `WriteOp` (`acom-contract`), in its domain crate, with at least one contract test.
- The folders follow the API hierarchy: `domain.resource.operation` lives in `crates/domains/{domain}/src/{resource}/{tool}.rs` (one file per operation, one folder per resource; `docs/backend/README.md` §2.2). `cargo xtask check-layout` checks it; `cargo xtask check-coverage` checks that every specified tool and person-only endpoint is built.
- Input types use `#[serde(deny_unknown_fields)]`, word enums, a doc comment on every field, and no data-carrying enums (they become `oneOf`; use a `mode` word field instead); the registry refuses schemas that are not closed.
- Money is `acom_core::Money`, never a float. Time comes from the `Clock`, never `Utc::now()`. Ids use the prefixes in `docs/api/conventions.md` §9; state words come from `docs/api/enums.md`; error codes and classes from `docs/api/errors.md`.
- Business numbers come from `policies/*.yml` through `acom-policy`, never from constants in code or prompts.
- `core`, `contract`, and `policy` do no I/O. Domain crates call each other only through exported service traits, never each other's tables. The agent crate never depends on a domain crate.
- The agent can preview and draft change sets; it can never approve, run an approved change set, or call a person-only endpoint.
- Before pushing: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask check-docs`, `cargo xtask check-layout`, `cargo xtask check-coverage` (CI runs all of them, with `check-schema` and `check-seed`).

## Database

Rules: `docs/backend/database.md`.

- Tables are schema.org types and columns are schema.org properties, in snake_case. Only concepts schema.org lacks get our own `acom:` names.
- Every table and column gets a `COMMENT` that starts with its term (`schema:…`, `acom:…`, or `-` for technical columns) followed by one English sentence.
- The primary key is the prefixed text id the API uses. Status columns store words under a `CHECK` list taken from `docs/api/enums.md`.
- Money is `price` + `price_currency` or `*_value` + `*_currency`. Quantities are `*_value` + `*_unit_text`. No abbreviations.
- Every store-scoped table has `tenant_id` and a row-level security policy.
- Run `cargo xtask check-schema --write` after changing a migration and commit the generated dictionary.

## Front end (Next.js)

Rules: `docs/frontend-ui.md` (UI library and look) and `docs/frontend/README.md` (AI layer).

- Components come from Meta Astryx (`@astryxdesign/*`). Read `docs/frontend-ui.md` first. Look up components and templates with `npm run astryx -- search "<query>"` before writing your own.
- One UI block from `docs/api/chat.md` §3 is one component, built from Astryx parts. The look lives in our theme (CSS custom properties); no gradients, glows, decorative shadows, colored side bars, or sparkle icons.
- Data types come from `web/src/contract/` (generated, do not edit).
- Every meaningful element (row, price, check, chip, card, input) calls `useAiTarget` so people can ask about it and the agent can point at it.
- Elements that the agent may change also show the block's `suggested_intents`.
- State changes made by a person (approve, place order, send a return request, undo) go through `PersonButton` only. They are never reachable from `ui.command` or page tools.
- The web never computes business numbers. Prices, totals, and stock come from tool results.

## Infrastructure

Serving architecture: `infra/README.md`. Commands: `make help`.

- Every component is a container, and every environment runs the same images and topology; only configuration differs. Don't add a code path that runs only locally; put local conveniences in configuration (`.env`, overlays).
- Configuration comes from environment variables, and secrets come from files or Kubernetes Secrets. Names are the same everywhere (`infra/README.md` §5). Never commit secrets: `infra/secrets/` and `infra/k8s/overlays/local/secrets/` are git-ignored.
- New Rust services use `acom-server`: JSON logs, `/healthz`, `/readyz`, `/metrics`, request ids, graceful shutdown, and the `healthcheck` subcommand.
- Images run as non-root with a read-only root file system. Write only to declared temporary volumes.
- Integration checks (bringing the stack up) run on request. Always run `make check-infra` after changing Compose or Kubernetes files.
