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
});
