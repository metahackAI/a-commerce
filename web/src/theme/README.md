# Theme

The app uses Astryx **Neutral** with white backgrounds (docs/frontend-ui.md §4), added with
`npm run astryx -- theme add neutral`, which generates `src/astryx-themes.ts` (do not edit it).

To change the look, author our theme from `theme.template.ts` (every `defineTheme` option is
annotated there, starting from `extends: neutralTheme`), build it with
`npm run astryx -- theme build <file>`, and add it with `npm run astryx -- theme add <slug>`.
Colors and sizes change only in the theme, never with `:root` overrides or per-component CSS.
