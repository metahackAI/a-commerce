# Progress

What works, what is proven by a test, and what is still missing. Updated every build loop. Plan: [plan.md](plan.md).

Run everything with `make check`: server tests, web build, and the browser test (CI runs the same on every PR).

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
| Back office web | Layout A at `make web`: conversation left, `change_set` block right (understood, checks, price group, coupon, left out, hero preview, after-approval). In Chromium: suggestion → v1 with 4 of 6 and Approve disabled → typed revision → v2 with 6 of 6, $67.15 and $71.20 → Approve → "Live · verified" with the read-back panel (40 of 40 prices, VIPWEEKEND20 to 1,284) → Jamie's price is $71.20 on Saturday → Undo all together → back to $89.00 | `web/e2e/back-office.spec.ts` (Playwright, in `make check` and CI) |
| Shopper agent | The model picks the component (`compare_table`, `look_gallery`, `product_list`) and the products; the harness fills every price and spec from the tools, so no model-written price reaches the screen. Memories come back as `memory_note` blocks with Undo; shoppers never see cost or operator tools | `test_storefront.py` |
| Storefront web | `/fo?shopper=jamie`: after the operator approves #1042, Jamie asks for a light jacket → comparison table (because Jamie compares specs) with $71.20 / $102.40 / $67.15 → "Which one packs smallest?" is remembered with Undo → "The windbreaker. Same size as last time." → cart $89.00 − $17.80, free shipping, $71.20 → Jamie taps Place order → #20432. Mina gets a list at list prices, says "bigger photos" → same products as a gallery, memory undone | `web/e2e/storefront.spec.ts` (Playwright) |
| Real open model | Llama 3.2 3B (Meta open weights, local) gets the first message right in 3 of 3 runs: 42 products at 20%, coupon, VIP banner, low stock and the overshirt floor flagged. The revision is 0 of 3. Ten failure types found and fixed in tools and harness (`eval/README.md`) | `make try-model RUNS=3`; `test_harness.py::test_plan_sale_*`, `::test_a_model_that_times_out_*` |
| Replies checked against the change set | Claims that it is live before approval, that low-stock products were left out when they were not, wrong product counts, wrong per-product rates, and "all checks pass" are caught; the operator sees a summary built from the change set and a note with the correction. Correct replies pass through unchanged | `test_grounding.py` (with the real 3B replies); `e2e/back-office.spec.ts` (note shown); real 3B run in `eval/results/` |
| Returns (server) | Alex sees "New pair, same size" (6 in stock, arrives Tue Nov 10) or "Full refund" $312.00 to card 0937 confirmed by a person; only Alex's own Send creates the request. `plan_return_review` sorts the 12 waiting returns over $200 by policy: 9 refunds $2,463.00, 2 photo requests, 1 decline (#20290, day 39); checks 12 of 12 payments, 3 coupons, 11 of 12 in window, one repeat returner (#20431). One approval refunds and sends, reads each back; refunds cannot be undone | `test_returns.py` |
| Returns on screen | Alex on `/fo?shopper=alex`: the agent shows `return_options` (new pair or full refund), "Refund, please." preselects the refund, Alex taps Send. The operator asks "Check returns waiting over $200": the agent calls `plan_return_review` and the `refund_review` block shows 12 requests, 9 · $2,463.00, the policy per row, 1 check to look at, and the three messages; one approval refunds and sends, then every order is read back | `e2e/returns.spec.ts`; `test_returns.py::test_alex_and_the_operator_through_the_agents` |
| A2API vs screen-shaped API | Same model (Llama 3.2 3B), same 3 tasks, same harness: our tools pass 2/3 tasks (20/21 criteria, 0 errors); the screen-shaped facade passes 0/3 (3 errors a task: codes guessed as words, `E999`). The facade can do the task when driven by hand | `make compare`; `test_facade.py`; `eval/README.md` |
| HTTP | Tool definitions (every field described), shopper scope hides operator tools, change set and order over HTTP, action log | `test_api.py` |

## Not done yet (next loops, in order)

1. **Run `make try-model RUNS=3` on Glimmer and Spark Open** as soon as keys exist. The 3B stand-in fails the revision; that step is meant for the planner.
2. **More comparison tasks** written by someone who did not build the tools, and the same comparison on Glimmer and Spark Open.
3. P1: task list (layout E), live demo deploy, README for judges, demo video script.

## Known gaps

- The scripted stand-ins understand only the demo's sentences. A real model is needed for anything else.
- Removing a chip on the storefront only hides it; it does not re-run the search yet.
- Mina's first answer is a list (as in `fo-mobile-adapt`); `fo-two-shoppers` shows her a 4-item gallery instead. Pick one.

## Blockers for the team

- Model access: someone with a Baseten (or Fireworks / Modal) key must put it in `.env` (`WORKER_*`, `PLANNER_*`) and run `make try-model`. Without it the harness can only run in scripted mode, which only understands sale requests like the demo's.
- Decide §7 of the plan (mockup disagreements). The code uses the plan's values.
