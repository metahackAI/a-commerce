'use client';

// "What Ailism knows about you" and the shopper's own lists: remembered preferences (each with
// a person-only Undo), addresses, wishlist, coupons and loyalty balance. Everything shown is what
// get_my_profile, get_preferences, list_addresses, list_wishlist, list_member_coupons and
// get_loyalty_balance returned.

import {Button} from '@astryxdesign/core/Button';
import {Icon} from '@astryxdesign/core/Icon';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {Text} from '@astryxdesign/core/Text';
import {LightBulbIcon} from '@heroicons/react/24/outline';
import {useRouter} from 'next/navigation';
import {useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {day, longDate, money, price, words} from '@/app/(storefront)/_lib/format';
import type {Address, LoyaltyBalance, MemberCoupon, Preference, RestockAlert, StoreMessage, Subscription, WishlistItem} from '@/app/(storefront)/_lib/types';
import type {ApiErrorBody} from '@/lib/api';
import {signOut} from '@/lib/session';
import {PersonButton} from '@/components/PersonButton';

import {AskButton} from './Ask';
import {ProductThumb, productHref} from './ProductThumb';
import {ErrorNotice} from './ErrorNotice';
import {errorBody, write} from './write';
import {StatusTag} from './StatusTag';

function MemoryRow({memory}: {memory: Preference}) {
  const router = useRouter();
  const [gone, setGone] = useState(false);
  const target = useAiTarget({kind: 'memory_note', label: memory.text, ids: {memory_id: memory.id}, term: 'acom:Preference'});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      startContent={<Icon icon={LightBulbIcon} size="sm" color="secondary" />}
      label={gone ? <Text hasStrikethrough color="secondary">{memory.text}</Text> : memory.text}
      description={[words(memory.key), memory.created_at ? `remembered ${longDate(memory.created_at)}` : null, gone ? 'forgotten' : null].filter(Boolean).join(' · ')}
      endContent={
        gone ? undefined : (
          <HStack gap={1} vAlign="center">
            <AskButton target={target} label={memory.text} />
            <PersonButton
              action={{
                label: 'Undo',
                method: 'POST',
                path: `/v1/customer/me/memories/${encodeURIComponent(memory.id)}/undo`,
                confirm: `Forget “${memory.text}”? Ailism stops using it.`,
              }}
              variant="secondary"
              onDone={() => {
                setGone(true);
                router.refresh();
              }}
            />
          </HStack>
        )
      }
    />
  );
}

export function MemoryList({memories}: {memories: Preference[]}) {
  if (memories.length === 0) {
    return <Text color="secondary">Nothing yet. When you tell Ailism your size or what matters to you, it shows up here and you can undo it.</Text>;
  }
  return (
    <List hasDividers>
      {memories.map(m => (
        <MemoryRow key={m.id} memory={m} />
      ))}
    </List>
  );
}

function AddressRow({address}: {address: Address}) {
  const target = useAiTarget({kind: 'address', label: address.label ?? address.street_address, ids: {address_id: address.id}, term: 'schema:PostalAddress'});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={
        <HStack gap={2} vAlign="center">
          <Text weight="semibold">{address.label ?? 'Address'}</Text>
          {address.is_primary ? <StatusTag label="Primary" /> : null}
        </HStack>
      }
      description={[address.street_address, address.address_locality, address.address_region, address.postal_code, address.address_country].filter(Boolean).join(', ')}
    />
  );
}

export function AddressList({addresses}: {addresses: Address[]}) {
  if (addresses.length === 0) return <Text color="secondary">No saved addresses.</Text>;
  return (
    <List hasDividers>
      {addresses.map(a => (
        <AddressRow key={a.id} address={a} />
      ))}
    </List>
  );
}

function WishlistRow({item, onRemoved}: {item: WishlistItem; onRemoved: (id: string) => void}) {
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const target = useAiTarget({kind: 'wishlist_item', label: item.name, ids: {product_id: item.product_id}, term: 'schema:Product'});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      startContent={<ProductThumb productId={item.product_id} name={item.name} image={item.image} size={56} />}
      label={
        <Link href={`/p/${encodeURIComponent(item.product_id)}`}>
          <Text weight="semibold">{item.name}</Text>
        </Link>
      }
      description={
        <VStack gap={1}>
          <Text type="supporting" color="secondary">
            {[price(item.price, item.price_currency), item.availability ? words(item.availability) : null, item.added_at ? `saved ${longDate(item.added_at)}` : null]
              .filter(Boolean)
              .join(' · ')}
          </Text>
          {error ? <ErrorNotice error={error} /> : null}
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          <AskButton target={target} label={item.name} />
          <Button
            label="Remove"
            size="sm"
            variant="ghost"
            clickAction={async () => {
              setError(null);
              try {
                await write('DELETE', `/v1/customer/people/me/wishlist/${encodeURIComponent(item.product_id)}`, {});
                onRemoved(item.product_id);
              } catch (e) {
                setError(errorBody(e));
              }
            }}
          />
        </HStack>
      }
    />
  );
}

export function WishlistView({initial}: {initial: WishlistItem[]}) {
  const [items, setItems] = useState(initial);
  if (items.length === 0) {
    return (
      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text color="secondary">Nothing saved yet.</Text>
        <Button label="Browse products" size="sm" variant="ghost" href="/search" />
      </HStack>
    );
  }
  return (
    <List hasDividers>
      {items.map(i => (
        <WishlistRow key={i.product_id} item={i} onRemoved={id => setItems(list => list.filter(x => x.product_id !== id))} />
      ))}
    </List>
  );
}

const CYCLE: Record<string, string> = {every_week: 'Every week', every_2_weeks: 'Every two weeks', every_month: 'Every month'};

function SubscriptionRow({sub}: {sub: Subscription}) {
  const first = sub.lines[0];
  const what = sub.lines.map(l => `${l.name}${[l.color, l.size ? `size ${l.size}` : null].filter(Boolean).length ? ` (${[l.color, l.size].filter(Boolean).join(', ')})` : ''}${l.quantity > 1 ? ` × ${l.quantity}` : ''}`).join(', ');
  const target = useAiTarget({kind: 'subscription', label: `${what} · ${CYCLE[sub.billing_cycle] ?? words(sub.billing_cycle)}`, ids: {subscription_id: sub.id}, term: 'acom:Subscription'});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      startContent={first ? <ProductThumb productId={first.product_id} name={first.name} image={first.image} color={first.color} size={56} /> : undefined}
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          {first ? (
            <Link href={productHref(first.product_id, first.color)}>
              <Text weight="semibold">{what}</Text>
            </Link>
          ) : (
            <Text weight="semibold">{what}</Text>
          )}
          <StatusTag label={words(sub.status)} tone={sub.status === 'active' ? 'success' : 'neutral'} />
        </HStack>
      }
      description={[CYCLE[sub.billing_cycle] ?? words(sub.billing_cycle), sub.status === 'active' ? `next ${longDate(sub.next_charge_date)}` : null, money(sub.amount), sub.payment?.label ?? null]
        .filter(Boolean)
        .join(' · ')}
      endContent={<AskButton target={target} label={what} />}
    />
  );
}

export function SubscriptionList({subscriptions}: {subscriptions: Subscription[]}) {
  if (subscriptions.length === 0) return <Text color="secondary">No subscriptions. Ask Ailism to send something you buy again and again.</Text>;
  return (
    <List hasDividers>
      {subscriptions.map(s => (
        <SubscriptionRow key={s.id} sub={s} />
      ))}
    </List>
  );
}

function RestockRow({alert}: {alert: RestockAlert}) {
  const options = [alert.color, alert.size ? `size ${alert.size}` : null].filter(Boolean).join(', ');
  const back = alert.availability === 'in_stock';
  const target = useAiTarget({kind: 'restock_alert', label: `${alert.product_name}${options ? ` (${options})` : ''}`, ids: {restock_alert_id: alert.id, product_id: alert.product_id}});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      startContent={<ProductThumb productId={alert.product_id} name={alert.product_name} image={alert.image} color={alert.color} size={56} />}
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Link href={productHref(alert.product_id, alert.color)}>
            <Text weight="semibold">{`${alert.product_name}${options ? ` (${options})` : ''}`}</Text>
          </Link>
          <StatusTag label={back ? 'Back in stock' : 'Waiting'} tone={back ? 'success' : 'neutral'} />
        </HStack>
      }
      description={[
        `by ${words(alert.channel)}`,
        alert.notified_at ? `told ${longDate(alert.notified_at)}` : `since ${longDate(alert.start_time)}`,
        `until ${longDate(alert.expires)}`,
      ].join(' · ')}
      endContent={<AskButton target={target} label={alert.product_name} />}
    />
  );
}

export function RestockAlertList({alerts}: {alerts: RestockAlert[]}) {
  if (alerts.length === 0) return <Text color="secondary">No back-in-stock alerts. Ask for one on a sold-out size.</Text>;
  return (
    <List hasDividers>
      {alerts.map(a => (
        <RestockRow key={a.id} alert={a} />
      ))}
    </List>
  );
}

function CouponRow({coupon}: {coupon: MemberCoupon}) {
  const target = useAiTarget({kind: 'coupon', label: coupon.code ?? coupon.name ?? coupon.coupon_id, ids: {member_coupon_id: coupon.id}, term: 'acom:Coupon'});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={coupon.code ?? coupon.name ?? coupon.coupon_id}
      description={[coupon.name && coupon.code ? coupon.name : null, coupon.valid_through ? `until ${day(coupon.valid_through)}` : null, coupon.order_id ? `used on ${coupon.order_id}` : null]
        .filter(Boolean)
        .join(' · ')}
      endContent={<StatusTag label={words(coupon.status)} tone={coupon.status === 'available' || coupon.status === 'issued' ? 'success' : 'neutral'} />}
    />
  );
}

export function CouponList({coupons}: {coupons: MemberCoupon[]}) {
  if (coupons.length === 0) return <Text color="secondary">No coupons right now.</Text>;
  return (
    <List hasDividers>
      {coupons.map(c => (
        <CouponRow key={c.id} coupon={c} />
      ))}
    </List>
  );
}

export function LoyaltyView({balance}: {balance: LoyaltyBalance}) {
  const target = useAiTarget({kind: 'loyalty_balance', label: `Loyalty balance ${money(balance.balance)}`, ids: {customer_id: balance.customer_id}});
  return (
    <VStack gap={1} ref={target.ref} {...target.props}>
      <Text type="supporting" color="secondary">
        Reward balance
      </Text>
      <HStack gap={2} vAlign="center">
        <Text type="large" weight="semibold" hasTabularNumbers>
          {money(balance.balance)}
        </Text>
        <AskButton target={target} label="my loyalty balance" />
      </HStack>
      {balance.expiring_soon.map(e => (
        <Text key={e.expires} type="supporting" color="secondary">
          {money(e.amount)} expires {day(e.expires)}
        </Text>
      ))}
    </VStack>
  );
}

export function SignOutButton() {
  const router = useRouter();
  return (
    <Button
      label="Sign out"
      variant="secondary"
      clickAction={async () => {
        await signOut().catch(() => undefined);
        router.push('/');
        router.refresh();
      }}
    />
  );
}

function MessageRow({message}: {message: StoreMessage}) {
  const target = useAiTarget({
    kind: 'message',
    label: `${message.headline} (${longDate(message.date_sent)})`,
    ids: {message_id: message.id, ...(message.about ? {about: message.about} : {})},
    term: 'schema:Message',
  });
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={message.headline}
      description={`${message.text} · ${longDate(message.date_sent)}`}
      endContent={<AskButton target={target} label={message.headline} />}
    />
  );
}

/** What the store sent the shopper (list_notifications with `me`). */
export function MessageList({messages}: {messages: StoreMessage[]}) {
  if (messages.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        Nothing yet. Order and return updates show up here.
      </Text>
    );
  }
  return (
    <List hasDividers>
      {messages.map(m => (
        <MessageRow key={m.id} message={m} />
      ))}
    </List>
  );
}
