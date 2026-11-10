'use client';

// Ailism Customer Bot's dashboard (docs/api/platform.md#customer_bot): how it is set to run and
// when it runs next, what its shoppers have done so far, the run at work with its sessions as they
// shop, and the recent runs and sessions. Refreshes while shoppers are at work.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';

import {count, dateTime, money, words} from '@/app/bo/_lib/format';
import type {CustomerBotRun, CustomerBotRunDetail, CustomerBotSession, CustomerBotStatus} from '@/app/bo/_lib/types';

import {PageFrame, PageSection} from '../PageFrame';
import {StateToken} from '../StateToken';
import {DataTable} from '../tables';
import {TargetRegion} from '../targets';
import {CustomerBotTabs, RunNowButton, SessionTable, didLine, useCustomerBotRefresh} from './parts';

function every(minutes: number): string {
  if (minutes % 60 === 0) return minutes === 60 ? 'every hour' : `every ${minutes / 60} hours`;
  return `every ${minutes} minutes`;
}

function soon(seconds: number | null): string {
  if (seconds === null) return '';
  if (seconds <= 30) return 'any moment';
  const m = Math.round(seconds / 60);
  return m <= 1 ? 'in about a minute' : `in about ${m} minutes`;
}

function Tile({label, value, note}: {label: string; value: string; note?: string}) {
  return (
    <TargetRegion target={{kind: 'customer_bot_figure', label: `${label}: ${value}`, ids: {figure: label}, term: 'acom:CustomerBot'}}>
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

function RunLine({run}: {run: CustomerBotRun}) {
  return (
    <HStack gap={2} vAlign="center" wrap="wrap">
      <StateToken state={run.state} />
      <Text type="supporting" color="secondary">
        {`${run.trigger === 'manual' ? `Run now${run.requested_by ? ` by ${run.requested_by}` : ''}` : 'On schedule'} · started ${dateTime(run.started)}${run.finished ? ` · ended ${dateTime(run.finished)}` : ''} · ${run.workers} shopper${run.workers === 1 ? '' : 's'}`}
      </Text>
    </HStack>
  );
}

export function CustomerBotDashboard({
  status,
  working,
  runs,
  sessions,
}: {
  status: CustomerBotStatus;
  /** The run at work with its sessions, when there is one. */
  working: CustomerBotRunDetail | null;
  runs: CustomerBotRun[];
  sessions: CustomerBotSession[];
}) {
  const isWorking = status.working !== null;
  useCustomerBotRefresh(isWorking || (status.next_run_in_seconds ?? 999) < 40);
  const s = status.settings;
  const t = status.totals;
  return (
    <PageFrame
      title="Ailism Customer Bot"
      meta={
        <>
          <StateToken state={s.mode === 'on' ? 'active' : 'off'} size="md" />
          <Text type="supporting" color="secondary">
            {s.mode === 'on' ? `${s.workers} shopper${s.workers === 1 ? '' : 's'} ${every(s.every_minutes)}` : 'Off: only Run now'}
            {s.mode === 'on' && !isWorking && status.next_run_at ? ` · next ${soon(status.next_run_in_seconds)}` : ''}
          </Text>
        </>
      }
      description="Shoppers that keep this demo store busy. On its schedule each one signs in as a member, or signs up as a new one, and shops on the storefront through the assistant: it searches, asks, adds to the cart, places orders and reviews what was delivered. A member without a persona gets one first. Every step is recorded."
      actions={
        <>
          <RunNowButton />
          <Button label="Settings" variant="secondary" href="/bo/customer-bot/settings" />
        </>
      }
      toolbar={<CustomerBotTabs value="dashboard" />}>
      <Grid columns={{minWidth: 160}} gap={3}>
        <Tile label="Sessions" value={count(t.sessions)} note={t.failed_sessions ? `${count(t.failed_sessions)} stopped early` : `in ${count(t.runs)} run${t.runs === 1 ? '' : 's'}`} />
        <Tile label="Orders placed" value={count(t.orders)} note={`${money(t.spent)} spent`} />
        <Tile label="Reviews written" value={count(t.reviews)} note="for delivered items" />
        <Tile label="Chats" value={count(t.chat_turns)} note="turns with the assistant" />
        <Tile label="Personas" value={count(t.personas)} note="members with a persona" />
        <Tile label="New members" value={count(t.accounts_created)} note="signed up by the bot" />
      </Grid>

      {working ? (
        <PageSection title="Shopping now" count={working.sessions.length}>
          <Card>
            <VStack gap={3}>
              <HStack gap={2} vAlign="center">
                <RunLine run={{...working, sessions: working.sessions.length}} />
                <Spinner size="sm" />
              </HStack>
              <SessionTable sessions={working.sessions} emptyTitle="Opening sessions" />
            </VStack>
          </Card>
        </PageSection>
      ) : status.working ? (
        <PageSection title="Shopping now">
          <Card variant="muted">
            <HStack gap={2} vAlign="center">
              <RunLine run={status.working} />
              <Spinner size="sm" />
            </HStack>
          </Card>
        </PageSection>
      ) : null}

      {status.last_run?.state === 'failed' ? (
        <Banner status="warning" title="The last run stopped early" description={status.last_run.error ?? 'Every session failed. Open a session to see the step that failed.'} />
      ) : null}

      <PageSection title="Recent sessions" actions={<Button label="All sessions" size="sm" variant="ghost" href="/bo/customer-bot/sessions" />}>
        <SessionTable sessions={sessions} />
      </PageSection>

      <PageSection title="Recent runs">
        <DataTable
          rows={runs}
          rowKey={r => r.id}
          target={r => ({kind: 'customer_bot_run', label: `Run ${dateTime(r.started)} · ${r.state}`, ids: {customer_bot_run_id: r.id}, term: 'acom:CustomerBotRun'})}
          empty={{title: 'No runs yet', description: 'Press Run now, or wait for the schedule.'}}
          columns={[
            {header: 'Started', width: '18%', cell: r => dateTime(r.started)},
            {header: 'State', cell: r => <StateToken state={r.state} />},
            {header: 'Started by', cell: r => (r.trigger === 'manual' ? 'Run now' : words(r.trigger))},
            {header: 'Shoppers', numeric: true, cell: r => count(r.sessions)},
            {header: 'What they did', width: '40%', cell: r => <Text type="supporting">{r.state === 'done' || r.state === 'failed' ? didLine(r.summary) : 'Shopping…'}</Text>},
          ]}
        />
      </PageSection>
    </PageFrame>
  );
}
