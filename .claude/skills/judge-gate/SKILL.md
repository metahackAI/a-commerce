---
name: judge-gate
description: How a change gets into a-commerce. Build on a branch, collect evidence, get a fresh judge agent's verdict, and open a pull request only after APPROVE. Use for every code or docs change meant for this repository.
---

# Judge gate

This is the change gate proposed in `docs/direction.md` ("Change gate"). It applies once the team approves that file's pull request. Follow the steps in order.

1. **Branch.** `git fetch origin && git switch -c <kind>/<name> origin/main`. One change per branch, about 400 lines; do not build on unmerged branches. While the stacked pull requests #2–#20 are not merged, follow the interim rule in "Change gate" step 1 of `docs/direction.md`.
2. **Stay in your chunk.** Read "Decided" and "Proposed" in `docs/direction.md`. If the change needs another chunk's files or the a2api repository, stop and ask the people who own them.
3. **Collect evidence.**
   - Code: `cd server && uv run pytest -q`, and the browser tests if screens changed.
   - Docs only: no tests; write "docs only".
   - Agent behaviour: run it on Glimmer, or name the model that ran (`make try-model RUNS=3`, `make compare RUNS=3`), and keep the results file. It must record the model name and quantization. If no model endpoint was available, write "not run on a real model".
4. **Ask the judge.** Start a fresh agent with no share of your context (not a fork) and tell it to follow `.claude/agents/judge.md`. Give it only:
   - the repository path, the base ref and the head ref,
   - the a2api path or commit to compare against,
   - the evidence: commands you ran and their output,
   - whether it may run the tests.

   Do not tell it why you think the change is right; it judges the code, not your argument.
5. **Act on the verdict.** Keep every verdict, including REVISE and REJECT.
   - REVISE: make the required changes, collect the evidence again, and ask a new judge with the new head.
   - REJECT: stop and report the verdict to the person you work for.
   - APPROVE: push exactly the approved head SHA and open the pull request with `.github/pull_request_template.md`. List every verdict run. Any later push needs a new verdict.
6. **A person merges.** The merger checks that the pull request's head SHA is the approved one and is not the person who ran the authoring agent. Changes to "Decided", `judge.md` or this skill also need the named people's approval in the pull request. Agents never merge and never push to main.
