'use client';

// A write from a page, in the two steps every write has (docs/api/conventions.md §7): preview
// shows what would change and which rules pass; when no approval is needed the person applies it,
// otherwise the page says why it needs approval and offers to have the agent put it in a change
// set, where a person approves it. Nothing changes until "Apply".

import {useState, type ReactNode} from 'react';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';

import {useAi} from '@/ai/AiProvider';
import {ApiError, type ApiErrorBody, type Preview} from '@/lib/api';
import {dateTime} from '@/app/bo/_lib/format';
import {entityHref} from '@/app/bo/_lib/links';
import type {CommitResult} from '@/app/bo/_lib/types';
import {commitWrite, previewWrite, type WriteMethod} from '@/app/bo/_lib/write';

import {useAgentDock} from './AgentDock';
import {ChangesTable, ChecksList} from './ChangeParts';
import {ErrorState} from './ErrorState';

function toError(e: unknown): ApiErrorBody {
  if (e instanceof ApiError) return e.body;
  return {code: 'upstream_unavailable', class: 'upstream_unavailable', message: 'The store could not be reached. Try again.', next_tool: null};
}

export interface WriteFlowProps {
  method: WriteMethod;
  /** REST path under /api, e.g. `/v1/catalog/variants/var_x/price`. */
  path: string;
  /** The tool's input, or `null` while the form is incomplete. */
  input: unknown | null;
  /** The form fields. */
  children?: ReactNode;
  previewLabel?: string;
  applyLabel?: string;
  /** What to ask the agent when the write needs approval (it then drafts a change set). */
  askWhenApprovalNeeded: string;
  onApplied?: (result: CommitResult) => void;
}

export function WriteFlow({
  method,
  path,
  input,
  children,
  previewLabel = 'Preview',
  applyLabel = 'Apply',
  askWhenApprovalNeeded,
  onApplied,
}: WriteFlowProps) {
  const ai = useAi();
  const dock = useAgentDock();
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewedFor, setPreviewedFor] = useState<string | null>(null);
  const [result, setResult] = useState<CommitResult | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [busy, setBusy] = useState<'preview' | 'apply' | null>(null);

  const inputKey = input === null ? null : JSON.stringify(input);
  const current = preview && previewedFor === inputKey ? preview : null;

  async function runPreview() {
    if (input === null) return;
    setBusy('preview');
    setError(null);
    setResult(null);
    try {
      const p = await previewWrite(method, path, input);
      setPreview(p);
      setPreviewedFor(inputKey);
    } catch (e) {
      setPreview(null);
      setError(toError(e));
    } finally {
      setBusy(null);
    }
  }

  async function apply() {
    if (!current) return;
    setBusy('apply');
    setError(null);
    try {
      const r = await commitWrite(method, path, current.preview_id);
      setResult(r);
      setPreview(null);
      onApplied?.(r);
    } catch (e) {
      setError(toError(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <VStack gap={4}>
      {children}
      <HStack gap={2}>
        <Button
          label={previewLabel}
          variant={current ? 'secondary' : 'primary'}
          isDisabled={input === null}
          isLoading={busy === 'preview'}
          onClick={() => void runPreview()}
        />
        {current && !current.needs_approval ? (
          <Button label={applyLabel} variant="primary" isLoading={busy === 'apply'} onClick={() => void apply()} />
        ) : null}
      </HStack>

      {error ? (
        <ErrorState
          error={error}
          onRetry={error.code === 'preview_expired' || error.code === 'estimate_changed' ? () => void runPreview() : undefined}
        />
      ) : null}

      {result ? (
        <Banner
          status="success"
          title="Done."
          description={
            result.undo
              ? `Saved as ${result.id}. A person can undo it from change set ${result.undo.target}${result.undo.until ? ` until ${dateTime(result.undo.until)}` : ''}.`
              : `Saved as ${result.id}. This change can't be undone.`
          }
          endContent={
            result.undo && entityHref(result.undo.target) ? (
              <Button label="Open the change set" size="sm" href={entityHref(result.undo.target) ?? undefined} />
            ) : undefined
          }
        />
      ) : null}

      {current ? (
        <VStack gap={4}>
          {current.needs_approval ? (
            <Banner
              status="warning"
              title="This needs a person's approval"
              description={`${current.approval_reason ?? 'The policy asks for approval.'} It can only run inside a change set that a person approves.`}
              endContent={
                current.change_set_id ? (
                  <Button label="Open the change set" size="sm" href={entityHref(current.change_set_id) ?? undefined} />
                ) : (
                  <Button
                    label="Ask the agent to prepare it"
                    size="sm"
                    onClick={() => {
                      dock.open();
                      void ai.send(askWhenApprovalNeeded);
                    }}
                  />
                )
              }
            />
          ) : (
            <Banner status="info" title="No approval needed" description="Check the changes below, then apply them." />
          )}
          <VStack gap={2}>
            <Heading level={4}>Changes</Heading>
            <ChangesTable changes={current.changes} ids={{preview_id: current.preview_id}} />
          </VStack>
          <VStack gap={2}>
            <Heading level={4}>Checks</Heading>
            <ChecksList checks={current.checks} ids={{preview_id: current.preview_id}} />
          </VStack>
          <Text type="supporting" color="secondary">
            {`Preview ${current.preview_id}`}
            {current.expires_at ? ` · valid until ${dateTime(current.expires_at)}` : ''}
          </Text>
          {current.change_set_id ? (
            <Link href={entityHref(current.change_set_id) ?? '#'}>{`Part of change set ${current.change_set_id}`}</Link>
          ) : null}
        </VStack>
      ) : null}
    </VStack>
  );
}
