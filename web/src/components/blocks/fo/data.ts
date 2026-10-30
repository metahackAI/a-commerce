// Data of the storefront blocks (docs/api/chat.md §3.3), written by hand from the spec. The agent
// sends these; every field may be missing, and money may arrive as `{value, currency}`,
// `{price, price_currency}`, a bare number, or text the agent already formatted, so the blocks
// read them through `anyMoney`. Numbers are shown as they arrive, never recalculated.

/** Any money-like value a block may carry. */
export type MoneyLike = {value: number; currency: string} | {price: number; price_currency?: string} | number | string | null;

export interface LookingForData {
  chips: {label: string; source?: 'said' | 'profile' | 'his' | 'hers' | string; removable?: boolean}[];
}

export interface CompareTableData {
  products: {product_id: string; name: string; color?: string | null; material?: string | null; badge?: string | null; image?: unknown}[];
  rows: {label: string; values: unknown[]}[];
  prices: {price: MoneyLike; list_price?: MoneyLike; label?: string | null}[];
  sorted_by?: string | null;
}

export interface GalleryItem {
  product_id: string;
  name: string;
  price?: MoneyLike;
  list_price?: MoneyLike;
  image?: unknown;
  caption?: string | null;
}

export interface LookGalleryData {
  items: GalleryItem[];
}

export interface ProductListData {
  items: {product_id: string; name: string; price?: MoneyLike; list_price?: MoneyLike; image?: unknown; facts?: string[]}[];
}

export interface FitCheckData {
  product: {product_id?: string; name?: string; image?: unknown; facts?: string[]; summary?: string | null; price?: MoneyLike; list_price?: MoneyLike; note?: string | null};
  options: {label?: string; size: string; reason?: string | null; price?: MoneyLike; recommended?: boolean}[];
  measurements: {columns: string[]; rows: unknown[][] | {label: string; values: unknown[]}[]; caption?: string | null};
  reviews: {sample?: string | number | null; fit?: unknown[]; mentions?: unknown[]};
  used: unknown[];
  not_used: unknown[];
}

export interface GiftPickerData {
  picks: {product_id: string; name: string; price?: MoneyLike; list_price?: MoneyLike; why?: string | null; chosen_size?: string | null; image?: unknown; price_label?: string | null}[];
  gift_options: {wrap?: unknown; card_message?: string | null; receipt?: unknown; exchange_until?: string | null};
  delivery: {to?: string | null; arrives?: string | null; address_saved?: boolean | null};
  total: MoneyLike;
}

export interface CartBlockLine {
  cart_line_id?: string;
  product_id: string;
  variant_id?: string;
  name: string;
  color?: string | null;
  size?: string | null;
  quantity?: number;
  price?: MoneyLike;
  note?: string | null;
  image?: unknown;
}

export interface CartBlockData {
  lines: CartBlockLine[];
  subtotal: MoneyLike;
  discounts: {label: string; amount: MoneyLike}[];
  shipping: MoneyLike;
  total: MoneyLike;
  delivery: {address_label?: string | null; arrives?: string | null} | null;
  payment: {label?: string | null} | null;
}

export interface CheckoutCardData extends CartBlockData {
  preview_id: string;
  expires_at?: string | null;
  note?: string | null;
  /** A gift prepared by prepare_gift_order: its wrap line, options and recipient. */
  order_type?: 'gift' | string | null;
  gift_wrap?: MoneyLike;
  gift_options?: {card_message?: string | null; gift_receipt?: boolean; receipt_hides_prices?: boolean; exchange_until?: string | null} | null;
  recipient?: {name?: string; mode?: 'address_known' | 'recipient_accepts' | string} | null;
}

export interface ReturnOptionsData {
  order: {order_number?: string; item?: string; delivered_at?: string | null; photos?: number | unknown[] | null};
  options: {
    kind: 'replacement' | 'refund' | string;
    title?: string;
    facts?: string[];
    send_back?: {pickup?: unknown; drop_off?: unknown; methods?: string[]; pickup_window?: {start: string; end: string} | null} | null;
    you_pay?: MoneyLike;
    refund_amount?: MoneyLike;
  }[];
  selected?: string | null;
  steps?: (string | {label: string; detail?: string | null})[];
}

export interface OrderDetailData {
  order_id?: string;
  order_number: string;
  status: string;
  lines: {order_item_id?: string; id?: string; product_id?: string; name: string; size?: string | null; color?: string | null; quantity?: number; order_quantity?: number; price?: MoneyLike; status?: string | null}[];
  delivery: {address?: string | null; address_label?: string | null; arrives?: string | null; expected_arrival_until?: string | null; delivered_at?: string | null; state?: string | null; delivery_state?: string | null; tracking_number?: string | null; carrier?: string | null} | null;
  payment: {label?: string | null; payment_method?: string | null; card_last4?: string | null; payment_status?: string | null} | null;
  totals: {subtotal?: MoneyLike; discounts?: {label: string; amount: MoneyLike}[]; shipping?: MoneyLike; total?: MoneyLike} | null;
}

export interface MemoryNoteData {
  memory_id: string;
  text: string;
}

export interface ProfileCardData {
  /** Remembered preferences (with the memory the Forget button undoes) and saved sizes. */
  facts: {key: string; value: unknown; source?: string | null; memory_id?: string | null}[];
}
