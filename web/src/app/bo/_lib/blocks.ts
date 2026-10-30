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
    items: {
      task_id: string;
      title: string;
      origin: TaskOrigin;
      at: string;
      summary?: string | null;
      tag?: string | null;
      change_set_id?: string | null;
      /** The platform.yml watcher that keeps it up to date, if any. */
      watcher?: string | null;
      updated_at?: string;
    }[];
  }[];
  agents?: {name: string; stats?: unknown}[];
  /** Tasks in the store for the filter (`list_tasks` total); more than shown when the list was cut. */
  total?: number;
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
    /** Things a person should look at (for example 4 returns in the last 90 days). */
    flags?: {rule?: string; detail?: string}[];
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

/** `sales_explainer` as the agent builds it from `explain_sales_change` and `get_sales_trend`. */
export interface SalesExplainerBlockData {
  period?: {valid_from: string; valid_through: string; label?: string};
  summary?: {
    revenue?: Money | null;
    baseline?: Money | null;
    /** "Thursday average". */
    baseline_label?: string;
    difference?: Money | null;
    /** `difference / baseline`, 0–1 (negative for a drop). */
    change_rate?: number | null;
    basis?: string;
    /** "vs the last 4 Thursdays". */
    basis_label?: string;
    orders?: {count?: number | null; baseline?: number | null} | number | null;
  };
  series?: {date: string; revenue: Money | null}[];
  /** Causes with the data behind them, then the part left unexplained (`cause: unexplained`). */
  causes?: {
    cause: string;
    title?: string;
    detail?: string | null;
    evidence?: string | null;
    estimated?: Money | number | null;
    ids?: string[];
  }[];
  /** Next steps; each is also a `say` action whose `args.request` is the sentence it sends. */
  next?: {label: string; request: string; cause?: string}[];
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
