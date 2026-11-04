// What a storefront read comes back as, on the server (`load`) and in the browser (`loadInBrowser`):
// the data, or the API's error so the page can say what happened.
import {api, ApiError, type ApiErrorBody, type Query} from '@/lib/api';

export type Loaded<T> = {ok: true; data: T} | {ok: false; status: number; error: ApiErrorBody};

export const unreachable: ApiErrorBody = {
  code: 'upstream_unavailable',
  class: 'upstream_unavailable',
  message: 'The store could not be reached. Try again in a moment.',
  next_tool: null,
};

/** `GET /api{path}` from the browser. Never throws, like `load` on the server. */
export async function loadInBrowser<T>(path: string, query?: Query): Promise<Loaded<T>> {
  try {
    return {ok: true, data: await api<T>(path, {query})};
  } catch (e) {
    if (e instanceof ApiError) return {ok: false, status: e.status, error: e.body};
    return {ok: false, status: 503, error: unreachable};
  }
}
