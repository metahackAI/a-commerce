'use client';

// A short state word (an order status, availability, "Primary"). Badges are for counts only
// (web/AGENTS.md), so states are small tokens colored by their tone.

import {Token} from '@astryxdesign/core/Token';

export type Tone = 'success' | 'warning' | 'error' | 'info' | 'neutral';

const COLOR = {success: 'green', warning: 'orange', error: 'red', info: 'blue', neutral: 'gray'} as const;

export function StatusTag({label, tone = 'neutral'}: {label: string; tone?: Tone}) {
  return <Token label={label} size="sm" color={COLOR[tone]} />;
}
