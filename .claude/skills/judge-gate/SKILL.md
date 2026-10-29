---
name: judge-gate
description: How a change gets into a-commerce. Build on a branch from main, collect evidence, get the judge agent's verdict, and open a pull request only after APPROVE. Use for every code or docs change meant for this repository.
---

# Judge gate

The team decided that no change becomes a pull request on its author's judgment alone. An independent judge agent checks it against `docs/direction.md` first. Follow these steps in order.

1. **Branch from main.** `git fetch origin && git switch -c <kind>/<name> origin/main`. One change per branch, about 400 lines. Do not build on a branch that is not merged yet.
2. **Stay in your chunk.** Read "Boundaries between chunks" in `docs/direction.md`. If the change needs another chunk's files or the a2api repository, stop and ask the people who own them.
3. **Collect evidence.** Run the tests (`./scripts/check.sh` once it exists on main, otherwise `cd server && uv run pytest -q`). If agent behaviour changed, run it on the real model (`make try-model RUNS=3`, `make compare RUNS=3`) and keep the results file. If no model endpoint was available, write "not run on a real model".
4. **Ask the judge.** Start the `judge` agent (`.claude/agents/judge.md`). If your tool cannot start it by name, start a general agent and tell it to follow that file. Give it only:
   - the repository path, the base ref and the head ref,
   - the a2api path or commit to compare against,
   - the evidence: commands you ran and their output,
   - whether it may run the tests.

   Do not tell it why you think the change is right; it has to judge the code, not your argument.
5. **Act on the verdict.**
   - REVISE: make the required changes, run the evidence again, and ask the judge again with the new head ref.
   - REJECT: stop. Report the verdict to the person you work for.
   - APPROVE: push the branch and open the pull request with `.github/pull_request_template.md`. Paste the verdict into "Judge verdict".
6. **A person merges.** Agents never merge pull requests and never push to `main`.
