# Direction

Status: draft for team approval, Oct 30. "Decided" holds only what the team's Oct 30 message and the organizers' prize answer say, with their sources. Everything else is proposed until the people named next to it approve this file's pull request. Changes to "Decided" need the approval of the people the change affects, in the pull request.

## Decided

1. **Who does what.** Source: team message, Oct 30. Chunk numbers are the message's; [plan.md §6.1](plan.md) numbers roles differently. The message names chunks 1, 2 and 4.

   | Chunk | People | Work |
   |---|---|---|
   | 1. Real model + harness | Seongjun, Donguk | Review and merge the stacked pull requests #2–#20. Get Glimmer and Spark Open keys. Serve Glimmer (a condition for the Best Open Weight prize). Run the back-office and storefront flows on the real model and fix what fails. |
   | 2. A2API library + backend connection | Haesoo, Haryeom | Connect a2api and the a-commerce server. Legacy API conversion demo. Add write support to a2api: preview, change set, person approval. |
   | 4. Front end, desktop, demo | Seongjun | Polish screens. Tauri desktop wrapper. Live demo deploy. Video, README, X post, submission. |

2. **Model.** Source: team message, Oct 30. `nvidia/Muse-Glimmer-30B-NVFP4` on the team's RTX 5090, port-forwarded as an API.
3. **Prize eligibility.** Source: Cerebral Valley's answer in the hackathon Discord, Oct 30. A quantized Muse Glimmer served on our own hardware counts as an eligible open-weight model. The demo and README must say how and where the model is used, and we add provenance that judges can review without our GPU, such as run logs showing the model name and quantization, screen captures, and sample outputs.
4. **Rules.** Source: [AGENTS.md](../AGENTS.md). Only code written during the hackathon, no secrets, no names from reference material.

Background that is not a new decision: the goal in [design.md §1](design.md) is to show that "a small open-weight model (Muse Glimmer) can find the right tools, chain them, and finish back-office and storefront tasks", and that "the same tasks fail or cost much more when the same model faces a typical screen-shaped commerce API".

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
- The harness repairs loose model input inside the backend tools (`normalize` in `tools/base.py`). Through a2api, strict validation would reject that input before the repair runs.
- `server/pyproject.toml` pins `mcp>=1.10,<2` (from #14). It cannot share an environment with a2api's MCP extra.
- The comparison harness (from #11) names a-commerce's own tool registry `"a2api"` (`harness/operator.py`, `harness/compare.py`, and `"toolset": "a2api"` in `eval/results/*compare*.json`). No a2api library code ran in those results. The label must be renamed before chunk 2 adds a real a2api layer, and results must not be presented as a2api runs.

## Proposed

Each item needs approval from the people named, in the pull request that adds or changes it.

1. **Positioning** (all): a2api is the protocol and runtime; a-commerce is the first proof, in commerce, and the repository we submit. From the Oct 29 decision document; not in the Oct 30 message.
2. **Serving** (chunk 1): vLLM with the model card's tool and reasoning parsers for Glimmer; Muse Spark Open on Baseten (`meta-models/muse-spark-open-1026`, plan.md §2) as planner and fallback when keys arrive.
3. **Chunk boundaries** (chunks 1 and 2): the agents reach their tools through a2api once chunk 2's layer exists; chunk 1 does not change the a2api repository; chunk 2 changes `server/a_commerce/harness/` only to add its layer. Where the input repair lives is settled before the agents switch.
4. **Model gate** (all): the same tasks on the same Glimmer through the a-commerce tools, the screen-shaped facade, and the a2api layer once it exists. The numbers decide the claim.
5. **Scope** (all): no new intents or screens until the model gate passes.
6. **Questions for chunk 2** (chunk 2): GET endpoints for reads, the shape of approval in contracts, keeping the strict schema rules, `actor` and `tenant` in backend responses, and whether the agent and evaluation code moves into a2api.
7. **Change gate** (all): the process below.

## Change gate

Proposed by Donguk on Oct 30. It applies once the team approves this file's pull request. Steps for coding agents are in [.claude/skills/judge-gate/SKILL.md](../.claude/skills/judge-gate/SKILL.md).

1. **Branch from main.** One change per branch, about 400 lines, no stacking on unmerged branches.
   - Pull requests #2–#20 are already stacked. Each one is judged against its parent, as the pull request shows it. #2–#8 and the evaluation commits of #11 and #15 then enter main as one pull request. That pull request carries those verdicts, plus fix commits for every required change, judged as their own change. The stacking and size rules do not apply to this one pull request. A rejected pull request does not enter. The others stay parked under the tag `prototype-v0` and come back, if needed, as new judged changes.
   - Until that pull request is merged, new code work may start from its branch. Its own pull request opens only after that base is merged, and it targets main.
2. **Evidence.** Server tests (`cd server && uv run pytest -q`) for code. Docs-only changes need no tests; say so. When agent behaviour changes, a run on Glimmer, or name the model that ran, with a results file that records the model name and quantization. If no model ran, write "not run on a real model".
3. **Judge.** A fresh-context agent follows [.claude/agents/judge.md](../.claude/agents/judge.md) as it is on the base branch. It is not a forked agent that shares the author's context. It gets the refs and the evidence, not the author's reasons.
4. **Verdict per commit.** Each verdict names the base and head SHAs. Any push after APPROVE needs a new verdict. The pull request lists every verdict run, including REVISE and REJECT.
5. **Pull request after APPROVE only.** A person merges after checking that the pull request's head SHA is the approved one. The merger is not the person who ran the authoring agent. Agents never merge and never push to main.
6. **Rule changes.** Changes to "Decided", to `judge.md` or to the judge-gate skill need explicit approval from the people named, not only the judge's verdict.
7. **Enforcement.** Until someone enables branch protection on main (pull request plus one approval), the gate works only if everyone follows it.
