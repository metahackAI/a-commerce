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
import {ruleName, value, words} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {Change, Check} from '@/app/bo/_lib/types';

import {AskButton, AskHeaderCell, SuggestedIntents, TargetRow, useAskReveal} from './targets';

/** Intents for an element from a block's `suggested_intents` (`element: "change_row:prd_x"`). */
export type IntentsFor = (kind: string, id: string) => string[] | undefined;

/** A check and, in a change set, the item it was run on. */
export type ItemCheck = Check & {subject?: string};

function CheckRow({check, ids, blockId, intents}: {check: ItemCheck; ids: Record<string, string>; blockId?: string; intents?: string[]}) {
  const t = useAiTarget({
    kind: 'check_row',
    label: `${ruleName(check.rule)} · ${check.passed ? 'passed' : 'failed'}${check.subject ? ` · ${check.subject}` : ''}${check.detail ? ` · ${check.detail}` : ''}`,
    ids: {rule: check.rule, ...ids},
    term: 'acom:Check',
    blockId,
  });
  const reveal = useAskReveal();
  return (
    <HStack ref={t.ref} {...t.props} {...reveal.container} gap={3} vAlign="start" paddingBlock={1}>
      <Icon icon={check.passed ? 'success' : 'error'} color={check.passed ? 'success' : 'error'} size="sm" label={check.passed ? 'Passed' : 'Failed'} />
      <StackItem size="fill">
        <VStack gap={0.5}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Link href={withParams('/bo/policies', {key: check.rule})} color="primary">
              {ruleName(check.rule)}
            </Link>
            <Text type="supporting" color="secondary">
              {check.rule}
            </Text>
          </HStack>
          {check.detail || check.subject ? (
            <Text type="supporting" color="secondary">
              {[check.subject, check.detail].filter(Boolean).join(' · ')}
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
      <StackItem {...reveal.ask}>
        <AskButton target={t} label={ruleName(check.rule)} />
      </StackItem>
    </HStack>
  );
}

/** Several passed checks of one rule (a price floor on nine prices): one row and their details. */
function PassedGroup({rule, checks, ids, blockId}: {rule: string; checks: ItemCheck[]; ids: Record<string, string>; blockId?: string}) {
  const t = useAiTarget({
    kind: 'check_row',
    label: `${ruleName(rule)} · ${checks.length} of ${checks.length} passed`,
    ids: {rule, ...ids},
    term: 'acom:Check',
    blockId,
  });
  const reveal = useAskReveal();
  const shown = checks.slice(0, GROUP_SHOWN);
  return (
    <HStack ref={t.ref} {...t.props} {...reveal.container} gap={3} vAlign="start" paddingBlock={1}>
      <Icon icon="success" color="success" size="sm" label="Passed" />
      <StackItem size="fill">
        <VStack gap={0.5}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Link href={withParams('/bo/policies', {key: rule})} color="primary">
              {ruleName(rule)}
            </Link>
            <Text type="supporting" color="secondary">
              {`${rule} · ${checks.length} of ${checks.length} passed`}
            </Text>
          </HStack>
          {shown.map((c, i) => (
            <Text key={`${c.subject ?? ''}-${i}`} type="supporting" color="secondary">
              {[c.subject, c.detail].filter(Boolean).join(' · ') || 'Passed'}
            </Text>
          ))}
          {checks.length > shown.length ? (
            <Text type="supporting" color="secondary">
              {`and ${checks.length - shown.length} more`}
            </Text>
          ) : null}
        </VStack>
      </StackItem>
      <StackItem {...reveal.ask}>
        <AskButton target={t} label={ruleName(rule)} />
      </StackItem>
    </HStack>
  );
}

/** Details listed under a group of passed checks before "and N more". */
const GROUP_SHOWN = 6;

/**
 * Rule checks, failed first. A failed check gets its own row; passed checks of the same rule
 * share one row, so nine price-floor passes read as one rule held nine times.
 */
export function ChecksList({
  checks,
  ids = {},
  blockId,
  intentsFor,
}: {
  checks: ItemCheck[];
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
  const failed = checks.filter(c => !c.passed);
  const passed = new Map<string, ItemCheck[]>();
  for (const c of checks.filter(c => c.passed)) passed.set(c.rule, [...(passed.get(c.rule) ?? []), c]);
  return (
    <VStack gap={1}>
      {failed.map((c, i) => (
        <CheckRow key={`failed-${c.rule}-${i}`} check={c} ids={ids} blockId={blockId} intents={intentsFor?.('check_row', c.rule)} />
      ))}
      {[...passed].map(([rule, group]) =>
        group.length === 1 ? (
          <CheckRow key={rule} check={group[0]} ids={ids} blockId={blockId} intents={intentsFor?.('check_row', rule)} />
        ) : (
          <PassedGroup key={rule} rule={rule} checks={group} ids={ids} blockId={blockId} />
        ),
      )}
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

/**
 * A side of a change whose `price` (and `cost`) came without a currency, as older previews wrote
 * them: read in `currency`, the one the tool input named, so it shows as money.
 */
function withCurrency(side: unknown, currency: string | undefined): unknown {
  if (!currency || !side || typeof side !== 'object' || Array.isArray(side)) return side;
  const o = {...(side as Record<string, unknown>)};
  if (typeof o.price === 'number' && typeof o.price_currency !== 'string') o.price_currency = currency;
  if (typeof o.cost === 'number') o.cost = {value: o.cost, currency};
  return o;
}

/** A side without the id the "What" column already shows. */
function withoutSubject(side: unknown, subject: string): unknown {
  if (!side || typeof side !== 'object' || Array.isArray(side)) return side;
  return Object.fromEntries(Object.entries(side as Record<string, unknown>).filter(([k, v]) => !(k.endsWith('_id') && v === subject)));
}

/** Before and after as text, without the fields both sides share. */
export function beforeAfter(c: Change, currency?: string): {from: string; to: string} {
  const subject = changeSubject(c);
  const from = withoutSubject(withCurrency(c.from, currency), subject);
  const to = withoutSubject(withCurrency(c.to, currency), subject);
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
  currency,
  ids = {},
  blockId,
  intentsFor,
}: {
  changes: Change[];
  total?: number;
  /** The currency of the tool input, for prices a change wrote without one. */
  currency?: string;
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
            <AskHeaderCell />
          </TableRow>
        </TableHeader>
        <TableBody>
          {changes.map((c, i) => {
            const subject = changeSubject(c);
            const {from, to} = beforeAfter(c, currency);
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
