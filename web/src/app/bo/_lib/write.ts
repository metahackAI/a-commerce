// Two-step writes from the browser (docs/api/conventions.md §7): preview with `?preview=true`
// and the input, then run with `{preview_id}` only when no approval is needed. The write goes
// with the REST method its spec names (`PUT /v1/catalog/variants/{id}/price`), which is why these
// wrap `api()` instead of `preview()`/`commit()`, which always POST.

import {api, type Preview} from '@/lib/api';

import type {CommitResult} from './types';

export type WriteMethod = 'POST' | 'PUT' | 'PATCH';

export function previewWrite(method: WriteMethod, path: string, input: unknown): Promise<Preview> {
  return api<Preview>(path, {method, query: {preview: true}, body: input});
}

export function commitWrite(method: WriteMethod, path: string, previewId: string): Promise<CommitResult> {
  return api<CommitResult>(path, {method, body: {preview_id: previewId}});
}
