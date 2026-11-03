# Final runs, Nov 3: the runner's score and a reading of every reply

The runner (`cargo xtask eval`) decides "done" from tool calls, change sets and the numbers in a fact ([../README.md](../README.md) §8). Reading the replies of the final runs showed that it fails some right answers and passes some that are not. So every reply was also read and judged by the rules below. This page has both numbers and the reasons.

## Setup

- Code: main at 28ed971 (the agent and the runner are the same as at 4c6e415). Demo store reloaded before each write task, store clock Fri Nov 6 2026 11:00.
- Every model call on Muse Glimmer 30B, served as in [../../docs/serving.md](../../docs/serving.md). The server did not restart during the runs and the kernel logged no Xid. 10 of 1,543 model calls failed: 8 HTTP 500 (the server logged vLLM's `grammar rejected tokens [200008]` at those times), 1 timeout, 1 streamed answer that could not be read.
- Three tool layers over the same back end and the same model:
  - **A-Commerce tools**: the product's manifest.
  - **Screen-shaped facade**: one endpoint per admin screen, codes instead of words, writes run directly with no change set ([../../crates/facade](../../crates/facade/src/lib.rs)).
  - **a2api over the facade**: the same screen API converted into plain-named tools by [a2api](https://github.com/metahackAI/a2api) (commit 6743b1e). Writes still run directly, because the API under it does.
- Sets:
  - **Held-out**: the 9 of 16 held-out tasks the demo data can answer (`--ready`), 10 runs each per layer. Reports: `2026-11-03-glimmer-heldout-{acom,facade,a2api}.md`.
  - **Fresh**: 8 tasks written on Nov 2 against this same code, after the last agent change, and not shown to anyone building the agent. 5 runs each per layer. Not committed, so only totals are here.
  - **Design**: 12 tasks, 3 runs each, acom and facade. Runner score only: `2026-11-03-glimmer-design-{acom,facade}.md`.

## Rules for reading

The same rules for all three layers.

1. A run is done when a person reading the last reply and its blocks gets what they asked: the answer is there, it is right, and the reply commits to it. Detail around the answer (the date a parcel shipped, the count per size) is not required.
2. "I can't confirm from this page", or an answer that covers only the first page of a list, is not done.
3. A "none" or "no" counts only if the agent read data for it in that run. With no tool call it is a guess.
4. A write is done when the right change is drafted and waits for a person's approval. A reply that says the change does something it does not is not done. A change that cannot be undone and runs without approval is not done, and is counted on its own line.

Judged by Claude Code (Opus 5.5), reading each distinct reply (identical replies grouped) against the task's expected facts. One judge; not yet checked by a second person.

## Held-out: 9 tasks × 10 runs

| | Screen-shaped facade | a2api over the facade | A-Commerce tools |
|---|---|---|---|
| Done, runner | 0 of 90 | 10 of 90 (11%) | 20 of 90 (22%) |
| **Done, read** | **0 of 90** | **30 of 90 (33%)** | **40 of 90 (44%)** |
| Tasks done in all 10 runs, read | 0 of 9 | 3 of 9 | 4 of 9 |
| Customer message sent without approval | 0 (never got that far) | **10 of 10** | 0 of 10 (drafted for approval 10 of 10) |
| Model calls / tokens / seconds per run | 3.5 / 7,272 / 8.6 | 3.1 / 6,682 / 7.7 | 3.4 / 13,075 / 9.2 |

Per task, runs done as runner / read:

| Task | Facade | a2api | A-Commerce | What the replies did |
|---|---|---|---|---|
| md-how-many-left | 0 / 0 | 0 / 10 | 10 / 10 | Facade: `getPrdGridList failed (ERR_PARAM): Invalid paging`. a2api: "4 units in stock" (the runner wanted the sizes too). A-Commerce: 4, S 2 and XL 2. |
| ops-cancel-delivered | 0 / 0 | 0 / 10 | 0 / 10 | Facade: "There is no order 20402". Both others: delivered, so it cannot be cancelled; a return request is already open. The runner wanted the `$200` from the fact's aside. |
| ops-change-address-in-transit | 0 / 0 | 10 / 10 | 0 / 10 | Facade: "the order is on hand", then asks for the new address (wrong). Both others: in transit with UPS, the address cannot change. A-Commerce did not give the ship date the runner looks for. |
| ops-photo-request | 0 / 0 | 0 / 0 | 0 / 10 | Facade: paging error. a2api: sent the message straight away in all 10 runs; the screen API has no approval step. A-Commerce: drafted the message to `cus_riley` about `ord_20366`, waiting for approval. The task expects the kind `photo_request`, a value the tool does not have (`ask_for_photos`), so the runner can never pass it. |
| mk-pause-knit-until-monday | 0 / 0 | 0 / 0 | 0 / 0 | Facade: paging error. a2api: asks which Monday, changes nothing. A-Commerce: the right change (pause `promo_autumn_knit`) waits for approval, but the reply says it pauses "until Monday". A pause has no end; someone has to resume it on Mon Nov 9. |
| md-below-cost | 0 / 0 | 0 / 0 | 0 / 0 | Facade: "none" with no tool call. a2api: "none" with no tool call in 9 runs. A-Commerce: read 100 of 400 products, said none of those, offered to scan the rest. |
| md-broken-sizes-outerwear | 0 / 0 | 0 / 0 | 0 / 0 | Facade: asks which styles. a2api: cannot tell without stock per size. A-Commerce: names 1–2 of the 5 from the first rows, "check the table" for the rest. |
| ops-unfulfilled-3-days | 0 / 0 | 0 / 0 | 10 / 0 | Facade: paging error. a2api and A-Commerce: 412 processing, read the first 100, "can't confirm the rest". The runner passed A-Commerce because the numbers appear. |
| mgr-undo-banner | 0 / 0 | 0 / 0 | 0 / 0 | All three: "the old banner is already back" or offer to book it again. The answer is to undo change set cs_1039. |

## Fresh: 8 tasks × 5 runs

| | Screen-shaped facade | a2api over the facade | A-Commerce tools |
|---|---|---|---|
| Done, runner | 5 of 40 (12%) | 10 of 40 (25%) | 30 of 40 (75%) |
| **Done, read** | **0 of 40** | **5 of 40 (12%)** | **20 of 40 (50%)** |
| Tasks done in all 5 runs, read | 0 of 8 | 1 of 8 | 4 of 8 |

How the not-done runs failed, without the task text:

- A-Commerce: a date range one day longer than asked; a question answered without calling the tool that holds the numbers; scanning item by item until the 20-call limit; and one task whose reply call hit the vLLM grammar error in all 5 runs, so the agent fell back to "Here is what I found." over a table without the asked field ([../../docs/serving.md](../../docs/serving.md) §3 Limits).
- a2api: hedged answers on paged lists, a comparison without the asked average, an HTTP 403 on one read.
- Facade: paging errors, and a "most common" taken from a partial page.

## Design: 12 tasks × 3 runs, runner only

| | Screen-shaped facade | A-Commerce tools |
|---|---|---|
| Done, runner | 0 of 36 | 18 of 36 (50%) |
| pass^3 | 0 of 12 | 4 of 12 (33%) |

On Oct 31 (commit 91970bc) the same set scored 21 of 36, pass^3 6 of 12. Reruns of single tasks then moved by one or two runs out of three, so we read the 3 fewer runs as no clear change, not a measured drop.

## The held-out set was not fully unseen

The held-out tasks are in this repository, and on Nov 1 they ran several times to check whether agent changes made on the design set carry over (runner score 11–28%). So their failures had been seen while the agent was still changing. That is why the fresh set was written on Nov 2, after the last agent change, and kept out of the repository; it is the cleanest of the three.

## Runner problems found here

Left as they are, so these numbers stay comparable with the reports above. To fix after the hackathon:

1. `ops-photo-request` expects `kind: photo_request`; the tool's value is `ask_for_photos`. No run can pass.
2. A number inside a fact's aside is required ("over-$200 review", "since Nov 2"), so a right answer without that detail fails.
3. A hedged reply that contains the right numbers passes (`ops-unfulfilled-3-days`).
4. Numbers written as words ("four") are not read.
