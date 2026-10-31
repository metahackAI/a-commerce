# Front-end UI library

Status: decided Oct 30, 2026. The back office (`/bo`) and the storefront (`/fo`) are built on **[Meta Astryx](https://github.com/meta/astryx)**, Meta's open-source React design system (MIT).

Links: [GitHub](https://github.com/meta/astryx) · [Docs](https://astryx.atmeta.com) · [Storybook](https://astryx.atmeta.com/storybook/) · [Sandbox](https://astryx.atmeta.com/sandbox/) · [npm `@astryxdesign/core`](https://www.npmjs.com/package/@astryxdesign/core)

## 1. Why Astryx

- **Meta end to end.** The model is Meta's (Muse Glimmer, Muse Spark) and so is the UI system. Astryx ran inside Meta for eight years before it was opened in June 2026.
- **Built for agents as well as people.** The `astryx` CLI lists components, templates, themes, and docs through a typed JSON API, so the coding agents on our team build with the same tooling we do.
- **Fits the back office.** Over 150 accessible components, with page templates for table pages, detail pages, form wizards, and navigation.
- **Our look, not a template look.** A theme is a set of CSS custom property overrides. When a component needs more than that, `swizzle` copies its source into our project.
- **No build setup.** It ships pre-built CSS and JS: three stylesheet imports and a theme provider in Next.js. StyleX stays inside the package.
- **License.** MIT, so it can be a dependency of this Apache-2.0 public repository.

## 2. Options considered

Weekly npm downloads and GitHub stars as of Oct 30, 2026.

| Library | Weekly downloads | Stars | License | Why not |
|---|---|---|---|---|
| **Meta Astryx** | 446k | 13.8k | MIT | Chosen |
| Mantine | 3.7M | 31.9k | MIT | Fallback (§5) |
| MUI | 13.9M | 99.3k | MIT | Material look is recognizable at a glance |
| Ant Design | 5.0M | 100.0k | MIT | Strong admin-template look |
| Base UI / React Aria | 26.0M / 7.5M | 11.3k / 16.0k | MIT / Apache-2.0 | Headless: we would style every component ourselves |
| shadcn/ui, HeroUI | — / 1.1M | 126.9k / 31.1k | MIT / Apache-2.0 | Reads as the default look of AI-generated apps |
| Medusa UI | 294k | 37.0k (repo) | MIT | Good for commerce admin, weak for the storefront |

Paid kits were ruled out: their licenses do not allow publishing the source in a public repository.

## 3. Setup (Next.js)

```bash
npm install @astryxdesign/core @astryxdesign/theme-neutral @stylexjs/stylex
npm install -D @astryxdesign/cli
```

`web/src/app/globals.css` (the order matters: reset → components → theme):

```css
@import '@astryxdesign/core/reset.css';
@import '@astryxdesign/core/astryx.css';
@import '@astryxdesign/theme-neutral/theme.css';
```

Then wrap the app in the theme provider as in the [core README](https://github.com/meta/astryx/blob/main/packages/core/README.md#quick-start). Requires React 19 or later; Next.js App Router already uses it.

Add the CLI as a script so that people and agents call it the same way:

```json
"scripts": { "astryx": "node node_modules/@astryxdesign/cli/clients/cli/bin/astryx.mjs" }
```

`npm run astryx -- component --list`, `npm run astryx -- template --list`, `npm run astryx -- search "<query>"`.

In this repository the setup is done (`web/`): versions are pinned, the CSS is imported in `src/app/layout.tsx`, and the theme is wired the Astryx way with `npm run astryx -- theme add neutral`, which generates `src/astryx-themes.ts` used by `src/app/providers.tsx`. `npm run astryx -- init --features agents,theme` also wrote `web/AGENTS.md` (the Astryx guide for coding agents) and the theme template. How the AI layer sits on top: [frontend/README.md](frontend/README.md).

## 4. Rules

- **One UI block, one component.** Each UI block in [api/chat.md](api/chat.md) §3 is one React component under `web/`, built from Astryx parts. The agent picks blocks; it never writes markup.
- **Brand.** The site is **Ailism Commerce**. Our theme `ailism` (`web/src/theme/ailism.ts`) extends Neutral with the Ailism red (`#D90117`) as the accent and Pretendard as the typeface; it is built with `npm run astryx -- theme build src/theme/ailism.ts` and provided in `src/app/providers.tsx`. The wordmark, the assistant's face (`/brand/`), the voice of the brand (two short beats: "Say it once. See it through.") and the footer live in `web/src/components/brand/Ailism.tsx`. The header follows the Ailism bar: the wordmark with the product name muted beside it, plain menu items, actions on the right.
- **Look.** Theme `ailism` (Neutral with the Ailism accent), white backgrounds. No gradients, glows, decorative shadows, colored side bars, or sparkle icons. Changes to the look go into our theme file as CSS custom properties, not into per-component overrides.
- **Overrides.** Small adjustments use `className` with CSS modules. Swizzle a component only when the theme and `className` cannot do it, and note the reason in the commit.
- **Templates first.** Start back-office pages from an Astryx template (`npm run astryx -- build`) and change it, instead of laying out pages by hand.
- **Charts.** Astryx charts (`@astryxdesign/charts`, `@astryxdesign/vega`) are only on the `canary` tag. Use them only if a scene needs a chart, and pin the version.

## 5. Risks and fallback

- Astryx is in beta (0.7.x). Pin exact versions in `package.json`; do not upgrade during the hackathon unless a fix needs it.
- If a blocker shows up (React 19 or Next.js incompatibility, a missing component we cannot build from parts), switch to **Mantine** (MIT, 3.7M weekly downloads, neutral look). Because the agent only speaks UI blocks, the switch stays inside `web/`.
- If the data table we need for `product_table` is missing a feature, pair Astryx with [TanStack Table](https://tanstack.com/table) (MIT, headless) and render it with Astryx parts.
