# Evaluation

`make try-model RUNS=3` runs the demo's two operator messages through the model in `.env` and judges the change set
(see `server/a_commerce/harness/acceptance.py`). Each run starts from a fresh seed; every tool call is saved in `results/` (kept: the first 3B run, 0 of 7; v1 3 of 3; and a run with replies corrected).

## Runs so far

| Date | Model (where) | First message (v1) | Revision (v2), all 7 criteria | Notes |
|---|---|---|---|---|
| Oct 28 | `scripted` stand-in | 3/3 | 3/3 | Rule-based, not a model. Proves the harness and judge only |
| Oct 28 | Llama 3.2 3B (Ollama, this Mac) | **3/3** | 0/3 | Meta open weights as a small-model stand-in until Muse keys arrive |
| — | Muse Glimmer 30B (partner server) | — | — | Needs keys; run `make try-model RUNS=3` and `make compare` |
| — | Muse Spark Open (Baseten) | — | — | Needs keys |

## A-Commerce tools vs a screen-shaped API (design.md §7)

`make compare` gives the same model the same three tasks with one of two tool layers over the same store:
our tools, or `server/a_commerce/facade.py`, a screen-shaped API we wrote to reproduce design.md §2 (grid reads with
paging and a separate count call, abbreviations, codes such as `G01` and `T03`, bulk saves with `rowStatus`, a V2
duplicate, `E999 System error`). `test_facade.py` shows the facade can do the demo task when driven by hand, so a
failure on it is about its shape. Both layers get the same lenient argument decoding and the same harness.

Tasks (Fri Nov 6 11:02): the demo request; "Give Gold members 10% off all summer outerwear next weekend. No banner.";
"Put shirts on a 15% sale for everyone this weekend." Seven criteria each (proposed, all products, rate, audience,
dates, coupon, banner).

| Date | Model | Tool layer | Tasks passed | Criteria met | Calls / task | Errors / task |
|---|---|---|---|---|---|---|
| Oct 28 | Llama 3.2 3B (local) | **A-Commerce tools** | **2/3** | **20/21** | 2.0 | 0.0 |
| Oct 28 | Llama 3.2 3B (local) | Screen-shaped facade | 0/3 | 0/3 | 3.0 | 3.0 |
| Oct 28, later | Llama 3.2 3B (local) | **A-Commerce tools** | **3/6** | **38/42** | 2.2 | 0.2 |
| Oct 28, later | Llama 3.2 3B (local) | Screen-shaped facade | 0/6 | 0/6 | 3.7 | 3.5 |

The second pair adds three tasks written after the first comparison (not used to tune the tools): "VIP members get 15%
off summer outerwear this weekend, except the rain parka.", "Gold members: 10% off shirts tomorrow only.", "Everyone
gets 10% off summer outerwear next weekend. Put it on the main page too." It also fixes a fairness bug in the first
pair: the facade had no member-grade code for "everyone", so the shirts task could not be done on it (`G00` added;
`test_facade.py` now checks every task can be expressed on the facade). The facade still passed none.

Misses on our tools, all the model's: "next weekend" read as `next_7_days` (two tasks), "VIP" sent as `gold`, and the
rain parka put in `overrides` instead of `exclude`. That last one was silently ignored by the tool; since this run an
override without a rate is an error that says to use `exclude`. The numbers above are from before that fix.

What happened on the facade: the model sent words where codes were needed (`"VIP members"` for `G01`, `"main page"`
for `MAIN_HERO`), dates in its own format, and never got past `E999 System error`, so no change set was proposed.
On our tools the one miss was "next weekend": the model passed `next_7_days`.

Limits: one run per task at temperature 0 (repeated runs give the same answer); the tasks were written by us and our
tools were tuned on the demo request earlier the same day; a 3B stand-in, not Glimmer. Re-run on Glimmer and Spark
Open, with more tasks written by someone who did not build the tools, before putting numbers in the main README.

## What a 3B model taught us (Oct 28)

The first run scored 0 of 7. Each failure below was fixed in the tool layer or harness, not by prompting harder:

| Failure | Fix |
|---|---|
| Dates in words (`"Friday, 12:00 AM"`) | Tools take `period: this_weekend` resolved against the store clock |
| Numbers and lists as strings (`"20"`, `"[]"`, `"['pv_1']"`, double-encoded lists) | Arguments are decoded leniently, then validated as before |
| Category by name (`"summer outerwear"`) | Category id or name |
| Stopped talking after an error, never proposed | Harness nudges up to twice with the error and the next step |
| Did not chain sale + coupon + banner | `plan_sale`: one business call that previews all three |
| Wrote the tool call as JSON text | Harness reads tool calls from text too |
| Endless generation, 120 s timeout | `max_tokens` cap; model errors fail the attempt cleanly and hand over |
| `"overshirt"` matched two products | Tool says which ones and asks for a fuller name; revisions see v1's checks |
| Revision dropped the period and rate | A revision call is a patch over the earlier call |
| Failing check gave no way out | Each failing check says the argument that fixes it |

Still failing with 3B: the revision mixes sale-wide and per-product changes (sets 15% for everything, or misses
low stock). That is the step design.md gives to the planner (Spark), so the next measurement is Glimmer and Spark Open.
The 3B model's replies also claimed things the change set did not do ("is now live", "dropped low-stock products").
Since Oct 28 every reply is checked against the change set (`harness/grounding.py`): a contradicting reply is replaced
by a summary built from the change set, and the operator sees what was corrected. On the 3B run in `results/` both
false replies were caught; the corrected v2 summary ("42 products … at 15% off … 5 of 6 checks pass") shows the operator
at once that the revision went wrong, where the model had said it went right.
