// Data of the back-office UI blocks (docs/api/chat.md §3.3), hand-written from the "data 요지"
// column until the generated block schema (`ui-blocks.schema.json`) exists. The chat spec names
// the fields but not every inner shape, so inner values are typed loosely and rendered through
// the formatting helpers; every number still comes from the block as sent.

import type {Check, LeftOut, Money, TaskOrigin, TaskState} from './types';

/** A value the agent may send as a plain string or as a small object with a headline. */
export type Understood = string | {text?: string; title?: string; label?: string; value?: unknown; detail?: string | null};

export interface ChangeSetGroup {
  op: string;
  label: string;
  summary?: string | null;
  rows: Record<string, unknown>[];
  more?: number | string | null;
}

export interface ChangeSetBlockData {
  change_set_id: string;
  number?: number;
  version: number;
  state: string;
  request?: string[] | string;
  understood?: Partial<Record<'who' | 'products' | 'benefit' | 'when' | 'where', Understood>>;
  checks?: Check[];
  groups?: ChangeSetGroup[];
  left_out?: LeftOut[];
  preview?: {segments?: string[]; selected?: string | null; content?: unknown} | null;
  after_approval?: (string | {label?: string; text?: string; tool?: string})[];
  undo?: {until: string | null} | null;
}

export interface TaskListBlockData {
  groups: {
    state: TaskState;
    items: {task_id: string; title: string; origin: TaskOrigin; at: string; summary?: string | null; tag?: string | null; change_set_id?: string | null}[];
  }[];
  agents?: {name: string; stats?: unknown}[];
}

export interface RefundReviewBlockData {
  summary?: {
    waiting?: number;
    refund?: {count?: number; amount?: Money | null};
    ask_photos?: number;
    decline?: number;
  };
  requests?: {
    claim_id?: string;
    order_number: string;
    item: string;
    amount?: Money | null;
    reason?: string;
    evidence?: unknown;
    day?: number | string | null;
    suggested?: string;
    because?: string | null;
  }[];
  checks?: Check[];
  messages?: (string | {to?: string; kind?: string; order_number?: string; text?: string; channel?: string})[];
  change_set_id?: string | null;
}

export interface ProductDraftsBlockData {
  progress?: {total?: number; ready?: number; needs_you?: number; working?: number};
  drafts?: {
    draft_id?: string;
    name: string;
    source_code?: string | null;
    cost?: Money | number | null;
    price?: Money | number | null;
    sizes?: string[] | string | null;
    status?: string;
    note?: string | null;
  }[];
  rules?: (string | {rule?: string; text?: string})[];
  checks?: Check[];
}

export interface SalesExplainerBlockData {
  summary?: {revenue?: Money | null; baseline?: Money | null; difference?: Money | null; orders?: number | string | null; [k: string]: unknown};
  series?: {date: string; revenue: Money | number | null}[];
  causes?: {cause: string; evidence?: string | null; estimated?: Money | number | string | null}[];
  next?: {label: string; request: string}[];
}

export interface ProductTableBlockData {
  columns?: (string | {key: string; label?: string; header?: string})[];
  rows?: Record<string, unknown>[];
  total?: number;
}

export interface ActionLogBlockData {
  entries?: {
    id?: string;
    at?: string;
    actor?: unknown;
    action_type?: string;
    tool?: string | null;
    object?: unknown;
    status?: string;
    result?: unknown;
  }[];
}
