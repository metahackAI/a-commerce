'use client';

// The parts every write shows before it runs (docs/api/README.md §3.3): the rule checks and the
// changes, each row an AI target. Values are the API's, formatted for reading; fields that are
// the same before and after are left out of the before → after columns.

import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Icon} from '@astryxdesign/core/Icon';
import {Token} from '@astryxdesign/core/Token';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import {useAiTarget} from '@/ai/useAiTarget';
import {value, words} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {Change, Check} from '@/app/bo/_lib/types';

import {AskButton, SuggestedIntents, TargetRow} from './targets';

/** Intents for an element from a block's `suggested_intents` (`element: "change_row:prd_x"`). */
export type IntentsFor = (kind: string, id: string) => string[] | undefined;

function CheckRow({check, ids, blockId, intents}: {check: Check; ids: Record<string, string>; blockId?: string; intents?: string[]}) {
  const t = useAiTarget({
    kind: 'check_row',
    label: `${words(check.rule)} · ${check.passed ? 'passed' : 'failed'}${check.detail ? ` · ${check.detail}` : ''}`,
    ids: {rule: check.rule, ...ids},
    term: 'acom:Check',
    blockId,
  });
  return (
    <HStack ref={t.ref} {...t.props} gap={3} vAlign="start" paddingBlock={1}>
      <Icon icon={check.passed ? 'success' : 'error'} color={check.passed ? 'success' : 'error'} size="sm" label={check.passed ? 'Passed' : 'Failed'} />
      <StackItem size="fill">
        <VStack gap={0.5}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Link href={withParams('/bo/policies', {key: check.rule})} color="primary">
              {words(check.rule)}
            </Link>
            <Text type="supporting" color="secondary">
              {check.rule}
            </Text>
          </HStack>
          {check.detail ? (
            <Text type="supporting" color="secondary">
              {check.detail}
            </Text>
          ) : null}
          {!check.passed && check.fix_tool ? (
            <Text type="supporting" color="secondary">
              {`The agent can fix this with ${check.fix_tool}.`}
            </Text>
          ) : null}
          <SuggestedIntents intents={intents} target={t} />
        </VStack>
      </StackItem>
      <AskButton target={t} label={words(check.rule)} />
    </HStack>
  );
}

/** Rule checks, failed first as the API ordered them otherwise. */
export function ChecksList({
  checks,
  ids = {},
  blockId,
  intentsFor,
}: {
  checks: Check[];
  ids?: Record<string, string>;
  blockId?: string;
  intentsFor?: IntentsFor;
}) {
  if (checks.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No rule checks for this change.
      </Text>
    );
  }
  const ordered = [...checks.filter(c => !c.passed), ...checks.filter(c => c.passed)];
  return (
    <VStack gap={1}>
      {ordered.map(c => (
        <CheckRow key={c.rule} check={c} ids={ids} blockId={blockId} intents={intentsFor?.('check_row', c.rule)} />
      ))}
    </VStack>
  );
}

/** The entity a change touches: its id, or the product it is about when the row has no id yet. */
export function changeSubject(c: Change): string {
  if (c.id) return c.id;
  for (const side of [c.to, c.from]) {
    if (side && typeof side === 'object') {
      const o = side as Record<string, unknown>;
      for (const k of ['product_id', 'variant_id', 'claim_id', 'promotion_id', 'slot_id', 'id', 'code']) {
        if (typeof o[k] === 'string') return o[k] as string;
      }
    }
  }
  return c.entity;
}

/** Before and after as text, without the fields both sides share. */
export function beforeAfter(c: Change): {from: string; to: string} {
  const from = c.from;
  const to = c.to;
  if (from && to && typeof from === 'object' && typeof to === 'object' && !Array.isArray(from) && !Array.isArray(to)) {
    const f = {...(from as Record<string, unknown>)};
    const t = {...(to as Record<string, unknown>)};
    for (const k of Object.keys(f)) {
      if (k in t && JSON.stringify(f[k]) === JSON.stringify(t[k]) && k !== 'price_currency') {
        delete f[k];
        delete t[k];
      }
    }
    return {from: value(f), to: value(t)};
  }
  return {from: from === undefined || from === null ? '—' : value(from), to: to === undefined || to === null ? '—' : value(to)};
}

/** The changes of a preview as a table. `total` above the rows shown adds a "more" line. */
export function ChangesTable({
  changes,
  total,
  ids = {},
  blockId,
  intentsFor,
}: {
  changes: Change[];
  total?: number;
  ids?: Record<string, string>;
  blockId?: string;
  intentsFor?: IntentsFor;
}) {
  if (changes.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        This preview changes nothing.
      </Text>
    );
  }
  return (
    <VStack gap={2}>
      <Table density="compact" verticalAlign="top">
        <TableHeader>
          <TableRow isHeaderRow>
            <TableHeaderCell>Change</TableHeaderCell>
            <TableHeaderCell>What</TableHeaderCell>
            <TableHeaderCell>Before</TableHeaderCell>
            <TableHeaderCell>After</TableHeaderCell>
            <TableHeaderCell>
              <Text type="supporting" color="secondary">
                Ask
              </Text>
            </TableHeaderCell>
          </TableRow>
        </TableHeader>
        <TableBody>
          {changes.map((c, i) => {
            const subject = changeSubject(c);
            const {from, to} = beforeAfter(c);
            const href = entityHref(subject);
            return (
              <TargetRow
                key={`${subject}-${i}`}
                target={{
                  kind: 'change_row',
                  label: `${words(c.op)} ${words(c.entity).toLowerCase()} ${subject}: ${from} → ${to}`,
                  ids: {[`${c.entity}_id`]: subject, ...ids},
                  term: `acom:${c.entity}`,
                  blockId,
                }}
                intents={intentsFor?.('change_row', subject)}>
                <TableCell>
                  <Token label={c.op} size="sm" color={c.op === 'exclude' ? 'gray' : c.op === 'delete' || c.op === 'revoke' ? 'red' : 'default'} />
                </TableCell>
                <TableCell>
                  <VStack gap={0}>
                    <Text>{words(c.entity)}</Text>
                    {href ? (
                      <Link href={href} color="secondary">
                        {subject}
                      </Link>
                    ) : (
                      <Text type="supporting" color="secondary">
                        {subject}
                      </Text>
                    )}
                    {c.note ? (
                      <Text type="supporting" color="secondary">
                        {c.note}
                      </Text>
                    ) : null}
                  </VStack>
                </TableCell>
                <TableCell>
                  <Text color="secondary" hasTabularNumbers>
                    {from}
                  </Text>
                </TableCell>
                <TableCell>
                  <Text hasTabularNumbers weight="medium">
                    {to}
                  </Text>
                </TableCell>
              </TargetRow>
            );
          })}
        </TableBody>
      </Table>
      {total !== undefined && total > changes.length ? (
        <Text type="supporting" color="secondary">
          {`Showing the first ${changes.length} of ${total} changes in this item.`}
        </Text>
      ) : null}
    </VStack>
  );
}
