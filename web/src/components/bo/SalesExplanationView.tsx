'use client';

// The result of `explain_sales_change` on the Insights page: the day against its basis, the
// causes the data shows with their estimated effect, and the part left unexplained. The amounts
// are the tool's own and add up to the difference as sent; this view only lays them out.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {MetadataList} from '@astryxdesign/core/MetadataList';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import {count, date, dateTime, isMoney, money, percent, value, words} from '@/app/bo/_lib/format';
import type {SalesExplanation} from '@/app/bo/_lib/types';

import {SuggestedIntents, TargetFact, TargetRow} from './targets';

const show = (v: unknown) => (isMoney(v) ? money(v) : value(v));

/** What a cause rests on, in a line: the products and sizes that ran out, or the send that failed. */
function grounds(item: SalesExplanation['items'][number]): string {
  if (item.products && item.products.length > 0) {
    const names = item.products.map(p => `${p.name ?? p.product_id} (${p.sizes.join(', ') || 'all sizes'})`);
    const back = item.products.filter(p => p.availability_starts).map(p => `${p.name ?? p.product_id} back ${date(p.availability_starts)}`);
    return [names.join(', '), ...back].join('; ');
  }
  if (item.campaign) {
    return [item.campaign.name, item.campaign.error_message, item.campaign.scheduled_time ? `due ${dateTime(item.campaign.scheduled_time)}` : null]
      .filter(Boolean)
      .join(' · ');
  }
  return item.evidence.join(', ');
}

const BASIS: Record<string, string> = {
  same_weekday_last_4_weeks: 'Same weekday, average of the last 4 weeks',
  previous_period: 'The days just before',
  custom: 'The days you chose',
};

export function SalesExplanationView({explanation, dayLabel, question}: {explanation: SalesExplanation; dayLabel: string; question: string}) {
  const e = explanation;
  const fig = (id: string, label: string) => ({kind: 'figure', label, ids: {figure: id}, term: 'schema:Observation'});
  return (
    <VStack gap={4}>
      <MetadataList columns="multi">
        <TargetFact label={`Revenue, ${dayLabel}`} target={fig('revenue', `Revenue on ${dayLabel}: ${money(e.period.revenue)}`)}>
          <Text weight="semibold" hasTabularNumbers>
            {money(e.period.revenue)}
          </Text>
        </TargetFact>
        <TargetFact label="Basis" target={fig('basis', `Basis: ${money(e.compare_to.revenue)}`)}>
          <VStack gap={0}>
            <Text weight="semibold" hasTabularNumbers>
              {money(e.compare_to.revenue)}
            </Text>
            <Text type="supporting" color="secondary">
              {BASIS[e.compare_to.basis] ?? words(e.compare_to.basis)}
            </Text>
          </VStack>
        </TargetFact>
        <TargetFact label="Difference" target={fig('difference', `Difference: ${show(e.gap)} (${percent(e.change_rate)})`)}>
          <Text weight="semibold" hasTabularNumbers>
            {show(e.gap)} ({percent(e.change_rate)})
          </Text>
        </TargetFact>
        <TargetFact label="Orders" target={fig('orders', `Orders: ${count(e.period.order_count)} against ${count(e.compare_to.order_count)}`)}>
          <VStack gap={0}>
            <Text weight="semibold" hasTabularNumbers>
              {count(e.period.order_count)}
            </Text>
            <Text type="supporting" color="secondary" hasTabularNumbers>
              basis {count(e.compare_to.order_count)}
            </Text>
          </VStack>
        </TargetFact>
      </MetadataList>

      <Table density="compact" verticalAlign="top">
        <TableHeader>
          <TableRow isHeaderRow>
            <TableHeaderCell>Cause</TableHeaderCell>
            <TableHeaderCell>What the data shows</TableHeaderCell>
            <TableHeaderCell>Expected</TableHeaderCell>
            <TableHeaderCell>Estimated effect</TableHeaderCell>
            <TableHeaderCell />
          </TableRow>
        </TableHeader>
        <TableBody>
          {e.items.map(i => (
            <TargetRow key={i.cause} target={{kind: 'cause', label: `${i.detail}: ${show(i.amount)}`, ids: {cause: i.cause, evidence: i.evidence.join(',')}}}>
              <TableCell>
                <VStack gap={0}>
                  <Text weight="semibold">{i.detail}</Text>
                  <Text type="supporting" color="secondary">
                    {words(i.cause)}
                  </Text>
                </VStack>
              </TableCell>
              <TableCell>
                <Text>{grounds(i)}</Text>
              </TableCell>
              <TableCell>
                <HStack hAlign="end">
                  <Text hasTabularNumbers>{show(i.expected)}</Text>
                </HStack>
              </TableCell>
              <TableCell>
                <HStack hAlign="end">
                  <Text weight="semibold" hasTabularNumbers>
                    {show(i.amount)}
                  </Text>
                </HStack>
              </TableCell>
            </TargetRow>
          ))}
          <TargetRow target={{kind: 'cause', label: `Not explained: ${show(e.unexplained)}`, ids: {cause: 'unexplained'}}}>
            <TableCell>
              <Text weight="semibold">Not explained</Text>
            </TableCell>
            <TableCell>
              <Text color="secondary">Not shown by stock or campaign data</Text>
            </TableCell>
            <TableCell />
            <TableCell>
              <HStack hAlign="end">
                <Text weight="semibold" hasTabularNumbers>
                  {show(e.unexplained)}
                </Text>
              </HStack>
            </TableCell>
          </TargetRow>
        </TableBody>
      </Table>

      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text type="supporting" color="secondary">
          The agent can take it from here:
        </Text>
        <SuggestedIntents intents={[question]} />
      </HStack>
    </VStack>
  );
}
