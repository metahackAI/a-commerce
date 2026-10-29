---
name: judge-gate
description: How a change gets into a-commerce. Build on a branch, collect evidence, get a fresh judge agent's verdict, and open a pull request only after APPROVE. Use for every code or docs change meant for this repository.
---

# Judge gate

This is the change gate proposed in `docs/direction.md` ("Change gate"). It applies once the team approves that file's pull request. Follow the steps in order.

1. **Branch.** `git fetch origin && git switch -c <kind>/<name> origin/main`. One change per branch, about 400 lines; do not build on unmerged branches. While the stacked pull requests #2–#20 are not merged, follow the interim rule in "Change gate" step 1 of `docs/direction.md`.
2. **Stay in your chunk.** Read "Decided" and "Proposed" in `docs/direction.md`; a Proposed item with an `Approved by …` line binds like a Decided one. If the change crosses an approved chunk boundary (Proposed 3) or touches the a2api repository, stop and ask the people who own that work.
3. **Collect evidence.**
   - Code: `cd server && uv run pytest -q`, and the browser tests if screens changed. The judge runs the server tests again itself.
   - Docs only: no tests; write "docs only".
   - Agent behaviour: run it on Glimmer, or name the model that ran (`make try-model RUNS=3`, `make compare RUNS=3`), and keep the results file. It must record the model name and quantization. If no model endpoint was available, write "not run on a real model".
4. **Ask the judge.** Start a fresh agent with no share of your context (not a fork) and tell it to follow the judge as it is on main: `git show origin/main:.claude/agents/judge.md`, not your working copy. The `judge` agent type may be used only when `git diff origin/main -- .claude/agents/judge.md` is empty. Give it only:
   - the repository path, the base ref and the head ref,
   - the criteria ref (`origin/main`, or the one exception in "Change gate" step 3 of `docs/direction.md`),
   - the a2api path or commit to compare against,
   - the evidence: commands you ran and their output.

   Do not tell it why you think the change is right; it judges the code, not your argument.
5. **Act on the verdict.** Keep every verdict, including REVISE and REJECT.
   - REVISE: make the required changes, collect the evidence again, and ask a new judge with the new head.
   - REJECT: stop and report the verdict to the person you work for.
   - APPROVE: push exactly the approved head SHA and open the pull request with `.github/pull_request_template.md`. Paste every verdict block in full. Any later push needs a new verdict.
6. **A person merges.** The merger is not the person who ran the authoring agent, and checks that the head SHA is the approved one, the criteria ref is allowed, and the verdict's base is still current (`docs/direction.md`, "Change gate" step 5). Changes to "Decided" also need approval from the people they affect, and changes to `judge.md`, this skill, the pull request template or the AGENTS.md pointer from all four, in the pull request. Agents never merge and never push to main.
