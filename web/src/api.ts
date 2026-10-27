// Typed calls to the A-Commerce API (server/a_commerce/api.py). Vite proxies /api to :8000.

export type Check = { rule: string; passed: boolean; title: string; detail: string };
export type Row = { product_id?: string; name: string; stock?: number; discount?: string; note?: string | null; was?: number; now?: number };
export type Group = {
  op: "update" | "create" | "exclude" | "schedule";
  title: string;
  summary: string;
  undo?: string;
  columns?: string[];
  rows?: Row[];
  total_rows?: number;
  fields?: Record<string, string>;
  preview?: { audiences: string[]; date: string; eyebrow: string; headline: string; cta: string };
};
export type ChangeSetView = {
  id: number;
  version: number;
  status: "waiting_for_approval" | "running" | "verifying" | "live" | "failed" | "undone";
  title: string;
  request: string[];
  understood: { label: string; value: string; detail: string }[];
  checks: Check[];
  checks_passed: number;
  checks_total: number;
  groups: Group[];
  after_approval: { tools: string[]; undo_until: string | null };
  verification: { all_match: boolean; changes: { tool: string; matches: boolean; detail: string }[] } | null;
};
export type RefundRow = {
  return_request_id: string; order_id: string; item: string; amount: number; reason: string; evidence: string;
  suggested: "refund" | "ask_for_photos" | "decline"; rule: string;
};
export type RefundView = {
  kind: "refund_review"; id: number; version: number; status: ChangeSetView["status"]; title: string; request: string[];
  summary: { waiting: number; refund: { count: number; amount: number }; ask_for_photos: number; decline: number };
  rows: RefundRow[]; checks: Check[]; checks_passed: number; checks_total: number;
  messages: { order_id: string; kind: string; text: string }[]; approve_label: string;
  after_approval: { tools: string[]; undo_until: null; text: string };
  verification: ChangeSetView["verification"];
};
export type AnyView = (ChangeSetView & { kind?: undefined }) | RefundView;

export type AgentStep = {
  type: "agent_step"; model: string; tool_calls: number; duration_ms: number; read_only: boolean; text: string;
  corrected?: { said: string; issues: string[] }; // the model's reply contradicted the change set; text is built from it instead
};
export type UiBlock = { type: "ui"; ui: string; block_id: string; title: string; data: AnyView };
export type Handoff = { type: "handoff"; from: string; to: string; reason: string };
export type Event = AgentStep | UiBlock | Handoff;
export type ChatResult = { ok: boolean; events: Event[]; change_set_id: number | null; error?: string };

export class ApiError extends Error {
  constructor(public code: string, message: string) {
    super(message);
  }
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(`/api${path}`, { headers: { "content-type": "application/json" }, ...init });
  const body = await r.json().catch(() => null);
  if (!r.ok) {
    const d = body?.detail;
    throw new ApiError(d?.code ?? String(r.status), d?.message ?? (typeof d === "string" ? d : r.statusText));
  }
  return body as T;
}

const post = <T,>(path: string, body: unknown) => req<T>(path, { method: "POST", body: JSON.stringify(body) });

export const api = {
  health: () => req<{ ok: boolean; clock: string; tools: number }>("/health"),
  setClock: (at: string) => post<{ clock: string }>("/demo/clock", { at }),
  reset: () => post<{ clock: string }>("/demo/reset", {}),
  operatorChat: (request: string, change_set_id: number | null) => post<ChatResult>("/bo/chat", { request, change_set_id }),
  approve: (id: number, version: number) => post<AnyView>(`/change-sets/${id}/approve`, { version }),
  undo: (id: number) => post<AnyView>(`/change-sets/${id}/undo`, {}),
};

// -- storefront --------------------------------------------------------------

export type Shopper = "jamie" | "mina" | "alex";
export type Price = { now: number; was: number | null; note: string | null };
export type Chip = { label: string; source?: "conversation" | "profile" };
export type CompareData = { products: { product_id: string; name: string; sub: string }[]; rows: { label: string; values: (string | Price)[] }[]; sort: string };
export type GalleryData = { items: { product_id: string; name: string; color: string; price: Price; facts: string[]; look: string | null }[] };
export type CartData = {
  order_preview_id: string; lines: { product_id: string; name: string; variant: string; qty: number; price: number }[];
  subtotal: number; benefit: { label: string; amount: number; coupon: string | null } | null; shipping: number; total: number;
  ship_to: string; arrives: string; pay_with: string; notice: string;
};
export type ReturnOptionsData = {
  order: { id: string; delivered: string; item: string; variant: string; amount: number };
  reason: string; chosen: "replacement" | "refund" | null; note: string;
  options: ({ kind: "replacement"; title: string; ships: string; arrives: string; you_pay: number }
    | { kind: "refund"; title: string; amount: number; to: string; confirmed_by: string; money_back: string })[];
  send_back: { cost: string; pickup: string };
};
export type FoBlock =
  | { type: "ui"; ui: "understood_chips"; data: { chips: Chip[] } }
  | { type: "ui"; ui: "compare_table"; why: string; data: CompareData }
  | { type: "ui"; ui: "look_gallery" | "product_list"; why: string; data: GalleryData }
  | { type: "ui"; ui: "cart"; data: CartData }
  | { type: "ui"; ui: "memory_note"; data: { memory_id: string; text: string } }
  | { type: "ui"; ui: "return_options"; data: ReturnOptionsData };
export type FoEvent = AgentStep | FoBlock;
export type Profile = { name: string | null; tier: string | null; preferences: { id: string; key: string; value: string; source: string }[] };

const as = (shopper: Shopper) => ({ "content-type": "application/json", "X-Shopper": shopper });

export const shop = {
  chat: (shopper: Shopper, message: string) =>
    req<{ events: FoEvent[] }>("/fo/chat", { method: "POST", headers: as(shopper), body: JSON.stringify({ message }) }),
  profile: (shopper: Shopper) => req<Profile>("/fo/profile", { headers: as(shopper) }),
  placeOrder: (shopper: Shopper, order_preview_id: string) =>
    req<{ order_id: string; total: number; coupon: string | null }>("/orders", { method: "POST", headers: as(shopper), body: JSON.stringify({ order_preview_id }) }),
  sendReturn: (shopper: Shopper, body: { order_id: string; option: string; reason: string; photos: number }) =>
    req<{ return_request_id: string; status: string; next: string }>("/returns", { method: "POST", headers: as(shopper), body: JSON.stringify(body) }),
  undoMemory: (shopper: Shopper, id: string) =>
    req<{ ok: boolean }>(`/memories/${id}/undo`, { method: "POST", headers: as(shopper) }),
};

export const usd = (n: number) => `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
