// Ailism Commerce: Astryx Neutral with the Ailism red as the accent and Pretendard as the
// typeface (loaded in app/layout.tsx). Build with `npm run astryx -- theme build src/theme/ailism.ts`.

import {defineTheme} from '@astryxdesign/core/theme';
import {neutralTheme} from '@astryxdesign/theme-neutral';

export const ailismTheme = defineTheme({
  name: 'ailism',
  extends: neutralTheme,
  typography: {
    body: {family: 'Pretendard Variable', fallbacks: 'Pretendard, -apple-system, system-ui, sans-serif'},
  },
  tokens: {
    // The red of the Ailism wordmark, and a lighter one that keeps its contrast on dark surfaces.
    '--color-accent': ['#D90117', '#FF5A68'],
    '--color-on-accent': ['#FFFFFF', '#FFFFFF'],
    '--focus-outline-color': 'var(--color-accent)',
  },
  adaptations: {
    rules: [
      {
        // Under a finger, controls get a hit area a thumb can find: 36/40/44 px instead of
        // 28/32/36 px (quantity buttons, date pickers, filter clears, segmented controls).
        when: {pointer: 'coarse'},
        value: {
          tokens: {
            '--size-element-sm': '36px',
            '--size-element-md': '40px',
            '--size-element-lg': '44px',
          },
        },
      },
    ],
  },
});
