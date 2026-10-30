import type {Metadata} from 'next';
import type {ReactNode} from 'react';

// Astryx setup: reset, then components. The theme's CSS comes with src/astryx-themes.ts.
import '@astryxdesign/core/reset.css';
import '@astryxdesign/core/astryx.css';
import {Providers} from './providers';

export const metadata: Metadata = {
  title: {default: 'Northwind', template: '%s · Northwind'},
  description: 'A-Commerce: an AI-native store where people talk and the right screen appears.',
};

export default function RootLayout({children}: {children: ReactNode}) {
  return (
    <html lang="en">
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
