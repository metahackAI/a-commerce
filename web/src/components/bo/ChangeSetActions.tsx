'use client';

// A person's decisions on a change set (docs/api/platform.md, person-only endpoints): approve the
// version on screen, reject with a reason, undo before the deadline. Each goes through
// PersonButton only, appears only when the state allows it and the session holds the `human:*`
// scope, and refreshes the page when done. The server checks everything again.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {TextArea} from '@astryxdesign/core/TextArea';
import {Text} from '@astryxdesign/core/Text';

import {PersonButton} from '@/components/PersonButton';
import {dateTime} from '@/app/bo/_lib/format';
import type {ChangeSetState} from '@/app/bo/_lib/types';

export interface ChangeSetDecisionProps {
  id: string;
  number?: number;
  version: number;
  state: ChangeSetState | string;
  undoUntil: string | null;
  scopes: string[];
  /** Approve label from a block ("Approve 9 refunds and send 3 messages"). */
  approveLabel?: string;
}

/** Which decisions this session can make on a change set in this state. */
export function changeSetDecisions(p: Pick<ChangeSetDecisionProps, 'state' | 'undoUntil' | 'scopes'>) {
  const waiting = p.state === 'waiting_for_approval';
  return {
    approve: waiting && p.scopes.includes('human:approve'),
    reject: waiting && p.scopes.includes('human:reject'),
    undo: p.state === 'live' && p.undoUntil !== null && p.scopes.includes('human:undo'),
  };
}

export function ChangeSetActions({id, number, version, state, undoUntil, scopes, approveLabel}: ChangeSetDecisionProps) {
  const router = useRouter();
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState('');
  const can = changeSetDecisions({state, undoUntil, scopes});
  const name = number ? `#${number}` : id;
  const done = () => {
    setRejecting(false);
    setReason('');
    router.refresh();
  };

  if (!can.approve && !can.reject && !can.undo) return null;

  return (
    <VStack gap={2} hAlign="end">
      <HStack gap={2} vAlign="center" wrap="wrap">
        {can.reject && !rejecting ? <Button label="Reject…" onClick={() => setRejecting(true)} /> : null}
        {can.undo ? (
          <PersonButton
            action={{
              label: 'Undo',
              method: 'POST',
              path: `/v1/platform/change-sets/${encodeURIComponent(id)}/undo`,
              confirm: `Undo every change of ${name} in reverse order?${undoUntil ? ` Undo is open until ${dateTime(undoUntil)}.` : ''}`,
            }}
            variant="secondary"
            onDone={done}
          />
        ) : null}
        {can.approve ? (
          <PersonButton
            action={{
              label: approveLabel ?? 'Approve and run',
              method: 'POST',
              path: `/v1/platform/change-sets/${encodeURIComponent(id)}/approve`,
              body: {version},
              confirm: `Approve version ${version} of ${name}? Every item is checked again and then runs in order. If anything changed since this version, nothing runs and a new version is made for you to review.`,
            }}
            onDone={done}
          />
        ) : null}
      </HStack>
      {rejecting ? (
        <VStack gap={2} width={360}>
          <TextArea
            label="Why reject it?"
            description="The agent reads this when it drafts a new change set."
            value={reason}
            onChange={setReason}
            maxLength={500}
            rows={2}
            hasAutoFocus
          />
          <HStack gap={2} hAlign="end">
            <Button label="Cancel" variant="ghost" onClick={() => setRejecting(false)} />
            {reason.trim() ? (
              <PersonButton
                action={{
                  label: 'Reject',
                  method: 'POST',
                  path: `/v1/platform/change-sets/${encodeURIComponent(id)}/reject`,
                  body: {reason: reason.trim()},
                  confirm: `Reject ${name}? Nothing in it runs. Reason: “${reason.trim()}”`,
                }}
                onDone={done}
              />
            ) : (
              <Text type="supporting" color="secondary">
                Write a reason to reject.
              </Text>
            )}
          </HStack>
        </VStack>
      ) : null}
    </VStack>
  );
}
