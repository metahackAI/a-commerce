# Progress

What works, what is proven by a test, and what is still missing. Updated every build loop. Plan: [plan.md](plan.md).

Run everything with `make check` (CI runs the same on every PR).

## Works and is tested

| Area | What | Proven by |
|---|---|---|
| Seed | 42 summer outerwear products (chore coat 7 left, rain parka 4), 1,284 VIP members, 4 running promotions on Nov 7–8, policy as data | `server/tests/test_seed.py` |
| Change set #1042 | v1 flags low stock and the overshirt floor ($63.20 < $64.40); v2 has 6 of 6 checks with the mockup's wording; 40 of 42, overshirt 15%, $102.40 / $71.20 / $67.15 / $94.40; coupon to 1,284 VIP; hero slot Nov 7–8 | `test_scenario_vip_weekend.py::test_v1_*`, `::test_v2_matches_the_mockup` |
| Trust | Nothing changes before approval; an agent cannot apply its own preview; approving an old version is refused; checks re-run at commit | `::test_nothing_changes_before_approval`, `::test_agent_cannot_apply_its_own_preview`, `::test_approving_an_old_version_is_refused` |
| Run, read back, undo | Approve runs 3 writes then `verify_change_set` reads each back (40 of 40 prices, coupon count, banner); undo together restores prices and removes coupon and banner; undo closes after Nov 9 23:59 | `::test_approve_run_read_back_*`, `::test_undo_window_closes` |
| Storefront | Jamie (VIP) sees $71.20 on Sat Nov 7 and $89.00 on Friday; cart $89.00 − $17.80, free shipping, total $71.20 with VIPWEEKEND20; only the shopper's own request places the order; a guest pays list price | `::test_approve_run_read_back_*` |
| Harness | Operator's words → tool search (6–9 of 16 tools, graph neighbours via consumes/produces) → reads → previews → change set #1042. v1 takes 9 tool calls and reports the two checks that need a decision; "Drop the low-stock ones. Give the overshirt 15% instead." makes v2 with 6 of 6 checks. The agent never approves or applies | `test_harness.py::test_request_to_v1_then_v2_from_words`, `::test_the_agent_never_approves` |
| Real-model path | OpenAI-compatible client (Baseten / Fireworks / vLLM) drives the same loop; a worker that gives up hands the request to the planner | `test_harness.py::test_openai_compatible_client_*`, `::test_a_worker_that_gives_up_*` |
| Definition of done | `make try-model` runs the two operator messages through the model in `.env` and judges the change set on 7 criteria (40 products, chore coat and parka out, $67.15, $71.20, 1,284 coupons, VIP hero Nov 7–8, 6 of 6 checks) | `test_harness.py::test_acceptance_judge_*` (scripted); real model: pending keys |
| HTTP | Tool definitions (every field described), shopper scope hides operator tools, change set and order over HTTP, action log | `test_api.py` |

## Not done yet (next loops, in order)

1. **Back office web** (layout A + `change_set`) on `/bo/chat` and `/change-sets/{id}`.
2. **Storefront web** and the shopper agent (`understood_chips`, `compare_table`, `cart`, `memory_note`).
3. **Run `make try-model` on Glimmer and Spark Open** as soon as keys exist; until then the harness runs the scripted stand-in, which is labelled "scripted" in every event.
4. P1: returns (`return_options` → `refund_review`), Mina's layout switch, task list, evaluation against a screen-shaped facade.

## Blockers for the team

- Model access: someone with a Baseten (or Fireworks / Modal) key must put it in `.env` (`WORKER_*`, `PLANNER_*`) and run `make try-model`. Without it the harness can only run in scripted mode, which only understands sale requests like the demo's.
- Decide §7 of the plan (mockup disagreements). The code uses the plan's values.
