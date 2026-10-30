// Shopper writes that need no approval (cart, wishlist): preview first, then run the preview
// (docs/api/conventions.md §7). The preview's failing checks stop the write and their sentences
// are shown; nothing here decides a business rule.

import {api, ApiError, type ApiErrorBody, type Preview} from '@/lib/api';

type Method = 'POST' | 'PATCH' | 'PUT' | 'DELETE';

/** Thrown when a preview's checks fail or it would need a person's approval. */
export class WriteStopped extends Error {
  readonly preview: Preview;
  constructor(preview: Preview, message: string) {
    super(message);
    this.name = 'WriteStopped';
    this.preview = preview;
  }
}

/** Previews a write: `{method} {path}?preview=true` with the input. Nothing changes yet. */
export function previewWrite(method: Method, path: string, input: unknown): Promise<Preview> {
  return api<Preview>(path, {method, query: {preview: true}, body: input});
}

/** Previews, stops on failing checks, then runs the preview with its `preview_id`. */
export async function write<T>(method: Method, path: string, input: unknown = {}): Promise<T> {
  const preview = await previewWrite(method, path, input);
  const failed = preview.checks.filter(c => !c.passed);
  if (failed.length > 0) {
    throw new WriteStopped(preview, failed.map(c => c.detail ?? c.rule).join(' '));
  }
  if (preview.needs_approval) {
    throw new WriteStopped(preview, preview.approval_reason ?? 'This change needs a person at the store to approve it.');
  }
  return api<T>(path, {method, body: {preview_id: preview.preview_id}});
}

/** Any thrown value as the API's error shape, so the page can show one sentence. */
export function errorBody(e: unknown): ApiErrorBody {
  if (e instanceof ApiError) return e.body;
  if (e instanceof WriteStopped) {
    return {code: 'checks_failed', class: 'policy_violation', message: e.message, next_tool: null};
  }
  return {
    code: 'upstream_unavailable',
    class: 'upstream_unavailable',
    message: 'The store could not be reached. Try again in a moment.',
    next_tool: null,
  };
}
