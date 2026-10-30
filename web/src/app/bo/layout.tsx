import type {ReactNode} from 'react';
import {AppShell} from '@astryxdesign/core/AppShell';
import {SideNav, SideNavItem, SideNavSection} from '@astryxdesign/core/SideNav';
import {Text} from '@astryxdesign/core/Text';

import {AiProvider} from '@/ai/AiProvider';

export const metadata = {title: 'Back office'};

// Back office (/bo): operators talk; the canvas shows change sets, reviews, and explanations.
export default function BackOfficeLayout({children}: {children: ReactNode}) {
  return (
    <AiProvider surface="bo">
      <AppShell
        height="fill"
        variant="elevated"
        sideNav={
          <SideNav
            header={
              <Text type="label" weight="semibold">
                Northwind Apparel
              </Text>
            }>
            <SideNavSection title="Work">
              <SideNavItem label="Inbox" href="/bo" />
              <SideNavItem label="Change sets" href="/bo/change-sets" />
              <SideNavItem label="Activity" href="/bo/activity" />
            </SideNavSection>
            <SideNavSection title="Store">
              <SideNavItem label="Products" href="/bo/products" />
              <SideNavItem label="Promotions" href="/bo/promotions" />
              <SideNavItem label="Orders and returns" href="/bo/orders" />
            </SideNavSection>
          </SideNav>
        }>
        {children}
      </AppShell>
    </AiProvider>
  );
}
