# Evaluation

`make try-model RUNS=3` runs the demo's two operator messages through the model in `.env` and judges the change set
(see `server/a_commerce/harness/acceptance.py`). Each run starts from a fresh seed; every tool call is saved in `results/` (kept: the first 3B run, 0 of 7, and the last, v1 3 of 3).

## Runs so far

| Date | Model (where) | First message (v1) | Revision (v2), all 7 criteria | Notes |
|---|---|---|---|---|
| Oct 28 | `scripted` stand-in | 3/3 | 3/3 | Rule-based, not a model. Proves the harness and judge only |
| Oct 28 | Llama 3.2 3B (Ollama, this Mac) | **3/3** | 0/3 | Meta open weights as a small-model stand-in until Muse keys arrive |
| — | Muse Glimmer 30B (partner server) | — | — | Needs keys |
| — | Muse Spark Open (Baseten) | — | — | Needs keys |

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
The 3B model's replies also claim things the change set does not do ("the sale is live"); the change set panel shows
the facts, but replies should be checked against it (open item).
