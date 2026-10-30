'use client';

// Product drafts (scene B4): what each draft from a supplier sheet still needs, sending ready ones
// to review (submit_for_review), sending reviewed ones back (reject_product), and publishing them
// (approve_product), which always goes into a change set a person approves on the change set's
// page. Every write is previewed; the page shows the rule checks and computes nothing itself.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TextArea} from '@astryxdesign/core/TextArea';
import {Banner} from '@astryxdesign/core/Banner';
import {Text} from '@astryxdesign/core/Text';

import {api, ApiError, preview, type ApiErrorBody, type Preview} from '@/lib/api';
import {price, words} from '@/app/bo/_lib/format';
import type {ProductGroupSummary} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {ChecksList} from './ChangeParts';
import {ErrorState} from './ErrorState';
import {StateToken} from './StateToken';
import {DataTable, Figure, RecordCell} from './tables';
import {TargetField} from './targets';
import {WriteFlow} from './WriteFlow';

type Step = {kind: 'submit' | 'reject'; row: ProductGroupSummary} | null;

function toError(e: unknown): ApiErrorBody {
  if (e instanceof ApiError) return e.body;
  return {code: 'upstream_unavailable', class: 'upstream_unavailable', message: 'The store could not be reached. Try again.', next_tool: null};
}

/** What a person may ask about a draft row, from what is open on it. */
function intentsFor(r: ProductGroupSummary): string[] {
  const d = r['acom:draft'];
  if (!d) return [];
  if (d.open_fields.includes('material')) return ['Add the fabric mix'];
  if (d.open_fields.includes('image')) return ['Pick the photo for this one'];
  return ['Write the description in our tone'];
}

export function DraftsReview({rows}: {rows: ProductGroupSummary[]}) {
  const router = useRouter();
  const session = useBoSession();
  const canWrite = session?.scopes.includes('tools:write:catalog') ?? false;
  const canDraft = session?.scopes.includes('changeset:draft') ?? false;
  const [step, setStep] = useState<Step>(null);
  const [reason, setReason] = useState('');
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [previews, setPreviews] = useState<{id: string; preview: Preview}[]>([]);

  const inReview = rows.filter(r => r.approval_status === 'pending_review');
  const toggle = (id: string, on: boolean) => setPicked(p => (on ? [...p, id] : p.filter(x => x !== id)));

  // approve_product reaches shoppers, so it never runs from here: each one is previewed into a
  // new change set, the set goes to people, and a person approves it on its page.
  async function publish() {
    const ids = picked.filter(id => inReview.some(r => r.id === id));
    if (ids.length === 0) return;
    setBusy(true);
    setError(null);
    setPreviews([]);
    try {
      const names = ids.map(id => rows.find(r => r.id === id)?.name ?? id);
      const cs = await api<{id: string}>('/v1/platform/change-sets', {
        method: 'POST',
        body: {title: `Publish ${ids.length} product${ids.length === 1 ? '' : 's'}`, request_text: [`Publish ${names.join(', ')}.`]},
      });
      const made: {id: string; preview: Preview}[] = [];
      for (const id of ids) {
        made.push({id, preview: await preview(`/v1/catalog/products/${encodeURIComponent(id)}/approve`, {product_id: id}, {change_set_id: cs.id})});
      }
      setPreviews(made);
      if (made.every(m => m.preview.checks.every(c => c.passed))) {
        const head = await api<{version: number}>(`/v1/platform/change-sets/${encodeURIComponent(cs.id)}`, {query: {fields: 'version'}});
        await api(`/v1/platform/change-sets/${encodeURIComponent(cs.id)}/submit`, {method: 'POST', body: {version: head.version}});
      }
      router.push(`/bo/change-sets/${encodeURIComponent(cs.id)}`);
    } catch (e) {
      setError(toError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <VStack gap={4}>
      {canWrite && canDraft && inReview.length > 0 ? (
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Button
            label={picked.length ? `Publish ${picked.length} in a change set` : 'Pick products in review to publish'}
            variant="primary"
            isDisabled={picked.length === 0}
            isLoading={busy}
            onClick={() => void publish()}
          />
          <Text type="supporting" color="secondary">
            Publishing reaches shoppers, so a person approves the change set before anything changes.
          </Text>
        </HStack>
      ) : null}
      {error ? <ErrorState error={error} title="The change set could not be prepared." /> : null}
      {previews.some(p => p.preview.checks.some(c => !c.passed)) ? (
        <VStack gap={2}>
          <Banner status="warning" title="Some checks failed" description="The change set stays a draft until they pass." />
          {previews.map(p => (
            <ChecksList key={p.id} checks={p.preview.checks} ids={{product_id: p.id, preview_id: p.preview.preview_id}} />
          ))}
        </VStack>
      ) : null}
      <DataTable
        rows={rows}
        rowKey={r => r.id}
        empty={{title: 'No drafts here', description: 'Import a supplier sheet from the chat, or create a product.'}}
        target={r => ({
          kind: 'draft_row',
          label: `${r.name}${r['acom:draft'] ? ` · ${words(r['acom:draft'].draft_status)}` : ''}${r['acom:draft']?.note ? ` · ${r['acom:draft'].note}` : ''}`,
          ids: {product_id: r.id, ...(r['acom:draft'] ? {import_id: r['acom:draft'].import_id} : {})},
          term: 'schema:ProductGroup',
        })}
        intentsFor={intentsFor}
        columns={[
          {
            header: 'Pick',
            cell: r =>
              r.approval_status === 'pending_review' ? (
                <CheckboxInput label={`Pick ${r.name}`} isLabelHidden value={picked.includes(r.id)} onChange={on => toggle(r.id, on)} />
              ) : null,
          },
          {header: 'Product', cell: r => <RecordCell href={`/bo/products/${encodeURIComponent(r.id)}`} name={r.name} id={r.id} />},
          {header: 'Supplier code', cell: r => <Text color="secondary">{r['acom:draft']?.supplier_code ?? '—'}</Text>},
          {header: 'Draft', cell: r => (r['acom:draft'] ? <StateToken state={r['acom:draft'].draft_status} /> : <Text color="secondary">—</Text>)},
          {
            header: 'Still open',
            cell: r => (
              <VStack gap={0}>
                <Text>{r['acom:draft']?.open_fields.length ? r['acom:draft'].open_fields.map(words).join(', ') : '—'}</Text>
                {r['acom:draft']?.note ? (
                  <Text type="supporting" color="secondary">
                    {r['acom:draft'].note}
                  </Text>
                ) : null}
              </VStack>
            ),
          },
          {header: 'Cost → price', cell: r => <Figure>{`${price(r.cost ?? null, r.price_currency)} → ${price(r.price ?? null, r.price_currency)}`}</Figure>},
          {header: 'Review', cell: r => (r.approval_status ? <StateToken state={r.approval_status} /> : null)},
          {
            header: 'Next',
            cell: r =>
              !canWrite ? null : r.approval_status === 'draft' || r.approval_status === 'rejected' ? (
                <Button label="Send to review" size="sm" onClick={() => setStep({kind: 'submit', row: r})} />
              ) : r.approval_status === 'pending_review' ? (
                <Button label="Send back" size="sm" variant="ghost" onClick={() => setStep({kind: 'reject', row: r})} />
              ) : null,
          },
        ]}
      />

      {step ? (
        <Dialog isOpen onOpenChange={open => !open && setStep(null)} width={720} purpose="form">
          <DialogHeader
            title={step.kind === 'submit' ? `Send ${step.row.name} to review` : `Send ${step.row.name} back`}
            subtitle={step.row.id}
            onOpenChange={open => !open && setStep(null)}
          />
          <VStack gap={4} padding={4}>
            {step.kind === 'submit' ? (
              <WriteFlow
                method="POST"
                path={`/v1/catalog/products/${encodeURIComponent(step.row.id)}/submit`}
                input={{product_id: step.row.id}}
                previewLabel="Check it"
                applyLabel="Send to review"
                askWhenApprovalNeeded={`Prepare a change set to send ${step.row.name} (${step.row.id}) to review.`}
                onApplied={() => {
                  setStep(null);
                  router.refresh();
                }}>
                <Text type="supporting" color="secondary">
                  The check says what publishing still needs (variants with prices and costs, a description, nothing left open from the sheet).
                </Text>
              </WriteFlow>
            ) : (
              <WriteFlow
                method="POST"
                path={`/v1/catalog/products/${encodeURIComponent(step.row.id)}/reject`}
                input={reason.trim() ? {product_id: step.row.id, reason: reason.trim()} : null}
                previewLabel="Preview"
                applyLabel="Send back"
                askWhenApprovalNeeded={`Prepare a change set to send ${step.row.name} (${step.row.id}) back.`}
                onApplied={() => {
                  setStep(null);
                  setReason('');
                  router.refresh();
                }}>
                <TargetField target={{kind: 'input', label: `Why ${step.row.name} goes back: ${reason || 'empty'}`, ids: {product_id: step.row.id}, term: 'acom:reason'}}>
                  <TextArea label="Why?" description="Whoever edits it next reads this." value={reason} onChange={setReason} rows={2} maxLength={500} />
                </TargetField>
              </WriteFlow>
            )}
          </VStack>
        </Dialog>
      ) : null}
    </VStack>
  );
}
