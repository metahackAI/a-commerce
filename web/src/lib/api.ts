// Calls to the commerce tools through the gateway (`/api/v1/...`), same-origin, so the session
// cookie travels and no CORS is involved. State-changing requests carry the session's CSRF token
// (docs/backend/gateway.md §6). Errors arrive as `{"error": {...}}` (docs/api/errors.md) and are
// thrown as ApiError so a page can show the sentence and offer the suggested next step.
//
// The web never computes business numbers: prices, totals, stock and checks come from these
// responses as they are.

/** One error from the API (docs/api/errors.md). */
export interface ApiErrorBody {
  code: string;
  class: string;
  message: string;
  next_tool: string | null;
  detail?: Record<string, unknown>;
}

export class ApiError extends Error {
  readonly status: number;
  readonly body: ApiErrorBody;

  constructor(status: number, body: ApiErrorBody) {
    super(body.message);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
  }

  get code() {
    return this.body.code;
  }
}

/** A list read (docs/api/conventions.md §6). */
export interface Page<T> {
  items: T[];
  total: number;
  next_cursor?: string;
}

/** A write previewed with `?preview=true` (docs/api/conventions.md §7). */
export interface Preview {
  preview_id: string;
  expires_at: string | null;
  changes: {op: string; entity: string; id: string; from?: unknown; to?: unknown; note?: string}[];
  checks: {rule: string; passed: boolean; detail: string | null; fix_tool: string | null}[];
  needs_approval: boolean;
  approval_reason: string | null;
  change_set_id?: string;
}

/** A preview run with its `preview_id`. */
export interface CommitResult {
  ok: boolean;
  id: string;
  undo?: {by: 'person'; target: string; until: string | null};
  /** Anything else the write produced (`cart`, `memory`, …), per the tool's spec. */
  [slot: string]: unknown;
}

export type Query = Record<string, string | number | boolean | string[] | undefined | null>;

/** `?a=1&b=x&b=y`: arrays repeat the key, empty values are left out. */
export function toQuery(query?: Query): string {
  if (!query) return '';
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === '') continue;
    if (Array.isArray(value)) value.forEach(v => params.append(key, v));
    else params.append(key, String(value));
  }
  const s = params.toString();
  return s ? `?${s}` : '';
}

/** The CSRF token the gateway set for this session (cookie `acom_csrf`). */
export function csrfToken(): string | undefined {
  if (typeof document === 'undefined') return undefined;
  const match = document.cookie.split('; ').find(c => c.startsWith('acom_csrf='));
  return match ? decodeURIComponent(match.slice('acom_csrf='.length)) : undefined;
}

/** Headers for a state-changing request from the browser. */
export function writeHeaders(extra?: Record<string, string>): Record<string, string> {
  const headers: Record<string, string> = {'content-type': 'application/json', ...extra};
  const csrf = csrfToken();
  if (csrf) headers['x-csrf-token'] = csrf;
  return headers;
}

async function parse<T>(res: Response): Promise<T> {
  const text = await res.text();
  const body: unknown = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const err = (body as {error?: ApiErrorBody} | null)?.error ?? {
      code: 'upstream_unavailable',
      class: 'upstream_unavailable',
      message: `The request failed (${res.status}).`,
      next_tool: null,
    };
    throw new ApiError(res.status, err);
  }
  return body as T;
}

export interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  query?: Query;
  body?: unknown;
  headers?: Record<string, string>;
  signal?: AbortSignal;
}

/**
 * A browser call to a commerce path, given without the `/api` prefix: `api('/v1/sales/carts/mine')`.
 */
export async function api<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const method = options.method ?? 'GET';
  const res = await fetch(`/api${path}${toQuery(options.query)}`, {
    method,
    headers: method === 'GET' ? {accept: 'application/json', ...options.headers} : writeHeaders(options.headers),
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    credentials: 'same-origin',
    signal: options.signal,
  });
  return parse<T>(res);
}

/** The method a write's spec gives it (most are POST; `set_price` is PUT). */
export type WriteMethod = 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** Previews a write: `{method} {path}?preview=true` with the input. Nothing changes yet. */
export function preview(path: string, input: unknown, query?: Query, method: WriteMethod = 'POST'): Promise<Preview> {
  return api<Preview>(path, {method, query: {...query, preview: true}, body: input});
}

/** Runs a previewed write that needs no approval: `{method} {path} {preview_id}`. */
export function commit(path: string, previewId: string, method: WriteMethod = 'POST'): Promise<CommitResult> {
  return api<CommitResult>(path, {method, body: {preview_id: previewId}});
}
