'use client';

// `change_set` block (docs/api/chat.md §3.3): the result of a write request. The request and how
// it was understood, the checks, the changes in groups, what was left out, a preview of what
// customers will see, what runs after approval, and the undo deadline. Approve and reject are the
// block's person buttons (PersonButton), shown only while the set waits for approval; undo only
// while it is live. The state those follow is the set's now (useLiveChangeSet), not the one the
// block was sent with: once approved, the block says Live and offers Undo instead of Approve.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {MetadataList} from '@astryxdesign/core/MetadataList';
import {Section} from '@astryxdesign/core/Section';
import {Token} from '@astryxdesign/core/Token';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';

import type {BlockAction} from '@/ai/types';
import type {ChangeSetBlockData, Understood} from '@/app/bo/_lib/blocks';
import {dateTime, value, words} from '@/app/bo/_lib/format';
import type {Change} from '@/app/bo/_lib/types';
import {ChangesTable, ChecksList} from '@/components/bo/ChangeParts';
import {mayPress, useBoSession} from '@/components/bo/BoSession';
import {useLiveChangeSet} from '@/components/bo/useLiveChangeSet';
import {PersonButton} from '@/components/PersonButton';
import {StateToken} from '@/components/bo/StateToken';
import {TargetBlock, TargetFact} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';
import {RowsTable} from './rows';

const SLOTS = [
  ['who', 'Who'],
  ['products', 'Products'],
  ['benefit', 'Benefit'],
  ['when', 'When'],
  ['where', 'Where'],
] as const;

function understoodText(u: Understood): {main: string; detail: string | null} {
  if (typeof u === 'string') return {main: u, detail: null};
  return {main: u.title ?? u.text ?? u.label ?? value(u.value), detail: u.detail ?? (u.title && u.text ? u.text : null)};
}

export function ChangeSetBlock({block}: BlockProps) {
  const d = block.data as ChangeSetBlockData;
  const intentsFor = intentsLookup(block);
  const ids = {change_set_id: d.change_set_id};
  const request = Array.isArray(d.request) ? d.request : d.request ? [d.request] : [];
  const checks = d.checks ?? [];
  const failed = checks.filter(c => !c.passed).length;
  const session = useBoSession();
  const {live, follow} = useLiveChangeSet(d.change_set_id, {state: d.state, version: d.version, undo_until: d.undo?.until ?? null});
  // A newer version replaced the one this block shows: its approval would be refused.
  const replaced = live.version > d.version;
  const name = d.number ? `#${d.number}` : d.change_set_id;
  const undoPath = `/v1/platform/change-sets/${encodeURIComponent(d.change_set_id)}/undo`;
  const undo = live.state === 'live' && live.undo_until !== null && !replaced && mayPress(session, 'POST', undoPath);

  // Buttons that no longer fit the state are left out; the server would refuse them anyway.
  const hide = (a: BlockAction) =>
    a.kind === 'person' &&
    ((/\/(approve|reject)$/.test(a.path) && (live.state !== 'waiting_for_approval' || replaced)) ||
      (/\/undo$/.test(a.path) && live.state !== 'live'));

  return (
    <BlockFrame
      block={block}
      kind="change_set"
      id={d.change_set_id}
      label={`Change set ${d.number ? `#${d.number}` : d.change_set_id} v${d.version}`}
      hideActions={hide}
      onActionDone={follow}
      meta={
        <>
          <Text type="supporting" color="secondary">
            {`${name} · v${d.version}`}
          </Text>
          <StateToken state={replaced ? 'drafting' : live.state} label={replaced ? `Replaced by v${live.version}` : undefined} />
          {checks.length > 0 ? (
            <StateToken state={failed > 0 ? 'failed' : 'passed'} label={failed > 0 ? 'Some checks failed' : 'All checks passed'} />
          ) : null}
        </>
      }>
      {request.length > 0 ? (
        <VStack gap={2}>
          <Text type="label" color="secondary">
            Your request, as I understood it
          </Text>
          {request.map((r, i) => (
            <Blockquote key={i}>{r}</Blockquote>
          ))}
        </VStack>
      ) : null}

      {d.understood ? (
        <MetadataList columns="multi">
          {SLOTS.filter(([k]) => d.understood?.[k] !== undefined).map(([k, label]) => {
            const {main, detail} = understoodText(d.understood?.[k] as Understood);
            return (
              <TargetFact
                key={k}
                label={label}
                target={{kind: 'understood_field', label: `${label}: ${main}${detail ? ` (${detail})` : ''}`, ids: {slot: k, ...ids}, blockId: block.block_id}}
                intents={intentsFor('understood_field', k)}>
                <VStack gap={0}>
                  <Text weight="medium">{main}</Text>
                  {detail ? (
                    <Text type="supporting" color="secondary">
                      {detail}
                    </Text>
                  ) : null}
                </VStack>
              </TargetFact>
            );
          })}
        </MetadataList>
      ) : null}

      {checks.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Checks</Heading>
          <ChecksList checks={checks} ids={ids} blockId={block.block_id} intentsFor={intentsFor} />
        </VStack>
      ) : null}

      {(d.groups ?? []).map((g, i) => (
        <Section key={`${g.op}-${g.label}-${i}`} padding={3}>
          <VStack gap={2}>
            <TargetBlock
              target={{kind: 'change_group', label: `${g.op} ${g.label}${g.summary ? ` · ${g.summary}` : ''}`, ids: {group: `${i}`, ...ids}, blockId: block.block_id}}
              intents={intentsFor('change_group', g.label)}>
              <HStack gap={2} vAlign="center" wrap="wrap">
                <Token label={g.op} size="sm" />
                <Text weight="semibold">{g.label}</Text>
                {g.summary ? (
                  <Text type="supporting" color="secondary">
                    {g.summary}
                  </Text>
                ) : null}
              </HStack>
            </TargetBlock>
            {(g.rows ?? []).length > 0 ? (
              <ChangesTable changes={(g.rows ?? []) as unknown as Change[]} ids={ids} blockId={block.block_id} intentsFor={intentsFor} />
            ) : (
              <Text type="supporting" color="secondary">
                No rows in this group
              </Text>
            )}
            {g.more ? (
              <Text type="supporting" color="secondary">
                {typeof g.more === 'number' ? `And ${g.more} more` : g.more}
              </Text>
            ) : null}
          </VStack>
        </Section>
      ))}

      {d.left_out && d.left_out.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Left out</Heading>
          <RowsTable
            rows={d.left_out.map(l => ({...l}))}
            columns={[
              {key: 'reason', label: 'Why'},
              {key: 'rule', label: 'Rule'},
            ]}
            kind="left_out"
            blockId={block.block_id}
            ids={ids}
            intentsFor={intentsFor}
          />
        </VStack>
      ) : null}

      {d.preview ? (
        <VStack gap={2}>
          <Heading level={4}>What customers will see</Heading>
          <HStack gap={1} wrap="wrap">
            {(d.preview.segments ?? []).map(s => (
              <Token key={s} label={words(s)} size="sm" color={s === d.preview?.selected ? 'blue' : 'default'} />
            ))}
          </HStack>
          <TargetBlock target={{kind: 'preview', label: `Preview: ${value(d.preview.content)}`, ids, blockId: block.block_id}}>
            <Text>{value(d.preview.content)}</Text>
          </TargetBlock>
        </VStack>
      ) : null}

      {d.after_approval && d.after_approval.length > 0 ? (
        <VStack gap={1}>
          <Heading level={4}>After you approve</Heading>
          <List density="compact" listStyle="decimal">
            {d.after_approval.map((s, i) => (
              <ListItem key={i} label={typeof s === 'string' ? s : (s.label ?? s.text ?? s.tool ?? '')} />
            ))}
          </List>
        </VStack>
      ) : null}

      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text type="supporting" color="secondary">
          {replaced
            ? `Version ${live.version} replaced this one; approve it on the full change set.`
            : live.undo_until
              ? `${live.state === 'live' ? 'Live. ' : ''}You can undo it until ${dateTime(live.undo_until)}.`
              : d.undo === null
                ? 'This change set can’t be undone.'
                : ''}
        </Text>
        {undo ? (
          <PersonButton
            action={{
              label: 'Undo',
              method: 'POST',
              path: undoPath,
              confirm: `Undo every change of ${name} in reverse order?${live.undo_until ? ` Undo is open until ${dateTime(live.undo_until)}.` : ''}`,
            }}
            variant="secondary"
            size="sm"
            onDone={follow}
          />
        ) : null}
        <Link href={`/bo/change-sets/${encodeURIComponent(d.change_set_id)}`}>Open the full change set</Link>
      </HStack>
    </BlockFrame>
  );
}
