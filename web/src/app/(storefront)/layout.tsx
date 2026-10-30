import type {ReactNode} from 'react';
import {AppShell} from '@astryxdesign/core/AppShell';
import {TopNav, TopNavItem} from '@astryxdesign/core/TopNav';
import {Text} from '@astryxdesign/core/Text';

import {AiProvider} from '@/ai/AiProvider';

// Storefront (/): shoppers talk; the canvas shows comparisons, galleries, the cart.
export default function StorefrontLayout({children}: {children: ReactNode}) {
  return (
    <AiProvider surface="fo">
      <AppShell
        height="fill"
        variant="surface"
        topNav={
          <TopNav
            label="Store"
            heading={
              <Text type="label" weight="semibold">
                NORTHWIND
              </Text>
            }
            startContent={
              <>
                <TopNavItem label="New in" href="/?c=new-in" />
                <TopNavItem label="Men" href="/?c=men" />
                <TopNavItem label="Women" href="/?c=women" />
              </>
            }
            endContent={<TopNavItem label="Orders" href="/orders" />}
          />
        }>
        {children}
      </AppShell>
    </AiProvider>
  );
}
