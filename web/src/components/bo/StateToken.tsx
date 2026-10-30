// A state word from docs/api/enums.md as a colored chip. The color says only "good", "needs a
// person", "in progress", "stopped"; the label is the word itself made readable.

import {Token} from '@astryxdesign/core/Token';

import {words} from '@/app/bo/_lib/format';

type TokenColor = 'default' | 'red' | 'orange' | 'yellow' | 'green' | 'teal' | 'cyan' | 'blue' | 'purple' | 'gray';

const GREEN = new Set([
  'live',
  'completed',
  'delivered',
  'done',
  'active',
  'in_stock',
  'payment_complete',
  'running_promotion',
  'passed',
  'matched',
  'on_track',
  'visible',
  'refund',
  'approved',
  'ready',
]);
const ORANGE = new Set([
  'waiting_for_approval',
  'needs_you',
  'requested',
  'pending_review',
  'photos_requested',
  'ask_for_photos',
  'paused',
  'at_risk',
  'payment_due',
  'back_order',
  'charge_due',
  'needs_approval',
  'potential',
  'in_grace',
  'open',
]);
const RED = new Set([
  'failed',
  'rejected',
  'declined',
  'decline',
  'out_of_stock',
  'problem',
  'blocked',
  'payment_declined',
  'payment_past_due',
  'delivery_failed',
  'mismatch',
]);
const BLUE = new Set([
  'running',
  'verifying',
  'working',
  'in_transit',
  'processing',
  'scheduled',
  'picked_up',
  'pickup_instructed',
  'inspected',
  'reship_requested',
  'shipped',
  'label_created',
  'packed',
  'ship_instructed',
  'pre_order',
]);

export function stateColor(state: string | null | undefined): TokenColor {
  if (!state) return 'gray';
  if (GREEN.has(state)) return 'green';
  if (ORANGE.has(state)) return 'orange';
  if (RED.has(state)) return 'red';
  if (BLUE.has(state)) return 'blue';
  return 'gray';
}

export function StateToken({state, label, size = 'sm'}: {state: string | null | undefined; label?: string; size?: 'sm' | 'md'}) {
  return <Token label={label ?? words(state)} color={stateColor(state)} size={size} />;
}
