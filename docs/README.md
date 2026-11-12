# Documents

[README](../README.md)

The map of `docs/`. Each folder's README lists what is under it, so you can walk from here to every file. Specifications that implementers read daily are in Korean; overview documents are in English.

## Product and system

| Document | What it settles |
|---|---|
| [design.md](design.md) | Principles, tool rules, domain scope, harness, UI protocol, results. Read this first. |
| [research.md](research.md) | What AI-native commerce means, who else is building it, where A-Commerce fits |
| [architecture.md](architecture.md) | Services and trust boundaries, API hierarchy, schema.org data model, AI-native features |
| [scenarios.md](scenarios.md) | The demo store: people, products, policies, each scene with expected results |
| [catalog-taxonomy.md](catalog-taxonomy.md) | Draft: product taxonomy and attributes for several verticals, back-office entry, showcase scenes, data generation |
| [plan.md](plan.md) | Scope, milestones, status, open work |
| [serving.md](serving.md) | Glimmer's image and flags, speed, the RTX 5090 crash and its fix |

## Specifications

| Index | What is under it |
|---|---|
| [api/README.md](api/README.md) | Every operation: hierarchy, kinds, the index. Below it: conventions, vocabulary, enums, errors, data model, the chat stream and blocks, and one file per domain |
| [backend/README.md](backend/README.md) | How the Rust code is organized. Below it: database, the generated column dictionary, commerce, agent, gateway, testing |
| [frontend/README.md](frontend/README.md) | The AI layer of the web. Below it: the UI library and look, the theme, the Astryx guide for coding agents |

## Outside this folder

| Index | What is under it |
|---|---|
| [agents/README.md](../agents/README.md) | Agent definitions, their prompts, the step prompts, models and runtime settings |
| [policies/README.md](../policies/README.md) | The store's rules as data |
| [seed/demo/README.md](../seed/demo/README.md) | The demo store's data and how each number in the scenes maps to rows |
| [seed/mall/README.md](../seed/mall/README.md) | Draft: a second demo store, a fashion mall of 30 brands and 5,000 products, with tiers, brands and garment types |
| [eval/README.md](../eval/README.md) | Task sets, how to run them, and every result |
| [infra/README.md](../infra/README.md) | Containers, Compose, Kubernetes, the demo server, local secrets |
| [design/README.md](../design/README.md) | Mockups, layout options, the storefront plan, the demo storyboard |

## How these files link

[README.md](../README.md) is the root, and every markdown file can be reached from it by following links.

- A folder's README lists every markdown file under it, with one line on what the file is for.
- A file names its way up on the line under its title, from the root to its parent.
- A path to another document is a link, so a reader can follow it, not plain text.
- Three kinds of file carry no way up, and their parent still lists them: model prompts in `agents/`, which are read into the prompt as written; generated files ([backend/schema.generated.md](backend/schema.generated.md) and the run reports in `eval/results/`); and [CLAUDE.md](../CLAUDE.md), which only imports [AGENTS.md](../AGENTS.md).

A new markdown file goes into its parent's list and gets the line under its title in the same change.
