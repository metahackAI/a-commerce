'use client';

// The only way a person's decision reaches the back end (docs/frontend/README.md §5): approve a
// change set, place an order, send a return request, undo. It calls the person-only endpoint
// through the gateway after a confirmation, with an idempotency key. It never registers as an AI
// target, so `ui.command` (which only reaches registered targets) and page tools cannot press it.

import {useState} from 'react';
import {Button} from '@astryxdesign/core/Button';
import {AlertDialog} from '@astryxdesign/core/AlertDialog';

import {type ApiErrorBody, writeHeaders} from '@/lib/api';

export interface PersonAction {
  label: string;
  method: string;
  /** Path under /api, e.g. `/v1/sales/orders`. */
  path: string;
  body?: unknown;
  /** Sentence shown before the request is sent. */
  confirm: string;
}

export interface PersonButtonProps {
  action: PersonAction;
  onDone?: (result: unknown) => void;
  /**
   * Called when the endpoint refuses (for example `estimate_changed` with `detail.new_preview`), so
   * the page can show the new numbers. The dialog closes; return `true` to keep it open with the
   * error sentence instead.
   */
  onError?: (error: ApiErrorBody, status: number) => boolean | void;
}

export function PersonButton({action, onDone, onError}: PersonButtonProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send() {
    setIsSending(true);
    setError(null);
    try {
      const res = await fetch(`/api${action.path}`, {
        method: action.method,
        headers: writeHeaders({'idempotency-key': crypto.randomUUID()}),
        credentials: 'same-origin',
        body: action.body === undefined ? undefined : JSON.stringify(action.body),
      });
      const result: unknown = await res.json().catch(() => null);
      if (!res.ok) {
        const body = (result as {error?: ApiErrorBody} | null)?.error ?? {
          code: 'upstream_unavailable',
          class: 'upstream_unavailable',
          message: `The request failed (${res.status}).`,
          next_tool: null,
        };
        if (onError && onError(body, res.status) !== true) {
          setIsOpen(false);
          return;
        }
        setError(body.message);
        return;
      }
      setIsOpen(false);
      onDone?.(result);
    } finally {
      setIsSending(false);
    }
  }

  return (
    <>
      <Button label={action.label} variant="primary" onClick={() => setIsOpen(true)} />
      <AlertDialog
        isOpen={isOpen}
        onOpenChange={setIsOpen}
        title={action.label}
        description={error ?? action.confirm}
        actionLabel={action.label}
        actionVariant="primary"
        isActionLoading={isSending}
        onAction={send}
      />
    </>
  );
}
