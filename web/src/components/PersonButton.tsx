'use client';

// The only way a person's decision reaches the back end (docs/frontend/README.md §5): approve a
// change set, place an order, send a return request, undo. It calls the person-only endpoint
// through the gateway after a confirmation, with an idempotency key. It never registers as an AI
// target, so `ui.command` (which only reaches registered targets) and page tools cannot press it.

import {useState} from 'react';
import {Button} from '@astryxdesign/core/Button';
import {AlertDialog} from '@astryxdesign/core/AlertDialog';

export interface PersonAction {
  label: string;
  method: string;
  /** Path under /api, e.g. `/v1/sales/orders`. */
  path: string;
  body?: unknown;
  /** Sentence shown before the request is sent. */
  confirm: string;
}

export function PersonButton({action, onDone}: {action: PersonAction; onDone?: (result: unknown) => void}) {
  const [isOpen, setIsOpen] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send() {
    setIsSending(true);
    setError(null);
    try {
      const res = await fetch(`/api${action.path}`, {
        method: action.method,
        headers: {'content-type': 'application/json', 'idempotency-key': crypto.randomUUID()},
        body: action.body === undefined ? undefined : JSON.stringify(action.body),
      });
      const result: unknown = await res.json().catch(() => null);
      if (!res.ok) {
        const message = (result as {error?: {message?: string}} | null)?.error?.message;
        setError(message ?? `The request failed (${res.status}).`);
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
