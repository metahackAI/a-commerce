'use client';

// The back office's side navigation: every section, the current one selected, a way to ask the
// agent from anywhere, and who is signed in with a way to sign out.

import {useState} from 'react';
import {usePathname, useRouter} from 'next/navigation';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {SideNav, SideNavItem, SideNavSection} from '@astryxdesign/core/SideNav';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Avatar} from '@astryxdesign/core/Avatar';
import {Icon} from '@astryxdesign/core/Icon';
import {
  ArrowPathRoundedSquareIcon,
  BuildingStorefrontIcon,
  ChatBubbleOvalLeftEllipsisIcon,
  CircleStackIcon,
  ClipboardDocumentListIcon,
  InboxIcon,
  QueueListIcon,
  ReceiptPercentIcon,
  ScaleIcon,
  ShoppingBagIcon,
  TagIcon,
  UsersIcon,
} from '@heroicons/react/24/outline';

import {signOut} from '@/lib/session';

import {useAgentDock} from './AgentDock';

interface NavLink {
  label: string;
  href: string;
  icon: typeof InboxIcon;
}

const WORK: NavLink[] = [
  {label: 'Inbox', href: '/bo', icon: InboxIcon},
  {label: 'Change sets', href: '/bo/change-sets', icon: ClipboardDocumentListIcon},
  {label: 'Activity', href: '/bo/activity', icon: QueueListIcon},
];

const STORE: NavLink[] = [
  {label: 'Products', href: '/bo/products', icon: TagIcon},
  {label: 'Promotions', href: '/bo/promotions', icon: ReceiptPercentIcon},
  {label: 'Merchandising', href: '/bo/merchandising', icon: BuildingStorefrontIcon},
  {label: 'Orders', href: '/bo/orders', icon: ShoppingBagIcon},
  {label: 'Returns', href: '/bo/returns', icon: ArrowPathRoundedSquareIcon},
  {label: 'Customers', href: '/bo/customers', icon: UsersIcon},
];

const RULES: NavLink[] = [
  {label: 'Policies', href: '/bo/policies', icon: ScaleIcon},
  {label: 'Data dictionary', href: '/bo/data', icon: CircleStackIcon},
];

function isCurrent(pathname: string, href: string): boolean {
  if (href === '/bo') return pathname === '/bo' || pathname.startsWith('/bo/tasks');
  if (href === '/bo/returns') return pathname.startsWith('/bo/returns') || pathname.startsWith('/bo/claims');
  return pathname === href || pathname.startsWith(`${href}/`);
}

export function BoNav({userName, role}: {userName: string; role: string}) {
  const pathname = usePathname();
  const router = useRouter();
  const dock = useAgentDock();
  const [signingOut, setSigningOut] = useState(false);

  const section = (title: string, links: NavLink[]) => (
    <SideNavSection title={title}>
      {links.map(l => (
        <SideNavItem key={l.href} label={l.label} href={l.href} icon={l.icon} isSelected={isCurrent(pathname, l.href)} />
      ))}
    </SideNavSection>
  );

  return (
    <SideNav
      header={
        <VStack gap={0}>
          <Text type="label" weight="semibold">
            Northwind Apparel
          </Text>
          <Text type="supporting" color="secondary">
            Back office
          </Text>
        </VStack>
      }
      topContent={
        <Button
          label="Ask the agent"
          width="100%"
          icon={<Icon icon={ChatBubbleOvalLeftEllipsisIcon} />}
          onClick={() => dock.open()}
        />
      }
      footer={
        <HStack gap={2} vAlign="center">
          <Avatar name={userName} size="sm" />
          <StackItem size="fill">
            <VStack gap={0}>
              <Text type="label" maxLines={1}>
                {userName}
              </Text>
              <Text type="supporting" color="secondary" maxLines={1}>
                {role}
              </Text>
            </VStack>
          </StackItem>
          <Button
            label="Sign out"
            size="sm"
            variant="ghost"
            isLoading={signingOut}
            onClick={async () => {
              setSigningOut(true);
              try {
                await signOut();
              } catch {
                // Already signed out or the gateway is down: go to sign-in either way.
              }
              router.replace('/bo/login');
              router.refresh();
            }}
          />
        </HStack>
      }>
      {section('Work', WORK)}
      {section('Store', STORE)}
      {section('Rules and data', RULES)}
    </SideNav>
  );
}
