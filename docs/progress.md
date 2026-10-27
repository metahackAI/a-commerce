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
| HTTP | Tool definitions (every field described), shopper scope hides operator tools, change set and order over HTTP, action log | `test_api.py` |

## Not done yet (next loops, in order)

1. **Harness with a model.** Tool search over `consumes`/`produces`, planner, executor that turns the request into the calls the acceptance test makes by hand. Needs model endpoints: no Glimmer or Spark Open key is configured yet (`.env.example` lists what is needed).
2. **Back office web** (layout A + `change_set`) reading `/change-sets/{id}`.
3. **Storefront web** (`understood_chips`, `compare_table`, `cart`, `memory_note`).
4. P1: returns (`return_options` → `refund_review`), Mina's layout switch, task list, evaluation against a screen-shaped facade.

## Blockers for the team

- Model access: someone with a Baseten (or Fireworks / Modal) key must put it in `.env`. Without it the harness can only run in scripted mode.
- Decide §7 of the plan (mockup disagreements). The code uses the plan's values.
