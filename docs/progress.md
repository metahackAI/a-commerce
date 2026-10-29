# Progress

What works, what is proven by a test, and what is still missing. Updated every build loop. Plan: [plan.md](plan.md).

Run everything with `make check`: server tests, web build, and the browser test (CI runs the same on every PR).

## Works and is tested

| Area | What | Proven by |
|---|---|---|
| Seed | 42 summer outerwear products (chore coat 7 left, rain parka 4), 1,284 VIP members, 4 running promotions on Nov 7–8, policy as data | `server/tests/test_seed.py` |
| Change set #1042 | v1 flags low stock and the overshirt floor ($63.20 < $64.40); v2 has 6 of 6 checks with the mockup's wording; 40 of 42, overshirt 15%, $102.40 / $71.20 / $67.15 / $94.40; coupon to 1,284 VIP; hero slot Nov 7–8 | `test_scenario_vip_weekend.py::test_v1_*`, `::test_v2_matches_the_mockup` |
| Trust | Nothing changes before approval; an agent cannot apply its own preview; approving an old version is refused; checks re-run at commit; a change set holds only open write previews | `::test_nothing_changes_before_approval`, `::test_agent_cannot_apply_its_own_preview`, `::test_approving_an_old_version_is_refused`, `test_robustness.py::test_checks_run_again_at_commit`, `::test_a_change_set_holds_only_write_previews` |
| Run, read back, undo | Approve runs 3 writes then `verify_change_set` reads each back (40 of 40 prices, coupon count, banner); undo together restores prices and removes coupon and banner; undo closes after Nov 9 23:59 | `::test_approve_run_read_back_*`, `::test_undo_window_closes` |
| Storefront | Jamie (VIP) sees $71.20 on Sat Nov 7 and $89.00 on Friday; cart $89.00 − $17.80, free shipping, total $71.20 with VIPWEEKEND20; only the shopper's own request places the order; a guest pays list price | `::test_approve_run_read_back_*` |
| Harness | Operator's words → tool search (7 and 8 of the 11 operator tools for the two demo messages, graph neighbours via consumes/produces) → reads → previews → change set #1042 → v2 from the revision. The agent never approves or applies. **Shown with the scripted stand-in (a hand-written rule plan), not a model** | `test_harness.py::test_request_to_v1_then_v2_from_words`, `::test_the_agent_never_approves` |
| Model client | OpenAI-compatible client (vLLM, Ollama, Baseten) drives the same loop; a worker that gives up, errors or times out hands the request to the planner; a malformed tool call is a model error. **Tested with a fake server that replays fixed calls**; real runs are in `eval/` | `test_harness.py::test_openai_compatible_client_*`, `::test_a_worker_that_gives_up_*`, `test_robustness.py::test_a_model_server_error_*` |
| Definition of done | `make try-model` runs the two operator messages through the model in `.env` and judges the change set on 7 criteria (40 products, chore coat and parka out, $67.15, $71.20, 1,284 coupons, VIP hero Nov 7–8, 6 of 6 checks). With no model configured it exits 2 instead of passing on the stand-in | `test_harness.py::test_acceptance_judge_*`; Glimmer: not run yet |
| Back office web | Layout A at `make web`: conversation left, `change_set` block right (understood, checks, price group, coupon, left out, hero preview, after-approval). In Chromium: suggestion → v1 with 4 of 6 and Approve disabled → typed revision → v2 with 6 of 6, $67.15 and $71.20 → Approve → "Live · verified" with the read-back panel (40 of 40 prices, VIPWEEKEND20 to 1,284) → Undo all together. Jamie's price ($71.20 on Saturday, back to $89.00 after undo) is checked by API calls in the same test, not in the browser. The agent is the scripted stand-in | `web/e2e/back-office.spec.ts` (Playwright, in `make check` and CI) |
| Shopper agent | Designed so the model picks the component (`compare_table`, `look_gallery`, `product_list`) and the products, and the harness fills every price and spec inside the blocks from the tools. **Only the scripted stand-in has run it, and it picks the view from the profile by itself, so no model has been measured choosing components.** The reply sentence is model-written and is not checked against prices. A view outside the list comes back to the model as an error. Memories come back as `memory_note` blocks with Undo; shoppers never see cost or operator tools; a guest can place the order the agent prepared | `test_storefront.py`, `test_robustness.py` |
| Storefront web | `/fo?shopper=jamie`: after the operator approves #1042, Jamie asks for a light jacket → comparison table (because Jamie compares specs) with $71.20 / $102.40 / $67.15 → "Which one packs smallest?" is remembered with Undo → "The windbreaker. Same size as last time." → cart $89.00 − $17.80, free shipping, $71.20 → Jamie taps Place order → #20432. Mina gets a list at list prices, says "bigger photos" → same products as a gallery, memory undone | `web/e2e/storefront.spec.ts` (Playwright) |
| Real open model | Llama 3.2 3B (Meta open weights, local) got the first message right (42 products at 20%, coupon, VIP banner, low stock and the overshirt floor flagged) in 3 runs at temperature 0. **The three runs were identical, so this is one result, not 3 of 3.** The revision failed. On Oct 30, after the judges' fixes, at temperature 0.7: first message right in 2 of 3 runs, revision 0 of 3; the Oct 28 temperature-0 result did not reproduce. Failure types and fixes are in `eval/README.md`; three Oct 28 runs were saved (the first, 0 of 7, and two later ones), so eight of the ten fixes have no saved run behind them. The model is Ollama's `llama3.2:3b`, Q4_K_M | `eval/results/`; `test_harness.py::test_plan_sale_*`, `::test_a_model_that_times_out_*`, `test_robustness.py` |
| Replies checked against the change set | Seen from the real 3B and caught: "is now live" before approval, and "low-stock products left out" when they were not. Wrong product counts, wrong per-product rates and "all checks pass" are caught too, **but only tested on hand-written sentences**. Questions, offers and negations are not flagged. The operator sees a summary built from the change set and a note with the correction. The browser test checks that the note renders (it injects the response), not the server check | `test_grounding.py`; `e2e/back-office.spec.ts`; real 3B run in `eval/results/` |
| Comparison | Same model, same six tasks: A-Commerce tools vs a screen-shaped facade whose codes and formats can be learned from what it shows. Llama 3.2 3B, Oct 30: 2/18 vs 0/18 tasks at temperature 0.7 (3 runs each), 1/6 vs 0/6 at temperature 0. The model never read the facade's code lists. The A-Commerce result is within run-to-run noise (an earlier same-day run gave 4/18), so this does not yet show the tool design helps The arms also differ in prompt, tool offer and composition (`eval/README.md`). Not run on Glimmer | `test_facade.py`, `make compare`, `eval/results/` |
| HTTP | Tool definitions (every field described), shopper scope hides operator tools, change set and order over HTTP, action log | `test_api.py` |

## Not done yet (next loops, in order)

1. **Run `make try-model RUNS=3` on Glimmer and Spark Open** as soon as keys exist. The 3B stand-in fails the revision; that step is meant for the planner.
2. Returns, the task list and the other intents are parked (docs/direction.md); they come back only as new judged changes.

## Known gaps

- The scripted stand-ins understand only the demo's sentences. A real model is needed for anything else.
- Removing a chip on the storefront only hides it; it does not re-run the search yet.
- Mina's first answer is a list (as in `fo-mobile-adapt`); `fo-two-shoppers` shows her a 4-item gallery instead. Pick one.

## Blockers for the team

- Model access: Glimmer (`nvidia/Muse-Glimmer-30B-NVFP4`) on the team's RTX 5090 with vLLM. Put its endpoint in `.env` (`WORKER_*`, see `.env.example`) and run `make try-model RUNS=3` and `make compare RUNS=3`. Without a model the harness runs only the scripted stand-in, which understands only the demo's sentences.
- Decide §7 of the plan (mockup disagreements). The code uses the plan's values.
