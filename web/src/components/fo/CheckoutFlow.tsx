'use client';

// Checkout: prepare_order makes a preview of the order from the cart, the saved address and the
// payment method (POST /v1/sales/orders/prepare?preview=true). The card shows exactly what the
// shopper will pay, the server's checks, and the person-only "Place order" button
// (POST /v1/sales/orders {preview_id}) — the agent can prepare this card but never press it.
//
// If the server's total differs from what the shopper saw (`server_total_matches_client` fails,
// or placing the order answers `estimate_changed`), the new numbers are shown and the shopper
// prepares again with them.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {Icon} from '@astryxdesign/core/Icon';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {RadioList, RadioListItem} from '@astryxdesign/core/RadioList';
import {Skeleton} from '@astryxdesign/core/Skeleton';
import {Heading, Text} from '@astryxdesign/core/Text';
import {CheckCircleIcon, ExclamationTriangleIcon} from '@heroicons/react/24/outline';
import {useRouter} from 'next/navigation';
import {useCallback, useEffect, useRef, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, money} from '@/app/(storefront)/_lib/format';
import type {Address, Cart, Money, OrderDraft, PaymentMethod, PlacedOrder} from '@/app/(storefront)/_lib/types';
import {preview as previewWrite, type ApiErrorBody, type Preview} from '@/lib/api';
import {PersonButton} from '@/components/PersonButton';

import {CartSummary} from './CartSummary';
import {ErrorNotice} from './ErrorNotice';
import {errorBody} from './write';

const TOTAL_CHECK = 'checkout_validation.server_total_matches_client';

const FIX_PAGE: Record<string, {label: string; href: string}> = {
  get_cart: {label: 'Review your cart', href: '/cart'},
  get_product: {label: 'Review your cart', href: '/cart'},
  get_payment_methods: {label: 'Choose another payment method', href: '#payment'},
};

function CheckRow({check}: {check: Preview['checks'][number]}) {
  const target = useAiTarget({kind: 'check_row', label: check.detail ?? check.rule, ids: {rule: check.rule}});
  const fix = check.fix_tool ? FIX_PAGE[check.fix_tool] : undefined;
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      startContent={
        <Icon
          icon={check.passed ? CheckCircleIcon : ExclamationTriangleIcon}
          color={check.passed ? 'success' : 'warning'}
          size="sm"
          label={check.passed ? 'Passed' : 'Needs attention'}
        />
      }
      label={check.detail ?? check.rule}
      endContent={!check.passed && fix ? <Button label={fix.label} size="sm" variant="ghost" href={fix.href} /> : undefined}
    />
  );
}

function draftOf(p: Preview): OrderDraft | null {
  const change = p.changes.find(c => c.entity === 'order' && c.op === 'create') ?? p.changes[0];
  const to = change?.to;
  return to && typeof to === 'object' ? (to as OrderDraft) : null;
}

export function CheckoutFlow({cart, addresses, methods}: {cart: Cart; addresses: Address[]; methods: PaymentMethod[]}) {
  const router = useRouter();
  const [addressId, setAddressId] = useState<string>((addresses.find(a => a.is_primary) ?? addresses[0])?.id ?? '');
  const [method, setMethod] = useState<string>((methods.find(m => m.default) ?? methods[0])?.payment_method ?? '');
  const [prepared, setPrepared] = useState<Preview | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [isPreparing, setPreparing] = useState(false);
  // What the shopper has seen and agrees to pay; the server compares its own total with it.
  const seenTotal = useRef<Money>(cart.total);
  const run = useRef(0);

  const prepare = useCallback(
    async (clientTotal: Money) => {
      if (!addressId) return;
      const mine = ++run.current;
      seenTotal.current = clientTotal;
      setPreparing(true);
      setError(null);
      try {
        const p = await previewWrite('/v1/sales/orders/prepare', {
          delivery_mode: 'saved',
          delivery_address_id: addressId,
          ...(method ? {payment_method: method} : {}),
          client_total: clientTotal,
        });
        if (mine === run.current) setPrepared(p);
      } catch (e) {
        if (mine === run.current) {
          setPrepared(null);
          setError(errorBody(e));
        }
      } finally {
        if (mine === run.current) setPreparing(false);
      }
    },
    [addressId, method],
  );

  // Prepare again whenever the address or payment method changes.
  useEffect(() => {
    void prepare(seenTotal.current);
  }, [prepare]);

  const draft = prepared ? draftOf(prepared) : null;
  const failed = prepared?.checks.filter(c => !c.passed) ?? [];
  const totalChanged = failed.some(c => c.rule === TOTAL_CHECK);
  const canPlace = !!prepared && !!draft && failed.length === 0 && !isPreparing;
  const card = useAiTarget({
    kind: 'checkout_card',
    label: draft ? `Order to pay ${money(draft.total)}` : 'Order being prepared',
    ids: {preview_id: prepared?.preview_id ?? 'pending'},
    term: 'schema:OrderAction',
  });

  return (
    <Grid columns={{minWidth: 320, max: 2}} gap={6} align="start">
      <VStack gap={5}>
        <VStack gap={2}>
          <Heading level={2}>Deliver to</Heading>
          {addresses.length === 0 ? (
            <Banner status="warning" title="No saved address" description="Add a delivery address to your account, then come back to check out." endContent={<Button label="Your account" size="sm" href="/account" />} />
          ) : (
            <RadioList label="Delivery address" isLabelHidden value={addressId} onChange={setAddressId}>
              {addresses.map(a => (
                <RadioListItem
                  key={a.id}
                  value={a.id}
                  label={a.label ?? a.street_address}
                  description={[a.street_address, a.address_locality, a.address_region, a.postal_code].filter(Boolean).join(', ')}
                />
              ))}
            </RadioList>
          )}
        </VStack>
        <VStack gap={2} id="payment">
          <Heading level={2}>Pay with</Heading>
          {methods.length === 0 ? (
            <Text color="secondary">No payment method works for every item in this cart.</Text>
          ) : (
            <RadioList label="Payment method" isLabelHidden value={method} onChange={setMethod}>
              {methods.map(m => (
                <RadioListItem key={m.payment_method} value={m.payment_method} label={m.label} description={m.default ? 'Your default' : undefined} />
              ))}
            </RadioList>
          )}
        </VStack>
        <VStack gap={2}>
          <Heading level={2}>In this order</Heading>
          <List hasDividers>
            {(draft?.lines ?? cart.lines).map((l, i) => (
              <ListItem
                key={l.cart_line_id ?? `${l.variant_id}-${i}`}
                label={l.name}
                description={[[l.color, l.size].filter(Boolean).join(' · '), `Qty ${l.quantity}`, `${money(l.price)} each`].filter(Boolean).join(' · ')}
              />
            ))}
          </List>
        </VStack>
      </VStack>

      <Card ref={card.ref} {...card.props}>
        <VStack gap={4}>
          <Heading level={2}>What you will pay</Heading>
          {error ? <ErrorNotice error={error} /> : null}
          {isPreparing && !draft ? (
            <VStack gap={2}>
              <Skeleton height={24} />
              <Skeleton height={24} index={1} />
              <Skeleton height={40} index={2} />
            </VStack>
          ) : draft ? (
            <VStack gap={3}>
              {totalChanged ? (
                <Banner
                  status="warning"
                  title={`The total is now ${money(draft.total)}`}
                  description={`You saw ${money(seenTotal.current)}. Prices or benefits changed since then. Check the numbers below, then prepare the order again with the new total.`}
                  endContent={<Button label="Use the new total" size="sm" variant="primary" clickAction={() => prepare(draft.total)} />}
                />
              ) : null}
              <CartSummary
                subtotal={draft.subtotal}
                discounts={draft.discounts ?? []}
                shipping={draft.shipping}
                total={draft.total}
                giftWrap={draft.gift_wrap}
                delivery={draft.delivery}
                payment={draft.payment}
                coupons={draft.coupons_recorded}
              />
              <List density="compact">
                {prepared?.checks.map(c => (
                  <CheckRow key={c.rule} check={c} />
                ))}
              </List>
              {canPlace && prepared ? (
                <VStack gap={2}>
                  <PersonButton
                    action={{
                      label: `Place order · ${money(draft.total)}`,
                      method: 'POST',
                      path: '/v1/sales/orders',
                      body: {preview_id: prepared.preview_id},
                      confirm: `Place this order and pay ${money(draft.total)}${draft.payment?.label ? ` with ${draft.payment.label}` : ''}? It ships to ${draft.delivery?.address_label ?? 'your address'}.`,
                    }}
                    onDone={result => {
                      const placed = result as PlacedOrder | null;
                      router.push(placed?.id ? `/orders/${encodeURIComponent(placed.id)}?placed=1` : '/orders');
                      router.refresh();
                    }}
                  />
                  <Text type="supporting" color="secondary">
                    Northwind can prepare your order, but only you can place it.
                    {prepared.expires_at ? ` Prepared until ${dateTime(prepared.expires_at)}.` : ''}
                  </Text>
                </VStack>
              ) : null}
              <HStack>
                <Button
                  label={prepared ? 'Totals changed? Prepare again' : 'Prepare the order'}
                  size="sm"
                  variant="ghost"
                  isLoading={isPreparing}
                  clickAction={() => prepare(draft.total)}
                />
              </HStack>
            </VStack>
          ) : (
            <Button label="Prepare the order" variant="primary" isDisabled={!addressId} clickAction={() => prepare(cart.total)} />
          )}
        </VStack>
      </Card>
    </Grid>
  );
}
