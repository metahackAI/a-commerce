'use client';

// The storefront frame (Ailism Commerce): the top navigation with the store's display categories, search, the
// assistant, who is signed in, and the cart; the assistant drawer; and below the phone
// breakpoint, AppShell's own compact bar and menu drawer (scene F6).

import {AppShell, useAppShellMobile} from '@astryxdesign/core/AppShell';
import {Badge} from '@astryxdesign/core/Badge';
import {Button} from '@astryxdesign/core/Button';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Icon} from '@astryxdesign/core/Icon';
import {HStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';
import {TopNav, TopNavItem} from '@astryxdesign/core/TopNav';
import {
  ChatBubbleLeftEllipsisIcon,
  MagnifyingGlassIcon,
  ScaleIcon,
  ShoppingBagIcon,
  UserCircleIcon,
} from '@heroicons/react/24/outline';
import {usePathname, useRouter} from 'next/navigation';
import type {ReactNode} from 'react';

import {words} from '@/app/(storefront)/_lib/format';
import {signOut} from '@/lib/session';

import {AilismFooter, AilismMark} from '@/components/brand/Ailism';

import {AssistantDrawer} from './AssistantDrawer';
import {StorefrontProvider, useStorefront, type ShopperInfo} from './StorefrontContext';

export interface NavCategory {
  id: string;
  name: string;
}

function CartItem() {
  const {cartLines} = useStorefront();
  const pathname = usePathname();
  const label = cartLines === 1 ? 'Cart, 1 line' : `Cart, ${cartLines} lines`;
  return (
    <TopNavItem label={label} href="/cart" isSelected={pathname === '/cart'}>
      <HStack gap={1} vAlign="center">
        <Icon icon={ShoppingBagIcon} size="sm" />
        <Text type="label" hasTabularNumbers>
          {cartLines}
        </Text>
      </HStack>
    </TopNavItem>
  );
}

function AccountItem() {
  const {isShopper, isGuest, name, tierId} = useStorefront();
  const {isMobile} = useAppShellMobile();
  const router = useRouter();

  if (!isShopper) {
    return (
      <TopNavItem
        label={isGuest ? 'Guest · Sign in' : 'Sign in'}
        href="/login"
        icon={<Icon icon={UserCircleIcon} size="sm" />}
        isIconOnly={isMobile}
      />
    );
  }
  const who = tierId ? `${name ?? 'You'} · ${words(tierId)}` : (name ?? 'Your account');
  return (
    <DropdownMenu
      button={{
        label: who,
        variant: 'ghost',
        size: 'sm',
        icon: <Icon icon={UserCircleIcon} />,
        isIconOnly: isMobile,
      }}
      hasChevron={!isMobile}
      alignment="end"
      items={[
        {label: 'Account', href: '/account'},
        {label: 'Orders', href: '/orders'},
        {
          label: 'Sign out',
          onClick: () => {
            void signOut()
              .catch(() => undefined)
              .finally(() => {
                router.push('/');
                router.refresh();
              });
          },
        },
      ]}
    />
  );
}

function EndContent() {
  const {openAssistant, compare} = useStorefront();
  const {isMobile} = useAppShellMobile();
  const pathname = usePathname();
  return (
    <HStack gap={1} vAlign="center">
      <TopNavItem label="Search" href="/search" icon={<Icon icon={MagnifyingGlassIcon} size="sm" />} isIconOnly isSelected={pathname === '/search'} />
      {compare.length > 0 ? (
        <TopNavItem label={`Compare ${compare.length}`} href="/compare" icon={<Icon icon={ScaleIcon} size="sm" />} isIconOnly={isMobile}>
          {isMobile ? undefined : (
            <HStack gap={1} vAlign="center">
              <Icon icon={ScaleIcon} size="sm" />
              <Text type="label">Compare</Text>
              <Badge label={compare.length} />
            </HStack>
          )}
        </TopNavItem>
      ) : null}
      {pathname === '/ask' ? null : (
        // A button, not a TopNavItem: TopNavItem always renders the router link (next/link), which
        // throws during server rendering when there is no href.
        <Button
          label="Ask Ailism"
          variant="ghost"
          size="sm"
          icon={<Icon icon={ChatBubbleLeftEllipsisIcon} size="sm" />}
          isIconOnly={isMobile}
          onClick={openAssistant}
        />
      )}
      {isMobile ? null : <TopNavItem label="Orders" href="/orders" isSelected={pathname.startsWith('/orders')} />}
      <AccountItem />
      <CartItem />
    </HStack>
  );
}

/** The store's display categories; on a phone (the menu drawer) also the shopper's own pages. */
function NavLinks({categories}: {categories: NavCategory[]}) {
  const pathname = usePathname();
  const {isMobile} = useAppShellMobile();
  return (
    <>
      {categories.map(c => (
        <TopNavItem key={c.id} label={c.name} href={`/c/${encodeURIComponent(c.id)}`} isSelected={pathname === `/c/${c.id}`} />
      ))}
      <TopNavItem label="Shop all" href="/search" isSelected={pathname === '/search'} />
      {isMobile ? (
        <>
          <TopNavItem label="Ask Ailism" href="/ask" isSelected={pathname === '/ask'} />
          <TopNavItem label="Orders" href="/orders" isSelected={pathname.startsWith('/orders')} />
          <TopNavItem label="Account" href="/account" isSelected={pathname === '/account'} />
        </>
      ) : null}
    </>
  );
}

function Shell({categories, children}: {categories: NavCategory[]; children: ReactNode}) {
  return (
    <AppShell
      height="fill"
      variant="surface"
      topNav={
        <TopNav
          label="Store"
          heading={
            <Link href="/" label="Ailism Commerce home">
              <AilismMark />
            </Link>
          }
          startContent={<NavLinks categories={categories} />}
          endContent={<EndContent />}
        />
      }>
      {children}
      <AilismFooter />
      <AssistantDrawer />
    </AppShell>
  );
}

export function StorefrontShell({
  shopper,
  cartLines,
  categories,
  children,
}: {
  shopper: ShopperInfo;
  cartLines: number;
  categories: NavCategory[];
  children: ReactNode;
}) {
  return (
    <StorefrontProvider shopper={shopper} cartLines={cartLines}>
      <Shell categories={categories}>{children}</Shell>
    </StorefrontProvider>
  );
}
