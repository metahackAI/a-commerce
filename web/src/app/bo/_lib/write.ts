// Two-step writes from the browser (docs/api/conventions.md §7): preview with `?preview=true`
// and the input, then run with `{preview_id}` only when no approval is needed. The write goes
// with the REST method its spec names (`PUT /v1/catalog/variants/{id}/price`).

import {commit, preview, type Preview, type WriteMethod} from '@/lib/api';

import type {CommitResult} from './types';

export type {WriteMethod};

export function previewWrite(method: WriteMethod, path: string, input: unknown): Promise<Preview> {
  return preview(path, input, undefined, method);
}

export function commitWrite(method: WriteMethod, path: string, previewId: string): Promise<CommitResult> {
  return commit(path, previewId, method) as Promise<CommitResult>;
}
