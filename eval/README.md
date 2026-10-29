# Evaluation

Two measurements, both with the model in `.env` and a fresh seed per run:

- `make try-model RUNS=3` runs the demo's two operator messages and judges the change set on 7 criteria
  (`server/a_commerce/harness/acceptance.py`). Without a model configured it exits 2 instead of passing on the stand-in.
- `make compare RUNS=3` gives the same model six tasks on two tool layers (below).

When a model (not the scripted stand-in) ran, each command saves its runs to `results/` with each tool call, the
endpoint, temperature and `max_tokens`, the model the server says it ran (`served_model`), and the code commit
(`+dirty` if files had changed since it). In a revision the saved arguments are the harness-merged ones (the model's
patch over the earlier call), not exactly what the model sent. The model name in each file is the one the client sends
(`WORKER_MODEL`), so the quantization is on record only when that is the served name (serve Glimmer as
`nvidia/Muse-Glimmer-30B-NVFP4` and set `WORKER_MODEL` to it).

**No Muse model has run yet.** Everything below is Llama 3.2 3B (Meta open weights; Ollama's `llama3.2:3b`, 3.2B
parameters, Q4_K_M quantization) on one Mac, used as a small-model stand-in.

## Demo request (`make try-model`)

| Date | Model, sampling | First message (v1) | Revision (v2), all 7 criteria | Notes |
|---|---|---|---|---|
| Oct 28 | `scripted` stand-in | 3/3 | 3/3 | Rule-based, not a model. Shows the harness and the judge work |
| Oct 28 | Llama 3.2 3B, temperature 0 | right | wrong | The three runs were identical, so this is one result, not 3/3 |
| Oct 30 | Llama 3.2 3B, temperature 0.7, commit `b9b8a70` | 0/3 | 0/3 | `results/20261030-042131-llama3.2_3b.json` |
| — | Muse Glimmer (`nvidia/Muse-Glimmer-30B-NVFP4`, vLLM on the team's 5090) | — | — | Not run yet |
| — | Muse Spark Open (Baseten) | — | — | Not run yet |

The Oct 28 temperature-0 result does not reproduce: on Oct 30 the same code (`eb0e51f`) with the same model and the
same Ollama build made a different first call and failed (a two-run spot check of the demo task, not saved to
`results/`). A single temperature-0 run is not evidence.

## A-Commerce tools vs a screen-shaped API (design.md §7)

`make compare` gives the same model the same six tasks with one of two tool layers over the same store: the A-Commerce
tools (`acommerce`), or `server/a_commerce/facade.py`, a screen-shaped API we wrote to reproduce design.md §2. The
facade has grid reads with paging and a separate count call, abbreviations, codes such as `G01` and `T03`, bulk saves
with `rowStatus`, a V2 duplicate, and `E999 System error`. Since Oct 30 every code, flag and date format it needs can
be learned from its definitions and responses, as from a terse API spec. `test_facade.py` drives the demo task by hand
using only strings the facade showed.

Tasks (Fri Nov 6 11:02), all written by the person who built the tools: the demo request; "Give Gold members 10% off
all summer outerwear next weekend. No banner."; "Put shirts on a 15% sale for everyone this weekend."; "VIP members
get 15% off summer outerwear this weekend, except the rain parka."; "Gold members: 10% off shirts tomorrow only.";
"Everyone gets 10% off summer outerwear next weekend. Put it on the main page too." Six criteria each, all stated in
the request: change set proposed, products, rate, audience, dates, banner made or not.

The two arms differ in more than API shape. Report them as separate factors (a2api docs/03 §4):

- **Prompt:** the A-Commerce arm gets one extra line ("Prefer category-wide arguments …") about its own fields.
- **Tool offer:** the A-Commerce arm sees the tools that tool search picks (for example 8 operator tools plus
  `propose_change_set` for the demo request); the facade arm sees all 8 facade tools.
- **Composition:** A-Commerce has `plan_sale`, one call that previews the sale, coupon and banner. The facade has no
  composite call.
- **Shared prompt:** both arms' system prompt mentions rule checks and `ref:<preview_id>`, which only the A-Commerce
  tools have.
- **Input repair:** both arms decode JSON-like strings. The A-Commerce tools also repair period names and category
  names; the facade does not.

| Date | Model, sampling | Tool layer | Tasks passed | Criteria met | Calls / task | Errors / task |
|---|---|---|---|---|---|---|
| Oct 30 | Llama 3.2 3B, temperature 0.7, 3 runs per task | **A-Commerce tools** | **1/18** | **46/108** | 3.8 | 2.2 |
| Oct 30 | Llama 3.2 3B, temperature 0.7, 3 runs per task | Screen-shaped facade | 0/18 | 0/108 | 3.1 | 2.8 |
| Oct 30 | Llama 3.2 3B, temperature 0, 1 run per task | **A-Commerce tools** | **1/6** | **11/36** | 5.5 | 3.5 |
| Oct 30 | Llama 3.2 3B, temperature 0, 1 run per task | Screen-shaped facade | 0/6 | 0/36 | 3.3 | 3.2 |

Files, all made at commit `b9b8a70` with no uncommitted changes: `results/20261030-041619-compare-llama3.2_3b.json`
(temperature 0.7) and `results/20261030-042051-compare-llama3.2_3b.json` (temperature 0).

**What this shows, and what it does not.** The facade side is stable: in every facade task run on Oct 30 the model
never called a read tool (`getCodeList`, `getPrdGridList`). It wrote saves with codes it made up (product `SNW001`,
promotion type `CTG001`, dates such as `20261231 000000`), got `E999`, and proposed no change set. So this model does
not look codes up before writing; the result does not say what a model that does would manage. On the A-Commerce
tools the model often gets parts right (46 of 108 criteria) but rarely finishes a task: the most common failed
criterion is "change set proposed" (9 of 18 runs). Whole-task passes are not stable. Two earlier runs the same day,
on code with bugs since fixed (list decoding, then "null" in text fields), gave 4/18 and 2/18 at temperature 0.7; their
files were replaced and are kept in commits `d7742a4` and `28d94ae`. With a 3B model and 3 runs per task, the spread
between runs is as large as the A-Commerce task result itself. These numbers do not yet show that the tool design makes
a small model finish tasks; they show that this 3B model gets nowhere on the facade and partway on the A-Commerce tools.

What happened on the A-Commerce tools at temperature 0.7: the most common errors were `plan_sale` without
`percent_off` (8 of 39), a missing period (4) and a missing category (4). The one pass was "Give Gold members 10% off
all summer outerwear next weekend. No banner."; the demo request passed none of its 3 runs. We did not tune the tools
on these results, because the same person wrote the tools and the tasks.

Limits: a 3B stand-in, not Glimmer; tasks written by the tool author; one machine; too few runs to separate the
A-Commerce result from run-to-run noise. Before any number goes into the
main README, re-run on Glimmer with `RUNS=3` and with tasks written by someone who did not build the tools.

### Earlier comparison (Oct 28), not comparable

The Oct 28 files (`results/20261028-042929-…`, `results/20261028-045820-…`) reported A-Commerce 2/3 and 3/6 against
facade 0/3 and 0/6. An independent review on Oct 30 found three problems:

- The facade hid code group names, date formats, `rowStatus` values and `MAIN_HERO`, so its zero came from how it was
  built, not from a measurement.
- The A-Commerce arm is labelled `"toolset": "a2api"` in those files. It is a-commerce's own tool registry; the a2api
  library was not used.
- The coupon was scored, though only `plan_sale` knew that tier sales get one. The facade "criteria met" also had a
  different denominator.

## What a 3B model taught us (Oct 28)

The first run scored 0 of 7. Each failure below was addressed in the tool layer or harness, not by prompting harder.
Three Oct 28 runs were saved: `20261028-033644` (the first, 0 of 7), `20261028-035650` and `20261028-040349`. The first
shows the first four rows; the later two show the revision that still fails and the false replies. **The other six
rows have no saved run behind them.**

| Failure | Fix |
|---|---|
| Dates in words (`"Friday, 12:00 AM"`) | Tools take `period: this_weekend` resolved against the store clock |
| Numbers and lists as strings (`"20"`, `"[]"`, `"['pv_1']"`, double-encoded lists) | Arguments are decoded leniently, then validated as before; text fields stay text |
| Category by name (`"summer outerwear"`) | Category id or name |
| Stopped talking after an error, never proposed | Harness nudges up to twice with the error and the next step |
| Did not chain sale + coupon + banner | `plan_sale`: one business call that prepares all three previews |
| Wrote the tool call as JSON text | Harness reads tool calls from text too, including OpenAI's own shape |
| Endless generation, 120 s timeout | `max_tokens` cap; model errors fail the attempt cleanly and hand over |
| `"overshirt"` matched two products | Tool says which ones and asks for a fuller name; revisions see v1's checks |
| Revision dropped the period and rate | A revision call is a patch over the earlier call |
| Failing check gave no way out | Each failing check says the argument that fixes it |

Still failing with 3B: the revision mixes sale-wide and per-product changes (sets 15% for everything, or misses low
stock). The 3B model's replies also claimed things the change set did not do ("is now live", "dropped low-stock
products"). Every reply is now checked against the change set (`harness/grounding.py`): a contradicting reply is
replaced by a summary built from the change set, and the operator sees what was corrected. On the Oct 28 3B run both
false replies were caught. Count, per-product rate and "all checks pass" claims are covered only by hand-written test
sentences.
