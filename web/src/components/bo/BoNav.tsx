'use client';

// The back office's side navigation: the brand, Ask Ailism from any page (also ⌘K or Ctrl+K), the
// work with how many tasks need a decision, the store's sections with a section's sub-pages under
// it while one of its pages is open (as the Shopify admin shows them), the rules and data, and at
// the bottom the store's time (movable in demo mode) and who is signed in, with Sign out in their
// menu.

import {useEffect, type MouseEvent} from 'react';
import {usePathname, useRouter} from 'next/navigation';
import {HStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {SideNav, SideNavItem, SideNavSection} from '@astryxdesign/core/SideNav';
import {Avatar} from '@astryxdesign/core/Avatar';
import {Badge} from '@astryxdesign/core/Badge';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Icon} from '@astryxdesign/core/Icon';
import {Kbd} from '@astryxdesign/core/Kbd';
import {useMediaQuery} from '@astryxdesign/core/hooks';
import {
  ArrowRightStartOnRectangleIcon,
  BuildingStorefrontIcon,
  ChartBarIcon,
  ChevronUpDownIcon,
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
import {
  BuildingStorefrontIcon as BuildingStorefrontSolid,
  ChartBarIcon as ChartBarSolid,
  CircleStackIcon as CircleStackSolid,
  ClipboardDocumentListIcon as ClipboardDocumentListSolid,
  InboxIcon as InboxSolid,
  QueueListIcon as QueueListSolid,
  ReceiptPercentIcon as ReceiptPercentSolid,
  ScaleIcon as ScaleSolid,
  ShoppingBagIcon as ShoppingBagSolid,
  TagIcon as TagSolid,
  UsersIcon as UsersSolid,
} from '@heroicons/react/24/solid';

import {signOut} from '@/lib/session';

import {AILISM, AilismMark} from '@/components/brand/Ailism';
import {useAgentDock} from './AgentDock';
import {StoreClock} from './StoreClock';

interface NavLink {
  label: string;
  href: string;
  /** Outline, and filled while the page is open. Sub-pages have none. */
  icon?: typeof InboxIcon;
  selectedIcon?: typeof InboxIcon;
  /** Other paths that are this page: a task is the inbox's. */
  also?: string[];
  /** Sub-pages, shown under the section while one of its pages is open. */
  children?: NavLink[];
}

const WORK: NavLink[] = [
  {label: 'Inbox', href: '/bo', icon: InboxIcon, selectedIcon: InboxSolid, also: ['/bo/tasks']},
  {label: 'Change sets', href: '/bo/change-sets', icon: ClipboardDocumentListIcon, selectedIcon: ClipboardDocumentListSolid},
  {label: 'Activity', href: '/bo/activity', icon: QueueListIcon, selectedIcon: QueueListSolid},
];

const STORE: NavLink[] = [
  {
    label: 'Orders',
    href: '/bo/orders',
    icon: ShoppingBagIcon,
    selectedIcon: ShoppingBagSolid,
    children: [{label: 'Returns', href: '/bo/returns', also: ['/bo/claims']}],
  },
  {
    label: 'Products',
    href: '/bo/products',
    icon: TagIcon,
    selectedIcon: TagSolid,
    children: [
      {label: 'Drafts', href: '/bo/products/drafts'},
      {label: 'Categories', href: '/bo/categories'},
      {label: 'Brands', href: '/bo/brands'},
    ],
  },
  {label: 'Customers', href: '/bo/customers', icon: UsersIcon, selectedIcon: UsersSolid},
  {label: 'Promotions', href: '/bo/promotions', icon: ReceiptPercentIcon, selectedIcon: ReceiptPercentSolid},
  {label: 'Merchandising', href: '/bo/merchandising', icon: BuildingStorefrontIcon, selectedIcon: BuildingStorefrontSolid},
  {label: 'Insights', href: '/bo/insights', icon: ChartBarIcon, selectedIcon: ChartBarSolid},
];

const RULES: NavLink[] = [
  {label: 'Policies', href: '/bo/policies', icon: ScaleIcon, selectedIcon: ScaleSolid},
  {label: 'Data dictionary', href: '/bo/data', icon: CircleStackIcon, selectedIcon: CircleStackSolid},
];

function matches(pathname: string, link: NavLink): boolean {
  return [link.href, ...(link.also ?? [])].some(p => (p === '/bo' ? pathname === p : pathname === p || pathname.startsWith(`${p}/`)));
}

/** The section or one of its sub-pages is open. */
function isOpen(pathname: string, link: NavLink): boolean {
  return matches(pathname, link) || (link.children ?? []).some(c => matches(pathname, c));
}

/** This page is open; an open sub-page is selected instead of its section. */
function isCurrent(pathname: string, link: NavLink): boolean {
  return matches(pathname, link) && !(link.children ?? []).some(c => matches(pathname, c));
}

export function BoNav({userName, role, needsYou}: {userName: string; role: string; needsYou: number | null}) {
  const pathname = usePathname();
  const router = useRouter();
  const {open} = useAgentDock();

  // ⌘K or Ctrl+K asks Ailism from any page, as the row says.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        open();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open]);

  // The shortcut hint only where there is a keyboard to press it.
  const hasKeyboard = useMediaQuery('(hover: hover) and (pointer: fine)');

  // An open section keeps its filled icon while one of its sub-pages is selected, as in Polaris.
  const item = (link: NavLink) => (
    <SideNavItem
      key={link.href}
      label={link.label}
      href={link.href}
      icon={link.selectedIcon && isOpen(pathname, link) ? link.selectedIcon : link.icon}
      selectedIcon={link.selectedIcon}
      isSelected={isCurrent(pathname, link)}
      endContent={link.href === '/bo' && needsYou ? <Badge label={needsYou} /> : undefined}>
      {link.children && isOpen(pathname, link) ? link.children.map(item) : undefined}
    </SideNavItem>
  );

  return (
    <SideNav
      header={
        <HStack paddingInline={2} paddingBlock={1} vAlign="center">
          <Link href="/bo" aria-label={`${AILISM.name}, inbox`}>
            <AilismMark size="sm" />
          </Link>
        </HStack>
      }
      topContent={
        <SideNavItem
          label="Ask Ailism"
          icon={<Avatar src="/brand/ailism-avatar.png" name={AILISM.assistant} size={20} tooltip={false} />}
          endContent={hasKeyboard ? <Kbd keys="mod+k" /> : undefined}
          onClick={() => open()}
        />
      }
      footer={
        <>
          <StoreClock />
          <DropdownMenu
            placement="above"
            alignment="start"
            renderTrigger={({onClick, ...trigger}) => (
              <SideNavItem
                {...trigger}
                onClick={e => onClick(e as MouseEvent<HTMLElement>)}
                label={userName}
                icon={<Avatar name={userName} size={24} tooltip={false} />}
                endContent={<Icon icon={ChevronUpDownIcon} size="sm" color="secondary" />}
              />
            )}
            items={[
              {
                type: 'section',
                title: role,
                items: [
                  {
                    label: 'Sign out',
                    icon: <Icon icon={ArrowRightStartOnRectangleIcon} />,
                    onClick: async () => {
                      try {
                        await signOut();
                      } catch {
                        // Already signed out or the gateway is down: go to sign-in either way.
                      }
                      router.replace('/bo/login');
                      router.refresh();
                    },
                  },
                ],
              },
            ]}
          />
        </>
      }>
      <SideNavSection title="Work" isHeaderHidden>
        {WORK.map(item)}
      </SideNavSection>
      <SideNavSection title="Store">{STORE.map(item)}</SideNavSection>
      <SideNavSection title="Rules and data">{RULES.map(item)}</SideNavSection>
    </SideNav>
  );
}
