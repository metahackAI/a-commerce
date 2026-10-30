import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';
import type {ReactNode} from 'react';

import {AddressList, CouponList, LoyaltyView, MemoryList, MessageList, SignOutButton, WishlistView} from '@/components/fo/AccountViews';
import {ErrorNotice} from '@/components/fo/ErrorNotice';
import type {Page} from '@/lib/api';

import {words} from '../_lib/format';
import {load, shopper, type Loaded} from '../_lib/server';
import type {Address, LoyaltyBalance, MemberCoupon, Preference, Profile, StoreMessage, WishlistItem} from '../_lib/types';

export const metadata: Metadata = {title: 'Account'};

function Section({title, subtitle, children}: {title: string; subtitle?: string; children: ReactNode}) {
  return (
    <Card>
      <VStack gap={3}>
        <VStack gap={0.5}>
          <Heading level={2}>{title}</Heading>
          {subtitle ? (
            <Text type="supporting" color="secondary">
              {subtitle}
            </Text>
          ) : null}
        </VStack>
        {children}
      </VStack>
    </Card>
  );
}

function Loadable<T>({loaded, children}: {loaded: Loaded<T>; children: (data: T) => ReactNode}) {
  return loaded.ok ? <>{children(loaded.data)}</> : <ErrorNotice error={loaded.error} />;
}

export default async function AccountPage() {
  const who = await shopper();
  if (!who.isShopper) {
    return (
      <Layout
        height="auto"
        contentWidth={1000}
        content={
          <LayoutContent padding={6}>
            <VStack gap={4}>
              <Heading level={1}>Your account</Heading>
              <Banner
                status="info"
                title={who.isGuest ? 'You are browsing as a guest' : 'You are not signed in'}
                description="Sign in to see your sizes, what Northwind remembers, your wishlist, coupons and orders."
                endContent={<Button label="Sign in" variant="primary" size="sm" href="/login?next=%2Faccount" />}
              />
            </VStack>
          </LayoutContent>
        }
      />
    );
  }

  const [profile, memories, addresses, wishlist, coupons, loyalty, messages] = await Promise.all([
    load<Profile>('/v1/customer/people/me'),
    load<Page<Preference>>('/v1/customer/people/me/preferences', {limit: 50}),
    load<Page<Address>>('/v1/customer/people/me/addresses', {limit: 20}),
    load<Page<WishlistItem>>('/v1/customer/people/me/wishlist', {limit: 50}),
    load<Page<MemberCoupon>>('/v1/promotion/customers/me/coupons', {limit: 50}),
    load<LoyaltyBalance>('/v1/promotion/customers/me/loyalty'),
    load<Page<StoreMessage>>('/v1/messaging/notifications', {customer_id: 'me', limit: 10}),
  ]);
  const p = profile.ok ? profile.data : null;

  return (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            <HStack gap={3} vAlign="center" justify="between" wrap="wrap">
              <VStack gap={0.5}>
                <Heading level={1}>{p ? `Hi, ${p.name}` : 'Your account'}</Heading>
                {p?.tier_id ? (
                  <Text color="secondary">{p.tier_id === 'member' ? 'Member' : `${words(p.tier_id)} member`}</Text>
                ) : null}
              </VStack>
              <HStack gap={2}>
                <Button label="Your orders" href="/orders" />
                <SignOutButton />
              </HStack>
            </HStack>
            {!profile.ok ? <ErrorNotice error={profile.error} /> : null}
            <Grid columns={{minWidth: 320, max: 2}} gap={5} align="start">
              <Section title="What Northwind knows about you" subtitle="Remembered from your conversations. Undo any of them; only you can.">
                <VStack gap={3}>
                  {p?.sizes ? (
                    <MetadataList>
                      <MetadataListItem label="Tops">{p.sizes.tops ?? 'Not set'}</MetadataListItem>
                      <MetadataListItem label="Bottoms">{p.sizes.bottoms ?? 'Not set'}</MetadataListItem>
                      <MetadataListItem label="Shoes">{p.sizes.shoes ?? 'Not set'}</MetadataListItem>
                      {p.card ? <MetadataListItem label="Card">{p.card.label}</MetadataListItem> : null}
                    </MetadataList>
                  ) : null}
                  <Loadable loaded={memories}>{d => <MemoryList memories={d.items} />}</Loadable>
                </VStack>
              </Section>
              <Section title="Wishlist">
                <Loadable loaded={wishlist}>{d => <WishlistView initial={d.items} />}</Loadable>
              </Section>
              <Section title="Addresses">
                <Loadable loaded={addresses}>{d => <AddressList addresses={d.items} />}</Loadable>
              </Section>
              <Section title="Messages from Northwind" subtitle="Emails and notices about your orders and returns, newest first.">
                <Loadable loaded={messages}>{d => <MessageList messages={d.items} />}</Loadable>
              </Section>
              <Section title="Coupons and rewards">
                <VStack gap={4}>
                  <Loadable loaded={loyalty}>{d => <LoyaltyView balance={d} />}</Loadable>
                  <Loadable loaded={coupons}>{d => <CouponList coupons={d.items} />}</Loadable>
                </VStack>
              </Section>
            </Grid>
          </VStack>
        </LayoutContent>
      }
    />
  );
}
