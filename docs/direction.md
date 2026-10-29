# Direction

Status: draft for team approval, Oct 30. "Decided" holds only what the team's Oct 30 message and the organizers' prize answer say, with their sources. Everything else is proposed until the people named next to it approve it (see "Proposed"). Changes to "Decided" need the approval of the people the change affects, in the pull request.

## Decided

1. **Who does what.** Source: team message, Oct 30. Chunk numbers are the message's; [plan.md §6.1](plan.md) numbers roles differently. The message names chunks 1, 2 and 4. Once approved, Change gate step 1 narrows "merge #2–#20" for chunk 1.

   | Chunk | People | Work |
   |---|---|---|
   | 1. Real model + harness | Seongjun, Donguk | Review and merge the stacked pull requests #2–#20. Get Glimmer and Spark Open keys. Serve Glimmer (a condition for the Best Open Weight prize). Run the back-office and storefront flows on the real model and fix what fails. |
   | 2. A2API library + backend connection | Haesoo, Haryeom | Connect a2api and the a-commerce server. Legacy API conversion demo. Add write support to a2api: preview, change set, person approval. |
   | 4. Front end, desktop, demo | Seongjun | Polish screens. Tauri desktop wrapper. Live demo deploy. Video, README, X post, submission. |

2. **Model.** Source: team message, Oct 30. `nvidia/Muse-Glimmer-30B-NVFP4` on a 5090, port-forwarded as an API.
3. **Prize eligibility.** Source: Cerebral Valley's answer in the hackathon Discord, Oct 30. A quantized Muse Glimmer served on our own hardware counts as an eligible open-weight model. The demo and README must say how and where the model is used, and we add provenance that judges can review without our GPU, such as run logs showing the model name and quantization, screen captures, and sample outputs.

## Background

Not a new decision; quoted so the judge and the team read the same thing.

- The goal in [design.md §1](design.md) is to show that "a small open-weight model (Muse Glimmer) can find the right tools, chain them, and finish back-office and storefront tasks", and that "the same tasks fail or cost much more when the same model faces a typical screen-shaped commerce API".
- The rules in [AGENTS.md](../AGENTS.md) still apply: only code written during the hackathon, no secrets, no names from reference material.

## Facts this work depends on

Checked on Oct 30. Re-check before relying on them.

a2api at commit `00510c3` (Oct 28):

- Enabled (executable) capabilities are read-only. Operations must be GET or HEAD, and `policy.approval` must be `not_required_for_this_read` (`src/a2api/contracts.py`, `src/a2api/runtime.py`).
- Enabled capabilities need `actor` and `tenant` context and explicit response checks for both (`contracts.py`).
- a2api validates input against the capability's schema, with closed objects, before it calls the backend (`runtime.py`, `schema.py`).
- Schemas of executable contracts may not use `$ref` or combinators such as `anyOf` (`schema.py`). Disabled drafts from the OpenAPI importer may.
- The MCP bridge needs the MCP SDK `mcp>=2.3,<3`.

a-commerce, as it is in the stacked pull requests #2–#20 (main holds only docs and mockups):

- Agents call tools from the registry in `server/a_commerce/tools/base.py`. The HTTP API exposes them as `POST /tools/{name}`, which enabled a2api capabilities cannot call.
- The tool-call layer repairs loose model input for every caller (`normalize`, called from `call()` in `tools/base.py`). Through a2api, strict validation would reject that input before the repair runs.
- `server/pyproject.toml` pins `mcp>=1.10,<2` (from #14). It cannot share an environment with a2api's MCP extra.
- The comparison harness (from #11) names a-commerce's own tool registry `"a2api"` (`harness/operator.py`, `harness/compare.py`, `"toolset": "a2api"` in `eval/results/*compare*.json`, and the "A2API vs screen-shaped API" row of `docs/progress.md` on #11–#20). No a2api library code ran in those results. The label must be renamed before chunk 2 adds a real a2api layer, and results must not be presented as a2api runs.
- Result files record the model name as the server reports it, so the quantization is on record only if the served name carries it (serve Glimmer as `nvidia/Muse-Glimmer-30B-NVFP4`).

## Proposed

Each item needs approval from the people named, given in the pull request that adds or changes it. "All" means all four. When an item is approved, the line `Approved by <names> in #<pull request>, <date>.` is added under it. **An item with that line is a requirement, like a Decided item; an item without it is context only.**

1. **Positioning** (all): a2api is the protocol and runtime; a-commerce is the first proof, in commerce, and the repository we submit. From the Oct 29 decision document; not in the Oct 30 message.
2. **Serving** (chunk 1): vLLM with the model card's tool and reasoning parsers for Glimmer; Muse Spark Open on Baseten (`meta-models/muse-spark-open-1026`, plan.md §2) as planner and fallback when keys arrive.
3. **Chunk boundaries** (chunks 1 and 2): the agents reach their tools through a2api once chunk 2's layer exists; chunk 1 does not change the a2api repository; chunk 2 changes `server/a_commerce/harness/` only to add its layer. Where the input repair lives is settled before the agents switch.
4. **Model gate** (all): the same tasks on the same Glimmer through the a-commerce tools, the screen-shaped facade, and the a2api layer once it exists. The numbers decide the claim.
5. **Scope** (all): no new intents or screens until the model gate passes.
6. **Questions for chunk 2** (chunk 2): GET endpoints for reads, the shape of approval in contracts, keeping the strict schema rules, `actor` and `tenant` in backend responses, and whether the agent and evaluation code moves into a2api.
7. **Change gate** (all, plus Seongjun and Donguk for step 1, which narrows Decided 1): the process below.

## Change gate

Proposed by Donguk on Oct 30. Steps for coding agents are in [.claude/skills/judge-gate/SKILL.md](../.claude/skills/judge-gate/SKILL.md).

1. **Branch from main.** One change per branch, about 400 lines (lockfiles and `eval/results/` files do not count), no stacking on unmerged branches.

   **The stacked pull requests #2–#20** are handled once, as follows.

   - **Judged:** #2–#8, each against its parent (`parent...child`), and the single commits of #11 and #15 (evaluation plus tool changes), each against its parent (#10 and #14). These are the server, harness, screens, real-model fixes and evaluation that Decided 1 and 3 build on. #14 was judged too.
   - **Not judged, closed:** #9, #10, #12, #13 and #16–#20 add intents and screens beyond the demo (see Proposed 5). Also closed: #14 (REJECT).
   - **Combined pull request:** branch `core/judged`, base `main`. It holds #2–#8, the commits of #11 and #15 if their verdicts allow it, and fix commits for every required change. A REJECT on #11 or #15 leaves that commit out; REVISE means its fixes go in as fix commits. A REJECT on one of #2–#8 ends the set just before it; #11 and #15 then go in only if they apply to the shorter set. The ~400-line rule does not apply to this pull request.
   - **Final verdict:** one verdict on `core/judged`, base `origin/main`, head at its tip. It covers the cherry-picked commits, their conflict resolutions and the fixes. Its head SHA is the one the merger checks.
   - **Tag first:** before any of #2–#20 is closed, Donguk creates and pushes the tag `prototype-v0` at the #20 tip (`6950b8f`), so the closed work stays retrievable. Closed work comes back only as new judged changes.
   - Until `core/judged` is merged, new code work may start from it. Its own pull request opens only after that base is merged, and it targets main.
2. **Evidence.** For code, the server tests (`cd server && uv run pytest -q`); the judge runs them itself. Docs-only changes need no tests; say so. When agent behaviour changes, a run on Glimmer, or name the model that ran, with a results file that records the model name and quantization. If no model ran, write "not run on a real model".
3. **Judge.** A fresh-context agent, not a fork that shares the author's context, follows `.claude/agents/judge.md` as it is on `origin/main`. If the author's branch changes that file, use a general agent given `git show origin/main:.claude/agents/judge.md`. The judge gets the refs and the evidence, not the author's reasons.
   - **Criteria:** the judge reads the criteria from `origin/main`. The only exception: before this file is on main, the pull request that adds it, the judgments of #2–#20 and the final verdict on `core/judged` read the criteria from the head of `process/judge-gate`. Every verdict names its criteria ref and SHA.
4. **Verdict per commit.** Each verdict names the base, head and criteria SHAs. Any push after APPROVE needs a new verdict. The pull request pastes every verdict block in full, including REVISE and REJECT.
5. **Pull request after APPROVE only.** A person merges after checking three things:
   - The pull request's head SHA is the approved one.
   - The criteria ref is `origin/main` or the exception in step 3.
   - The verdict's base is still `origin/main`, or main has moved only in files the change does not touch.

   The merger is not the person who ran the authoring agent. Agents never merge and never push to main.
6. **Rule changes.** Changes to "Decided" need explicit approval from the people the change affects. Changes to `judge.md`, the judge-gate skill, the pull request template or the AGENTS.md pointer need approval from all four. The judge's verdict alone is not enough.
7. **Enforcement.** Until someone enables branch protection on main (pull request plus one approval), the gate works only if everyone follows it.
