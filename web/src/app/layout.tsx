import type {Metadata} from 'next';
import type {ReactNode} from 'react';

// Astryx setup: reset, then components. The theme's CSS comes with the Ailism theme (providers.tsx).
import '@astryxdesign/core/reset.css';
import '@astryxdesign/core/astryx.css';
import {Providers} from './providers';

export const metadata: Metadata = {
  title: {default: 'Ailism Commerce', template: '%s · Ailism Commerce'},
  description: 'Ailism Commerce: say it once, see it through. You decide; Ailism prepares the rest.',
  applicationName: 'Ailism Commerce',
};

export default function RootLayout({children}: {children: ReactNode}) {
  return (
    <html lang="en">
      <head>
        {/* Pretendard, the typeface the Ailism theme names (src/theme/ailism.ts). */}
        <link
          rel="stylesheet"
          href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/variable/pretendardvariable-dynamic-subset.min.css"
        />
      </head>
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
