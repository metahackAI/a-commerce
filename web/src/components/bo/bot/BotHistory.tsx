'use client';

// Ailism Bot's history (list_bot_cycles, get_bot_cycle): every cycle it ran, and one cycle in
// full — its summary, the warning signs found by rule, the findings, each proposal with what became
// of it (opened as a decision, or why not), what it cost, and the reads it was based on.

import {Badge} from '@astryxdesign/core/Badge';
import {Banner} from '@astryxdesign/core/Banner';
import {Card} from '@astryxdesign/core/Card';
import {CodeBlock} from '@astryxdesign/core/CodeBlock';
import {Collapsible} from '@astryxdesign/core/Collapsible';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {count, dateTime, words} from '@/app/bo/_lib/format';
import type {ApiErrorBody} from '@/lib/api';
import type {BotCycle, BotCycleSummary} from '@/app/bo/_lib/types';

import {ErrorState} from '../ErrorState';
import {PageFrame, PageSection} from '../PageFrame';
import {Pager} from '../Pager';
import {StateToken} from '../StateToken';
import {DataTable} from '../tables';
import {TargetBlock, TargetCard} from '../targets';
import {AREA_LABEL, BotTabs, RiskToken, RunNowButton, effectText, useBotEvents} from './parts';

export function BotHistory({
  items,
  total,
  params,
  nextCursor,
  error,
  isOff,
}: {
  items: BotCycleSummary[];
  total: number;
  params: Record<string, string | string[] | undefined>;
  nextCursor: string | null;
  error: ApiErrorBody | null;
  isOff: boolean;
}) {
  useBotEvents();
  return (
    <PageFrame
      title="History"
      back={{href: '/bo/bot', label: 'Ailism Bot'}}
      description="Every cycle the bot ran: what it read, what it found, and what it proposed."
      actions={<RunNowButton isOff={isOff} />}
      toolbar={<BotTabs value="history" />}>
      {error ? <ErrorState error={error} title="The cycles could not be loaded." /> : null}
      <DataTable
        rows={items}
        rowKey={c => c.id}
        target={c => ({kind: 'bot_cycle', label: `Cycle ${dateTime(c.at)} · ${c.state}${c.summary ? ` · ${c.summary}` : ''}`, ids: {bot_cycle_id: c.id}, term: 'acom:BotCycle'})}
        empty={{title: 'No cycles yet', description: 'Press Run now, or wait for the schedule.'}}
        columns={[
          {header: 'When', width: '18%', cell: c => <Link href={`/bo/bot/history/${encodeURIComponent(c.id)}`}>{dateTime(c.at)}</Link>},
          {header: 'State', cell: c => <StateToken state={c.state} />},
          {header: 'Started by', cell: c => words(c.trigger)},
          {header: 'Summary', width: '40%', cell: c => <Text type="supporting">{c.summary ?? c.error ?? '—'}</Text>},
          {header: 'Opened', numeric: true, cell: c => `${c.opened} of ${c.proposals}`},
          {header: 'Signs', numeric: true, cell: c => count(c.anomalies)},
        ]}
      />
      <Pager base="/bo/bot/history" params={params} total={total} shown={items.length} nextCursor={nextCursor} noun="cycles" />
    </PageFrame>
  );
}

const SEVERITY_COLOR: Record<string, 'red' | 'orange' | 'gray'> = {high: 'red', medium: 'orange', low: 'gray'};

export function BotCycleView({cycle}: {cycle: BotCycle}) {
  useBotEvents();
  const report = cycle.report;
  const proposals = report?.proposals ?? [];
  const reads = cycle.snapshot.reads ?? [];
  const u = cycle.usage;
  return (
    <PageFrame
      title={`Cycle ${dateTime(cycle.at)}`}
      back={{href: '/bo/bot/history', label: 'History'}}
      meta={
        <>
          <StateToken state={cycle.state} size="md" />
          <Text type="supporting" color="secondary">
            {`${words(cycle.trigger)}${cycle.requested_by ? ` by ${cycle.requested_by}` : ''} · ${words(cycle.mode)} mode${cycle.end_time ? ` · ended ${dateTime(cycle.end_time)}` : ''}`}
          </Text>
        </>
      }
      description={report?.summary ?? undefined}
      panel={
        <PageSection title="What it cost">
          <MetadataList>
            <MetadataListItem label="Model">{u.model ?? '—'}</MetadataListItem>
            <MetadataListItem label="Model calls">{count(u.model_calls ?? 0)}</MetadataListItem>
            <MetadataListItem label="Tokens in / out">{`${count(u.input_tokens ?? 0)} / ${count(u.output_tokens ?? 0)}`}</MetadataListItem>
            <MetadataListItem label="Store reads">{count(u.reads ?? reads.length)}</MetadataListItem>
            <MetadataListItem label="Decisions opened">{count(cycle.decisions.length)}</MetadataListItem>
          </MetadataList>
        </PageSection>
      }>
      {cycle.state === 'observing' ? (
        <Banner status="info" title="Reading and analysing" description="This page fills in when the report comes back." />
      ) : null}
      {cycle.error ? <Banner status="error" title="The cycle failed" description={cycle.error} /> : null}

      <PageSection title="Warning signs" count={cycle.anomalies.length} description="Found by rule in what the bot read, before any model looked at it.">
        {cycle.anomalies.length === 0 ? (
          <Text type="supporting" color="secondary">
            None.
          </Text>
        ) : (
          <VStack gap={2}>
            {cycle.anomalies.map(a => (
              <TargetBlock key={`${a.code}${a.text}`} target={{kind: 'bot_anomaly', label: a.text, ids: {bot_cycle_id: cycle.id, code: a.code}, term: 'acom:anomalies'}}>
                <HStack gap={2} vAlign="center" wrap="wrap">
                  <Token label={words(a.severity)} size="sm" color={SEVERITY_COLOR[a.severity] ?? 'gray'} />
                  <Token label={AREA_LABEL[a.area] ?? words(a.area)} size="sm" />
                  <Text>{a.text}</Text>
                </HStack>
              </TargetBlock>
            ))}
          </VStack>
        )}
      </PageSection>

      {report?.findings && report.findings.length > 0 ? (
        <PageSection title="Findings" count={report.findings.length}>
          <VStack gap={2}>
            {report.findings.map(f => (
              <HStack key={`${f.area}${f.text}`} gap={2} vAlign="center" wrap="wrap">
                <Token label={AREA_LABEL[f.area] ?? words(f.area)} size="sm" />
                <Text>{f.text}</Text>
              </HStack>
            ))}
          </VStack>
        </PageSection>
      ) : null}

      <PageSection title="Proposals" count={proposals.length} description="Each one was opened as a decision, or left with the reason.">
        {proposals.length === 0 ? (
          <Text type="supporting" color="secondary">
            {cycle.state === 'observing' ? 'Not yet.' : 'It proposed nothing.'}
          </Text>
        ) : (
          <VStack gap={3}>
            {proposals.map((p, i) => {
              const effect = effectText(p.expected_effect);
              return (
                <TargetCard
                  key={`${i}${p.title}`}
                  target={{kind: 'bot_proposal', label: `${p.title}${p.outcome?.skipped ? ` · not opened: ${p.outcome.skipped}` : ''}`, ids: {bot_cycle_id: cycle.id, proposal: String(i + 1)}, term: 'acom:BotDecision'}}
                  heading={
                    <>
                      <Text weight="semibold">{p.title}</Text>
                      <HStack gap={2} vAlign="center" wrap="wrap">
                        <Token label={AREA_LABEL[p.area] ?? words(p.area)} size="sm" />
                        {p.risk ? <RiskToken risk={p.risk} /> : null}
                        {p.outcome?.opened ? (
                          <Link href={`/bo/bot/decisions?view=all&decision=${encodeURIComponent(p.outcome.opened)}`}>Opened as {p.outcome.opened}</Link>
                        ) : null}
                      </HStack>
                    </>
                  }>
                  <VStack gap={1}>
                    {p.reason ? <Text>{p.reason}</Text> : null}
                    {(p.evidence ?? []).map(e => (
                      <Text key={e} type="supporting" color="secondary">
                        · {e}
                      </Text>
                    ))}
                    {effect ? (
                      <Text type="supporting" color="secondary">
                        Expected: {effect}
                      </Text>
                    ) : null}
                    {p.outcome?.skipped ? (
                      <Text type="supporting" color="secondary">
                        Not opened: {p.outcome.skipped}
                      </Text>
                    ) : null}
                  </VStack>
                </TargetCard>
              );
            })}
          </VStack>
        )}
      </PageSection>

      <PageSection title="What it read" count={reads.length} description="The snapshot: each read tool, its arguments and what it returned, in order.">
        <VStack gap={2}>
          {reads.map((r, i) => (
            <Card key={`${i}${r.tool}`} variant="muted">
              <Collapsible
                trigger={
                  <HStack gap={2} vAlign="center" wrap="wrap">
                    <Token label={AREA_LABEL[r.area] ?? words(r.area)} size="sm" />
                    <Text weight="semibold">{r.tool}</Text>
                    {r.error ? <Badge label="failed" /> : null}
                  </HStack>
                }>
                <CodeBlock language="json" code={JSON.stringify(r.error ? {args: r.args, error: r.error} : {args: r.args, result: r.result}, null, 2)} />
              </Collapsible>
            </Card>
          ))}
        </VStack>
      </PageSection>
    </PageFrame>
  );
}
