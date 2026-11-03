'use client';

// Ailism Bot's dashboard (docs/api/platform.md#bot): how it is set to run, the counts of its
// decisions, the last cycle and its warning signs, the decisions waiting for a person, open
// feedback and directives, and recent cycles. Reloads when a cycle, task or change set moves.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';

import {count, dateTime, words} from '@/app/bo/_lib/format';
import type {BotCycleSummary, BotDecision, BotFeedback, BotStatus} from '@/app/bo/_lib/types';

import {PageFrame, PageSection} from '../PageFrame';
import {StateToken} from '../StateToken';
import {DataTable} from '../tables';
import {TargetFact} from '../targets';
import {BotTabs, DecisionCard, FeedbackForm, FeedbackList, RunNowButton, useBotEvents} from './parts';

const MODE: Record<string, string> = {
  off: 'Off',
  observe: 'Observe: reports only',
  propose: 'Propose: drafts changes for you to decide',
};

function every(minutes: number): string {
  if (minutes % 60 === 0) return minutes === 60 ? 'every hour' : `every ${minutes / 60} hours`;
  return `every ${minutes} minutes`;
}

function Tile({label, value, note}: {label: string; value: string; note?: string}) {
  return (
    <TargetFact target={{kind: 'bot_figure', label: `${label}: ${value}`, ids: {figure: label}, term: 'acom:Bot'}} label={label}>
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
    </TargetFact>
  );
}

export function BotDashboard({
  status,
  waiting,
  cycles,
  feedback,
}: {
  status: BotStatus;
  waiting: BotDecision[];
  cycles: BotCycleSummary[];
  feedback: BotFeedback[];
}) {
  useBotEvents();
  const s = status.settings;
  const d = status.decisions;
  const n = (...states: string[]) => states.reduce((sum, k) => sum + (d[k] ?? 0), 0);
  const last = status.last_cycle;
  const observing = last?.state === 'observing';
  const areasOn = Object.entries(s.areas).filter(([, on]) => on).length;
  return (
    <PageFrame
      title="Ailism Bot"
      meta={
        <>
          <StateToken state={s.mode === 'off' ? 'off' : 'active'} size="md" />
          <Text type="supporting" color="secondary">
            {MODE[s.mode] ?? words(s.mode)}
            {s.mode !== 'off' ? ` · ${every(s.every_minutes)}` : ''}
            {status.next_run_at ? ` · next ${dateTime(status.next_run_at)}` : ''}
          </Text>
        </>
      }
      description="The manager AI of the back office. It reads the store on its schedule, finds what is worth fixing, and opens each proposal as a task the operations agent drafts as a change set. Nothing changes until a person approves."
      actions={
        <>
          <RunNowButton isOff={s.mode === 'off'} />
          <Button label="Settings" variant="secondary" href="/bo/bot/settings" />
        </>
      }
      toolbar={<BotTabs value="dashboard" />}>
      {status.acts_as === null && s.mode === 'propose' ? (
        <Banner
          status="warning"
          title="No operator to draft for"
          description="Choose the operator the bot's drafts are made for in Settings, or the proposals stay as tasks for a person to open."
        />
      ) : null}
      <Grid columns={{minWidth: 170}} gap={3}>
        <Tile label="Waiting for you" value={count(n('waiting', 'needs_you'))} note="change sets to decide" />
        <Tile label="Drafting" value={count(n('drafting'))} note="the agent is on it" />
        <Tile label="Live" value={count(n('live', 'running'))} note="approved and ran" />
        <Tile label="Turned down" value={count(n('rejected', 'dismissed', 'undone'))} note="rejected, put away, undone" />
        <Tile label="Open feedback" value={count(status.open_feedback)} note="read in every analysis" />
        <Tile label="Areas watched" value={`${areasOn} of 8`} note={`up to ${s.max_proposals_per_cycle} proposals a cycle`} />
      </Grid>

      <PageSection
        title="Last cycle"
        actions={last ? <Button label="Open" size="sm" variant="ghost" href={`/bo/bot/history/${encodeURIComponent(last.id)}`} /> : undefined}>
        {last ? (
          <Card>
            <VStack gap={2}>
              <HStack gap={2} vAlign="center" wrap="wrap">
                <StateToken state={last.state} />
                <Text type="supporting" color="secondary">
                  {`${words(last.trigger)} · ${dateTime(last.at)}`}
                </Text>
                {observing ? <Spinner size="sm" /> : null}
              </HStack>
              <Text>
                {observing
                  ? 'Reading the store and analysing it. Its proposals appear here as soon as they are opened.'
                  : (last.summary ?? last.error ?? 'No summary.')}
              </Text>
            </VStack>
          </Card>
        ) : (
          <Text type="supporting" color="secondary">
            The bot has not run yet. Press Run now, or wait for its schedule.
          </Text>
        )}
      </PageSection>

      <PageSection
        title="Waiting for your decision"
        count={waiting.length}
        actions={<Button label="All decisions" size="sm" variant="ghost" href="/bo/bot/decisions" />}>
        {waiting.length === 0 ? (
          <Text type="supporting" color="secondary">
            Nothing waits for you from the bot.
          </Text>
        ) : (
          <VStack gap={3}>
            {waiting.map(dec => (
              <DecisionCard key={dec.id} decision={dec} compact />
            ))}
          </VStack>
        )}
      </PageSection>

      <PageSection title="Feedback and directives" count={feedback.length} description="Tell the bot what to do differently. It reads every open item in each analysis.">
        <FeedbackList items={feedback} />
        <FeedbackForm />
      </PageSection>

      <PageSection
        title="Recent cycles"
        actions={<Button label="History" size="sm" variant="ghost" href="/bo/bot/history" />}>
        <DataTable
          rows={cycles}
          rowKey={c => c.id}
          target={c => ({kind: 'bot_cycle', label: `Cycle ${dateTime(c.at)} · ${c.state}`, ids: {bot_cycle_id: c.id}, term: 'acom:BotCycle'})}
          empty={{title: 'No cycles yet'}}
          columns={[
            {header: 'When', width: '22%', cell: c => <Link href={`/bo/bot/history/${encodeURIComponent(c.id)}`}>{dateTime(c.at)}</Link>},
            {header: 'State', cell: c => <StateToken state={c.state} />},
            {header: 'Started by', cell: c => words(c.trigger)},
            {header: 'Opened', numeric: true, cell: c => `${c.opened} of ${c.proposals}`},
            {header: 'Signs', numeric: true, cell: c => count(c.anomalies)},
          ]}
        />
      </PageSection>
    </PageFrame>
  );
}
