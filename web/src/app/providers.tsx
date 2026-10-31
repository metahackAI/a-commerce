'use client';

import type {ReactNode} from 'react';
import Link from 'next/link';
import {Theme} from '@astryxdesign/core/theme';
import {LinkProvider} from '@astryxdesign/core/Link';

// The Ailism theme (src/theme/ailism.ts, Neutral with the Ailism red), built with
// `npm run astryx -- theme build src/theme/ailism.ts`: the module and its CSS go together.
import {ailismTheme} from '@/theme/ailism';
import '@/theme/ailism.css';

/** Astryx theme and Next.js links for every Astryx component in the app. */
export function Providers({children}: {children: ReactNode}) {
  return (
    <Theme theme={ailismTheme} mode="light">
      <LinkProvider component={Link}>{children}</LinkProvider>
    </Theme>
  );
}
