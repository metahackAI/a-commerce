'use client';

// Ailism Bot's history (list_bot_cycles, get_bot_cycle): every cycle it ran, and one cycle in
// full — what it concluded, what it proposed and what became of each proposal, the warning signs
// found by rule, its findings by area, what it cost, and the reads it was based on: each a line
// that opens to its items by name, with the raw result one more click away.

import {Banner} from '@astryxdesign/core/Banner';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {Card} from '@astryxdesign/core/Card';
import {CodeBlock} from '@astryxdesign/core/CodeBlock';
import {Collapsible, CollapsibleGroup} from '@astryxdesign/core/Collapsible';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
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
import {TargetBlock, TargetCard, TargetRegion} from '../targets';
import {AREA_LABEL, BotTabs, RiskToken, RunNowButton, effectText, useBotEvents} from './parts';
import {argsText, readLabel, readOutcome, readRows, recordFacts, took} from './present';

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

function Stat({label, value, note}: {label: string; value: string; note?: string}) {
  return (
    <TargetRegion target={{kind: 'bot_figure', label: `${label}: ${value}`, ids: {figure: label}, term: 'acom:BotCycle'}}>
      <Card variant="muted">
        <VStack gap={1}>
          <Text type="supporting" color="secondary">
            {label}
          </Text>
          <Text type="large" weight="semibold">
            {value}
          </Text>
          {note ? (
            <Text type="supporting" color="secondary">
              {note}
            </Text>
          ) : null}
        </VStack>
      </Card>
    </TargetRegion>
  );
}

type Read = NonNullable<BotCycle['snapshot']['reads']>[number];

/** One read: a line with what it looked at and found; open it for its items, then the raw result. */
function ReadLine({read}: {read: Read}) {
  const rows = readRows(read.result);
  const facts = rows.length === 0 ? recordFacts(read.result) : [];
  const outcome = readOutcome(read.result, read.error);
  return (
    <Collapsible
      defaultIsOpen={false}
      trigger={
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Token label={AREA_LABEL[read.area] ?? words(read.area)} size="sm" />
          <StackItem size="fill">
            <Text weight="semibold">{readLabel(read.tool, read.args)}</Text>
          </StackItem>
          {read.error ? (
            <Token label={outcome} size="sm" color="red" />
          ) : (
            <Text type="supporting" color="secondary">
              {outcome}
            </Text>
          )}
        </HStack>
      }>
      <VStack gap={2} paddingBlock={2}>
        <Text type="supporting" color="secondary">
          {read.tool}
          {argsText(read.args) ? ` · ${argsText(read.args)}` : ''}
        </Text>
        {read.error ? <Text>{read.error}</Text> : null}
        {rows.length > 0 ? (
          <MetadataList>
            {rows.map(r => (
              <MetadataListItem key={r.key} label={r.title}>
                {r.facts.join(' · ') || '—'}
              </MetadataListItem>
            ))}
          </MetadataList>
        ) : null}
        {facts.length > 0 ? (
          <VStack gap={1}>
            {facts.map(f => (
              <Text key={f} type="supporting">
                {f}
              </Text>
            ))}
          </VStack>
        ) : null}
        {read.result !== undefined ? (
          <Collapsible
            defaultIsOpen={false}
            trigger={
              <Text type="supporting" color="secondary">
                Raw result
              </Text>
            }>
            <CodeBlock language="json" code={JSON.stringify(read.result, null, 2)} />
          </Collapsible>
        ) : null}
      </VStack>
    </Collapsible>
  );
}

export function BotCycleView({cycle}: {cycle: BotCycle}) {
  useBotEvents();
  const report = cycle.report;
  const proposals = report?.proposals ?? [];
  const opened = proposals.filter(p => p.outcome?.opened).length;
  const reads = cycle.snapshot.reads ?? [];
  const failedReads = reads.filter(r => r.error).length;
  const u = cycle.usage;
  const duration = took(cycle.at, cycle.end_time);
  const findings = report?.findings ?? [];
  return (
    <PageFrame
      title={`Cycle · ${dateTime(cycle.at)}`}
      back={{href: '/bo/bot/history', label: 'History'}}
      meta={
        <>
          <StateToken state={cycle.state} size="md" />
          <Text type="supporting" color="secondary">
            {`${cycle.trigger === 'manual' ? `Run now${cycle.requested_by ? ` by ${cycle.requested_by}` : ''}` : 'On schedule'} · ${words(cycle.mode)} mode${duration ? ` · took ${duration}` : ''}`}
          </Text>
        </>
      }
      toolbar={<BotTabs value="history" />}>
      {cycle.state === 'observing' ? (
        <Banner status="info" title="Reading and analysing" description="This page fills in when the report comes back." />
      ) : null}
      {cycle.error ? <Banner status="error" title="The cycle failed" description={cycle.error} /> : null}

      {report?.summary ? (
        <PageSection title="What the bot concluded">
          <Blockquote>{report.summary}</Blockquote>
        </PageSection>
      ) : null}

      <Grid columns={{minWidth: 170}} gap={3}>
        <Stat label="Proposals opened" value={`${opened} of ${proposals.length}`} note={proposals.length > opened ? `${proposals.length - opened} not opened, see why below` : 'each became a task'} />
        <Stat label="Warning signs" value={count(cycle.anomalies.length)} note="found by rule" />
        <Stat label="Findings" value={count(findings.length)} note="what stood out" />
        <Stat label="Store reads" value={count(u.reads ?? reads.length)} note={failedReads ? `${failedReads} could not be read` : 'read tools only'} />
        <Stat label="Model" value={u.model ?? '—'} note={`${count(u.model_calls ?? 0)} call · ${count(u.input_tokens ?? 0)} in / ${count(u.output_tokens ?? 0)} out tokens`} />
      </Grid>

      <PageSection title="Proposals" count={proposals.length}>
        {proposals.length === 0 ? (
          <Text type="supporting" color="secondary">
            {cycle.state === 'observing' ? 'Not yet.' : 'Nothing new to propose: what it found is already being worked on, or not worth a change. See the findings.'}
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
                      <HStack gap={2} vAlign="center" wrap="wrap">
                        <Text weight="semibold">{p.title}</Text>
                        {p.outcome?.opened ? <StateToken state="opened" /> : <StateToken state="skipped" />}
                      </HStack>
                      <HStack gap={2} vAlign="center" wrap="wrap">
                        <Token label={AREA_LABEL[p.area] ?? words(p.area)} size="sm" />
                        {p.risk ? <RiskToken risk={p.risk} /> : null}
                      </HStack>
                    </>
                  }>
                  <VStack gap={2}>
                    {p.reason ? <Text>{p.reason}</Text> : null}
                    {(p.evidence ?? []).length > 0 ? (
                      <VStack gap={1}>
                        {(p.evidence ?? []).map(e => (
                          <Text key={e} type="supporting" color="secondary">
                            · {e}
                          </Text>
                        ))}
                      </VStack>
                    ) : null}
                    {effect ? (
                      <Text type="supporting" color="secondary">
                        Expected: {effect}
                      </Text>
                    ) : null}
                    {p.outcome?.opened ? (
                      <Link href={`/bo/bot/decisions?view=all&decision=${encodeURIComponent(p.outcome.opened)}`}>Open the decision</Link>
                    ) : p.outcome?.skipped ? (
                      <Text type="supporting">Not opened: {p.outcome.skipped}</Text>
                    ) : null}
                  </VStack>
                </TargetCard>
              );
            })}
          </VStack>
        )}
      </PageSection>

      <PageSection title="Warning signs" count={cycle.anomalies.length} description="Found by rule in what the bot read, before any model looked at it.">
        {cycle.anomalies.length === 0 ? (
          <Text type="supporting" color="secondary">
            None.
          </Text>
        ) : (
          <Card>
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
          </Card>
        )}
      </PageSection>

      {findings.length > 0 ? (
        <PageSection title="Findings by area" count={findings.length}>
          <Card>
            <MetadataList>
              {findings.map(f => (
                <MetadataListItem key={`${f.area}${f.text}`} label={AREA_LABEL[f.area] ?? words(f.area)}>
                  {f.text}
                </MetadataListItem>
              ))}
            </MetadataList>
          </Card>
        </PageSection>
      ) : null}

      <PageSection title="What it read" count={reads.length} description="Each read in order. Open one to see what it found; the raw result is inside.">
        {reads.length === 0 ? (
          <Text type="supporting" color="secondary">
            {cycle.state === 'observing' ? 'Not yet.' : 'Nothing was read.'}
          </Text>
        ) : (
          <Card padding={0}>
            <CollapsibleGroup hasDividers>
              {reads.map((r, i) => (
                <ReadLine key={`${i}${r.tool}`} read={r} />
              ))}
            </CollapsibleGroup>
          </Card>
        )}
      </PageSection>
    </PageFrame>
  );
}
