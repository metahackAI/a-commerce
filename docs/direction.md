# Direction

Status: Oct 30. What the team has decided, what is still only proposed, and how a change gets into this repository. The judge agent ([.claude/agents/judge.md](../.claude/agents/judge.md)) checks every change against this file. Where [plan.md](plan.md) or the README disagree with this file, this file wins. Change it by pull request.

## Decided

1. **What we prove.** Business APIs redesigned for agents (A2API) let a small open-weight model finish real back-office work that it fails on a screen-shaped API, and every write stays reviewable by a person. [a2api](https://github.com/metahackAI/a2api) is the protocol and runtime. a-commerce is the first proof, in commerce, and the repository we submit.
2. **Who does what.**

   | Chunk | People | Work |
   |---|---|---|
   | 1. Real model + harness | Seongjun, Donguk | Review and merge the stacked pull requests. Get Glimmer and Spark Open access, serve Glimmer. Run the back-office and storefront flows on the real model and fix what fails. |
   | 2. a2api library + backend connection | Haesoo, Haryeom | Connect a2api to the a-commerce server. The legacy-API conversion demo. Write support in a2api: preview, change set, person approval. |
   | 4. Front end, desktop, demo | Seongjun | Polish screens, Tauri desktop wrapper, live demo deploy, video, README, X post, submission. |

   Chunk 3 has no owner yet.
3. **Model.** Muse Glimmer as `nvidia/Muse-Glimmer-30B-NVFP4`, served with vLLM on the team's RTX 5090 and reached through an OpenAI-compatible API. Muse Spark Open on Baseten (`meta-models/muse-spark-open-1026`) when keys arrive.
4. **Prize eligibility.** Answer from Cerebral Valley in Discord `#office-hours`, Oct 30: a quantized Muse Glimmer served on our own hardware counts for Best Build on Meta Open Weight Models. The demo and README must say how and where the model is used, and we keep evidence that judges can check without our GPU: run logs with the model name and quantization, screen captures, and sample outputs.
5. **Rules** from [AGENTS.md](../AGENTS.md): only code written during the hackathon, no secrets, no names from reference material.

## Boundaries between chunks

- The a-commerce agents will reach their tools through a2api. Until then the harness keeps its own tool registry, and the tool layer stays swappable: `make compare` already swaps the a-commerce tools for the screen-shaped facade. Chunk 2 adds an "a2api" layer in the same place.
- Chunk 1 does not change the a2api repository. Chunk 2 does not change `server/a_commerce/harness/` except to add that layer.
- Open conflict for chunks 1 and 2: the harness repairs loose model input inside the backend tools (`normalize` in `tools/base.py`), but a2api validates input strictly before it calls the backend, so through a2api that repair would not run. Decide where the repair lives before the agents switch to a2api.

What a2api can do today (commit `00510c3`, Oct 28), so changes here do not assume more:

- Active capabilities are read-only: GET and HEAD operations only, and `policy.approval` must be `not_required_for_this_read` (`src/a2api/contracts.py`, `src/a2api/runtime.py`).
- Input schemas may not use `$ref` or combinators such as `anyOf`.
- Active capabilities need `actor` and `tenant` context and explicit response checks for both.
- The MCP bridge needs the MCP SDK `mcp>=2.3`.

## Proposed, not decided

- How the stacked pull requests #2–#20 enter main. Each is judged on its own.
- No new intents or screens until the model gate below passes.
- Model gate: the same tasks on the same Glimmer, through a-commerce/a2api tools and through the screen-shaped facade. The numbers decide the claim.
- Questions for chunk 2: GET endpoints for reads, the shape of approval in contracts, keeping the strict schema rules, `actor` and `tenant` in backend responses, and whether the agent and evaluation code moves into a2api.

## How a change gets in

1. Branch from `main`. One change per branch, about 400 lines; do not stack on unmerged branches.
2. Collect evidence: the tests, and when agent behaviour changes, a run on the real model. If no model was available, say so.
3. Send the branch and the evidence to the judge agent. It answers APPROVE, REVISE, or REJECT against this file.
4. Open the pull request only after APPROVE, with the verdict in the body ([template](../.github/pull_request_template.md)). A person merges. Agents never merge and never push to `main`.

The steps are written out for coding agents in [.claude/skills/judge-gate/SKILL.md](../.claude/skills/judge-gate/SKILL.md).
