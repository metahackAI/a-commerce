# Instructions for coding agents

Read `docs/design.md` before writing code. It defines the tool design rules, domain scope, harness, and UI protocol.

## Hackathon rules that affect code

- Everything in this repository must be written during the hackathon (Oct 27 – Nov 4, 2026). Do not paste code from other projects.
- The repository is public. Never commit secrets (`META_API_KEY` and others live in `.env`, which is git-ignored).

## Reference material

Team members may have reference material about an existing commerce platform on their machines, outside this repository. Use it only to understand how commerce businesses work (flows, rules, edge cases). Then design and write our own version:

- Do not copy code, SQL, table or column names, API paths, identifiers, sample data, or document text.
- Do not mention the product, its customers, or its internal names in code, comments, commits, or docs.
- Re-express each business rule in our own words and our own names, following `docs/design.md`.

## Tool checklist (every new tool)

- One business verb, `verb_noun` name in plain English
- Typed inputs, word enums, a description and example for every field
- Reads have no side effects and return items with the total
- Writes support `preview=true` and return rule checks and whether approval is needed
- Declares what it consumes and produces
- Errors include a code, a sentence, and a suggested next tool
- Business rules come from the policy file, not from code constants or prompts
