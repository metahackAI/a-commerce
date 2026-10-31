# Theme

The app uses our **`ailism`** theme (`ailism.ts`): Neutral with the Ailism red as the accent and
Pretendard as the typeface. After editing it, rebuild with
`npm run astryx -- theme build src/theme/ailism.ts`; `src/app/providers.tsx` imports the built module
and its CSS. The notes below describe the Neutral base it extends.

The app uses Astryx **Neutral** with white backgrounds (docs/frontend-ui.md §4), added with
`npm run astryx -- theme add neutral`, which generates `src/astryx-themes.ts` (do not edit it).

To change the look, author our theme from `theme.template.ts` (every `defineTheme` option is
annotated there, starting from `extends: neutralTheme`), build it with
`npm run astryx -- theme build <file>`, and add it with `npm run astryx -- theme add <slug>`.
Colors and sizes change only in the theme, never with `:root` overrides or per-component CSS.
