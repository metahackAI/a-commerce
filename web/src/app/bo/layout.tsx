import type {ReactNode} from 'react';

export const metadata = {title: {default: 'Back office', template: '%s · Back office'}};

// Back office (/bo). Sign-in (/bo/login) stands alone; everything else lives in the (console)
// group, whose layout checks the session and holds the side navigation, the AI layer and the
// agent dock.
export default function BackOfficeLayout({children}: {children: ReactNode}) {
  return children;
}
