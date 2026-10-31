// A state word from docs/api/enums.md as a small colored dot and the word (Vercel's Geist Status
// Dot does this for a deployment's state), so a column of states reads as text and only the color
// of the dot differs. The color says only "good", "needs a person", "stopped" or neither (in
// progress, or any other word); the label is the word itself made readable.

import {HStack} from '@astryxdesign/core/Layout';
import {StatusDot, type StatusDotVariant} from '@astryxdesign/core/StatusDot';
import {Text} from '@astryxdesign/core/Text';

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

/** The dot's color for a state: in-progress states stay neutral, as the brand's accent is red. */
const DOT: Record<TokenColor, StatusDotVariant> = {
  default: 'neutral',
  red: 'error',
  orange: 'warning',
  yellow: 'warning',
  green: 'success',
  teal: 'neutral',
  cyan: 'neutral',
  blue: 'neutral',
  purple: 'neutral',
  gray: 'neutral',
};

export function StateToken({state, label, size = 'sm'}: {state: string | null | undefined; label?: string; size?: 'sm' | 'md'}) {
  const text = label ?? words(state);
  // The word is read out; the dot only repeats it in color.
  return (
    <HStack gap={1.5} vAlign="center">
      <StatusDot variant={DOT[stateColor(state)]} label={text} aria-hidden />
      <Text type={size === 'md' ? 'label' : 'body'} maxLines={1}>
        {text}
      </Text>
    </HStack>
  );
}
