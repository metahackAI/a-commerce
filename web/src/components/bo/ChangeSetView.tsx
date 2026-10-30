'use client';

// One change set in full (get_change_set): the request and how the agent understood it, every
// item's changes, the rule checks, why it needs approval, what was left out, the verification
// after it ran, the undo deadline, and its history from the action log. The person's decisions
// (approve the version on screen, reject, undo) are PersonButtons in the header.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text, Heading} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Banner} from '@astryxdesign/core/Banner';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Section} from '@astryxdesign/core/Section';
import {Token} from '@astryxdesign/core/Token';
import {Button} from '@astryxdesign/core/Button';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import type {ApiErrorBody} from '@/lib/api';
import {actor, count, dateTime, value, words} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {ActionEntry, ChangeSet, Verification} from '@/app/bo/_lib/types';

import {ActionLogTable} from './ActionLogTable';
import {useBoSession} from './BoSession';
import {ChangeSetActions} from './ChangeSetActions';
import {ChangesTable, ChecksList, changeSubject, type ItemCheck} from './ChangeParts';
import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {TargetBlock, TargetFact, TargetRow} from './targets';

const SLOT_LABEL: Record<string, string> = {who: 'Who', products: 'Products', benefit: 'Benefit', when: 'When', where: 'Where', other: 'Also'};

export interface ChangeSetViewProps {
  changeSet: ChangeSet;
  latestVersion: number;
  verification: Verification | null;
  history: ActionEntry[] | null;
  historyError: ApiErrorBody | null;
}

export function ChangeSetView({changeSet: cs, latestVersion, verification, history, historyError}: ChangeSetViewProps) {
  const session = useBoSession();
  const ids = {change_set_id: cs.id};
  const isLatest = cs.version === latestVersion;
  const items = cs.items ?? [];
  // Each check says which item it was run on: its position and what it changes.
  const allChecks: ItemCheck[] = items.flatMap(i =>
    i.checks.map(c => ({...c, subject: `Item ${i.position}${i.changes[0] ? ` · ${changeSubject(i.changes[0])}` : ''}`})),
  );
  const summary = cs.checks_summary;

  return (
    <PageFrame
      back={{href: '/bo/change-sets', label: 'Change sets'}}
      title={`#${cs.number} · ${cs.title}`}
      meta={
        <>
          <StateToken state={cs.state} size="md" />
          <Text type="supporting" color="secondary">
            {`Version ${cs.version}${isLatest ? '' : ` (latest is ${latestVersion})`}`}
          </Text>
          {cs.needs_approval && !cs.approved_at ? <Token label="Needs approval" size="sm" color="orange" /> : null}
          {summary && summary.total > 0 ? (
            <StateToken
              state={summary.failed > 0 ? 'failed' : 'passed'}
              label={summary.failed > 0 ? `${summary.failed} of ${summary.total} checks failed` : `${summary.passed} of ${summary.total} checks passed`}
            />
          ) : null}
        </>
      }
      actions={
        isLatest ? (
          <ChangeSetActions
            id={cs.id}
            number={cs.number}
            version={cs.version}
            state={cs.state}
            undoUntil={cs.undo_until}
            scopes={session?.scopes ?? []}
          />
        ) : (
          <Button label={`Open version ${latestVersion}`} variant="primary" href={`/bo/change-sets/${encodeURIComponent(cs.id)}`} />
        )
      }
      panel={<ChangeSetFacts cs={cs} latestVersion={latestVersion} />}>
      {!isLatest ? (
        <Banner status="info" title={`This is version ${cs.version}.`} description={`Decisions are made on the latest version (${latestVersion}).`} />
      ) : null}
      {cs.state === 'live' && cs.undo_until ? (
        <Banner status="success" title="Live" description={`Every change ran and was checked again. A person can undo it until ${dateTime(cs.undo_until)}.`} />
      ) : null}
      {cs.state === 'live' && !cs.undo_until ? (
        <Banner status="success" title="Live" description="It ran and was checked again. It holds a change that can't be undone (a refund or a customer message)." />
      ) : null}
      {cs.state === 'failed' ? (
        <Banner status="error" title="It failed" description="An item failed or did not match when checked again. What ran was undone in reverse order; see the verification and history below." />
      ) : null}
      {cs.state === 'waiting_for_approval' && !(session?.scopes.includes('human:approve') ?? false) ? (
        <Banner status="info" title="Waiting for approval" description="Your account can’t approve change sets. Someone with the approver role has to." />
      ) : null}

      <PageSection title="Your request, as I understood it">
        {cs.request?.text?.length ? (
          <VStack gap={2}>
            {cs.request.text.map((t, i) => (
              <Blockquote key={i}>{t}</Blockquote>
            ))}
          </VStack>
        ) : (
          <Text color="secondary">No request text was recorded.</Text>
        )}
        {cs.interpretation?.length ? (
          <MetadataList columns="multi">
            {cs.interpretation.map((u, i) => (
              <TargetFact
                key={`${u.slot}-${i}`}
                label={SLOT_LABEL[u.slot] ?? words(u.slot)}
                target={{kind: 'understood_field', label: `${SLOT_LABEL[u.slot] ?? u.slot}: ${u.text}`, ids: {slot: u.slot, ...ids}, term: 'acom:Interpretation'}}>
                <Text weight="medium">{u.text}</Text>
              </TargetFact>
            ))}
          </MetadataList>
        ) : null}
      </PageSection>

      <PageSection
        title="Checks"
        count={summary ? `${summary.passed} of ${summary.total} passed` : undefined}
        description="Rules from the store's policies, checked on every item. Press a rule to read the policy.">
        <ChecksList checks={allChecks} ids={ids} />
      </PageSection>

      <PageSection title="Changes" count={`${items.length} ${items.length === 1 ? 'item' : 'items'}, run in this order after approval`}>
        {items.length === 0 ? <Text color="secondary">No items yet.</Text> : null}
        {items.map(item => (
          <Section key={item.preview_id} padding={4}>
            <VStack gap={3}>
              <TargetBlock
                target={{
                  kind: 'change_set_item',
                  label: `Item ${item.position}: ${item.tool}${item.approval_reason ? ` · ${item.approval_reason}` : ''}`,
                  ids: {preview_id: item.preview_id, ...ids},
                  term: 'acom:ChangeSetItem',
                }}>
                <HStack gap={2} vAlign="center" wrap="wrap">
                  <Heading level={3}>{`${item.position} · ${item.tool}`}</Heading>
                  {item.needs_approval ? (
                    cs.approved_at ? <Token label="Approved" size="sm" /> : <Token label="Needs approval" size="sm" color="orange" />
                  ) : (
                    <Token label="No approval needed" size="sm" />
                  )}
                  {item.action_status ? <StateToken state={item.action_status} /> : null}
                </HStack>
                {item.approval_reason ? <Text>{item.approval_reason}</Text> : null}
                <Text type="supporting" color="secondary">
                  {`Preview ${item.preview_id}${item.address ? ` · ${item.address}` : ''}${
                    item.depends_on && item.depends_on.length > 0 ? ` · uses the result of ${item.depends_on.join(', ')}` : ''
                  }`}
                </Text>
              </TargetBlock>
              <ChangesTable
                changes={item.changes}
                total={item.changes_total}
                currency={typeof item.input?.price_currency === 'string' ? item.input.price_currency : undefined}
                ids={{...ids, preview_id: item.preview_id}}
              />
            </VStack>
          </Section>
        ))}
      </PageSection>

      {cs.left_out && cs.left_out.length > 0 ? (
        <PageSection title="Left out" count={cs.left_out.length} description="Recorded in the preview, not changed.">
          <Table density="compact">
            <TableHeader>
              <TableRow isHeaderRow>
                <TableHeaderCell>What</TableHeaderCell>
                <TableHeaderCell>Why</TableHeaderCell>
                <TableHeaderCell>Rule</TableHeaderCell>
                <TableHeaderCell>
                  <Text type="supporting" color="secondary">
                    Ask
                  </Text>
                </TableHeaderCell>
              </TableRow>
            </TableHeader>
            <TableBody>
              {cs.left_out.map(l => (
                <TargetRow
                  key={`${l.entity}-${l.id}`}
                  target={{kind: 'left_out', label: `${l.name ?? l.id} left out: ${l.reason ?? ''}`, ids: {[`${l.entity}_id`]: l.id, ...ids}}}>
                  <TableCell>
                    <VStack gap={0}>
                      {entityHref(l.id) ? <Link href={entityHref(l.id) ?? '#'}>{l.name ?? l.id}</Link> : <Text>{l.name ?? l.id}</Text>}
                      <Text type="supporting" color="secondary">
                        {l.id}
                      </Text>
                    </VStack>
                  </TableCell>
                  <TableCell>
                    <Text>{l.reason ?? '—'}</Text>
                  </TableCell>
                  <TableCell>
                    {l.rule ? <Link href={withParams('/bo/policies', {key: l.rule})}>{l.rule}</Link> : <Text color="secondary">—</Text>}
                  </TableCell>
                </TargetRow>
              ))}
            </TableBody>
          </Table>
        </PageSection>
      ) : null}

      {verification ? (
        <PageSection
          title="Verification"
          count={`${words(verification.state)} · ${dateTime(verification.verified_at)}`}
          description="After running, the store was read again and compared with every change.">
          <VerificationTable verification={verification} />
        </PageSection>
      ) : cs.verification ? (
        <PageSection title="Verification">
          <Text>{`${words(cs.verification.state)}: ${cs.verification.matched} of ${cs.verification.total} matched`}</Text>
        </PageSection>
      ) : null}

      <PageSection title="History" description="Every preview, approval and run of this change set, from the action log.">
        {historyError ? <ErrorState error={historyError} title="The history could not be loaded." /> : null}
        {history ? <ActionLogTable entries={history} empty="Nothing recorded yet" /> : null}
      </PageSection>
    </PageFrame>
  );
}

function VerificationTable({verification}: {verification: Verification}) {
  return (
    <Table density="compact" verticalAlign="top">
      <TableHeader>
        <TableRow isHeaderRow>
          <TableHeaderCell>Item</TableHeaderCell>
          <TableHeaderCell>Result</TableHeaderCell>
          <TableHeaderCell>Checked</TableHeaderCell>
          <TableHeaderCell>Matched</TableHeaderCell>
          <TableHeaderCell>
            <Text type="supporting" color="secondary">
              Ask
            </Text>
          </TableHeaderCell>
        </TableRow>
      </TableHeader>
      <TableBody>
        {verification.items.map(v => (
          <TargetRow
            key={v.preview_id}
            target={{
              kind: 'verification_row',
              label: `${v.tool}: ${v.matched ? 'matched' : 'did not match'} (${v.checked} checked)`,
              ids: {preview_id: v.preview_id, change_set_id: verification.change_set_id},
            }}>
            <TableCell>
              <VStack gap={0}>
                <Text>{v.tool}</Text>
                <Text type="supporting" color="secondary">
                  {v.preview_id}
                </Text>
              </VStack>
            </TableCell>
            <TableCell>
              {v.result_id && entityHref(v.result_id) ? <Link href={entityHref(v.result_id) ?? '#'}>{v.result_id}</Link> : <Text>{v.result_id ?? '—'}</Text>}
            </TableCell>
            <TableCell>
              <Text hasTabularNumbers>{count(v.checked)}</Text>
            </TableCell>
            <TableCell>
              <VStack gap={1}>
                <StateToken state={v.matched ? 'matched' : 'mismatch'} label={v.matched ? 'Matched' : 'Did not match'} />
                {v.mismatches.map((m, i) => (
                  <Text key={i} type="supporting" color="secondary">
                    {`${m.entity} ${m.id} · ${m.field}: expected ${value(m.expected)}, found ${value(m.actual)}`}
                  </Text>
                ))}
              </VStack>
            </TableCell>
          </TargetRow>
        ))}
      </TableBody>
    </Table>
  );
}

/** A person's or agent's name as the change set's history recorded it, else their id. */
function nameOf(cs: ChangeSet, id: string | null | undefined): string {
  if (!id) return '—';
  return cs.history?.find(h => h.agent_id === id && h.agent_name)?.agent_name ?? id;
}

function ChangeSetFacts({cs, latestVersion}: {cs: ChangeSet; latestVersion: number}) {
  const versions = Array.from({length: latestVersion}, (_, i) => i + 1).reverse();
  return (
    <VStack gap={4}>
      <MetadataList title={<Heading level={4}>About</Heading>}>
        <MetadataListItem label="Number">{`#${cs.number}`}</MetadataListItem>
        <MetadataListItem label="State">
          <StateToken state={cs.state} />
        </MetadataListItem>
        <MetadataListItem label="Undo until">{cs.undo_until ? dateTime(cs.undo_until) : 'Can’t be undone or not run yet'}</MetadataListItem>
        <MetadataListItem label="Requested by">{nameOf(cs, cs.request?.requested_by ?? cs.created_by)}</MetadataListItem>
        <MetadataListItem label="Agent">{actor(cs.agent)}</MetadataListItem>
        <MetadataListItem label="Created">{dateTime(cs.created_at)}</MetadataListItem>
        <MetadataListItem label="Submitted">{dateTime(cs.submitted_at)}</MetadataListItem>
        <MetadataListItem label="Approved">{cs.approved_at ? `${dateTime(cs.approved_at)} by ${cs.approved_by ? nameOf(cs, cs.approved_by) : 'standing approval'}` : '—'}</MetadataListItem>
        {cs.request?.conversation_id ? <MetadataListItem label="Conversation">{cs.request.conversation_id}</MetadataListItem> : null}
      </MetadataList>
      {versions.length > 1 ? (
        <VStack gap={1}>
          <Heading level={4}>Versions</Heading>
          {versions.map(v => (
            <Link key={v} href={v === latestVersion ? `/bo/change-sets/${encodeURIComponent(cs.id)}` : withParams(`/bo/change-sets/${encodeURIComponent(cs.id)}`, {version: String(v)})}>
              {`Version ${v}${v === latestVersion ? ' (latest)' : ''}${v === cs.version ? ' · shown' : ''}`}
            </Link>
          ))}
        </VStack>
      ) : null}
    </VStack>
  );
}
