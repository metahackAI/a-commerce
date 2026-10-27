# Demo video (3 minutes)

What the judges should leave with: **the operator and the shopper only talk; the agent does the work through tools a
small open model can use; every change is a reviewable change set that is checked, read back, and undoable.**

Everything shown is in this repository and runs with `make demo`. Record at 1440×900. Before recording: `make demo`,
open http://localhost:5173, click **Reset data**. If Muse keys are in `.env`, the model name on each agent step is the
real one; otherwise it says `scripted` and the narration must not claim a model ran.

## Shot list

| Time | Screen | Do | Say (about this much) |
|---|---|---|---|
| 0:00–0:15 | Back office home (task list) | Point at To review: the returns the agent noticed, the reorder draft | "Running a sale today means five back-office screens in the right order. With A-Commerce the merchandiser just says it." |
| 0:15–0:40 | Back office | Click **Fri Nov 6, 11:02**, then the VIP weekend suggestion. Point at the step line (model, tool calls) and the change set: understood, 4 of 6 checks | "The agent found the tools it needed, prepared every change as a preview, and stopped. Nothing has changed in the store. Two checks need a decision: two products are almost out, and 20% would sell the overshirt below its price floor." |
| 0:40–1:00 | Back office | Type "Drop the low-stock ones. Give the overshirt 15% instead." | "I answer like I would a colleague. Version 2: 40 products, the overshirt at 15%, all six checks pass." |
| 1:00–1:20 | Back office | **Approve and run**; scroll to the read-back panel | "Only a person can approve. After running, the harness reads every change back from the store and compares it with what I approved: 40 of 40 prices, the coupon for 1,284 VIP members, the banner. And it can all be undone together until Monday night." |
| 1:20–1:50 | Storefront, Jamie | Click **Sat Nov 7, 10:00** in the back office first, then **Open storefront**. Click the jacket suggestion; then "Which one packs smallest?"; then "The windbreaker. Same size as last time." | "Saturday. Jamie compares specs, so the store answers with a table and says why. The price is $71.20, the same tool and the same promotion the operator just approved. Jamie's preference is remembered, visibly, with an undo. The agent prepares the cart, but only Jamie can place the order." Click **Place order**. |
| 1:50–2:05 | Storefront, Mina | Switch to Mina; jacket question; "Less text. Just show me bigger photos." | "Same question, first-time visitor on a phone: a list. One sentence changes how the store looks, not just what it shows." |
| 2:05–2:35 | Storefront Alex → back office | Alex: boots message, "Refund, please.", **Send refund request**. Back office: **Check returns waiting over $200**, then **Approve 9 refunds and send 3 messages** | "Alex's refund is over $200, so policy says a person decides. It lands in the operator's review with eleven others, sorted by what the policy says: nine refunds, two photo requests, one decline. One approval; refunds are final, so there is no undo button." |
| 2:35–2:50 | `eval/README.md` table | — | "Does the tool design matter? Same small open model, same tasks: on our tools it finished two of three; on a typical screen-shaped commerce API, zero. It guessed codes and got 'System error'." Say which model the numbers come from. |
| 2:50–3:00 | Back office | The corrected-reply note, if recorded with a real model | "And when a small model says something the change set doesn't do, the operator sees the facts instead." Close on the change set. |

## Lines to avoid

- Speed or cost claims we have not measured.
- "Glimmer did X" unless the agent steps in the recording show Glimmer.
- Calling the 3B comparison final; it is the first measurement (see `eval/README.md` for its limits).

## X post (Most Viral prize, post before submitting)

> Running a sale in most commerce back offices means 5 screens in the right order.
> We made the back end into tools a small open model can actually use. You say it; the agent builds a change set
> you review like a pull request: checks, preview, approve, read-back, undo.
> Same 3B model, same tasks: 2/3 done on our tools, 0/3 on a screen-shaped API.
> Built at the Meta AI Developer Hackathon. [video] [repo]

Update the numbers in the post to whatever the final measurement is, and say which model produced them.
