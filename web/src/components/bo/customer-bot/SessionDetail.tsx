'use client';

// One Ailism Customer Bot session (get_customer_bot_session): the member it shopped as, its
// persona, what it came for and did, and every step in order. A chat step shows what the shopper
// wrote and what the assistant answered and showed; a decision shows why; any step opens to the
// request it sent and what came back. Refreshes while the session is still shopping.

import {Blockquote} from '@astryxdesign/core/Blockquote';
import {Card} from '@astryxdesign/core/Card';
import {CodeBlock} from '@astryxdesign/core/CodeBlock';
import {Collapsible} from '@astryxdesign/core/Collapsible';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {count, dateTime, words} from '@/app/bo/_lib/format';
import type {CustomerBotSessionDetail, CustomerBotStep, ShopperPersonaBody} from '@/app/bo/_lib/types';

import {PageFrame, PageSection} from '../PageFrame';
import {StateToken} from '../StateToken';
import {TargetBlock} from '../targets';
import {ACCOUNT_LABEL, CustomerBotTabs, MemberLink, PersonaFacts, didLine, spent, useCustomerBotRefresh} from './parts';

const KIND: Record<string, string> = {
  sign_up: 'Signed up',
  sign_in: 'Signed in',
  persona: 'Persona',
  visit: 'Page',
  read: 'Read',
  think: 'Decided',
  chat: 'Chat',
  press: 'Pressed',
  review: 'Review',
  sign_out: 'Signed out',
};

const STATUS: Record<string, {state: string; label: string}> = {
  ok: {state: 'passed', label: 'Done'},
  failed: {state: 'failed', label: 'Failed'},
  skipped: {state: 'paused', label: 'Plain plan'},
};

function str(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

function list(v: unknown): Record<string, unknown>[] {
  return Array.isArray(v) ? (v.filter(x => x && typeof x === 'object') as Record<string, unknown>[]) : [];
}

function json(v: unknown): string {
  return JSON.stringify(v, null, 2) ?? '';
}

/** What a block the assistant showed held, in a line or two. */
function BlockLine({b}: {b: Record<string, unknown>}) {
  const ui = str(b.ui);
  const items = list(b.products).map(p => `${str(p.name)}${str(p.price) ? ` ${str(p.price)}` : ''}`);
  const product = b.product && typeof b.product === 'object' ? (b.product as Record<string, unknown>) : null;
  const lines = list(b.lines).map(l => [str(l.name), str(l.color), str(l.size), l.quantity ? `×${String(l.quantity)}` : ''].filter(Boolean).join(' '));
  const failed = Array.isArray(b.failed_checks) ? b.failed_checks.filter(Boolean).map(String) : [];
  const text =
    items.length > 0
      ? items.join(' · ')
      : product
        ? `${str(product.name)} ${str(product.price)}`
        : lines.length > 0
          ? `${lines.join(' · ')}${str(b.total) ? ` · total ${str(b.total)}` : ''}`
          : str(b.total)
            ? `Total ${str(b.total)}${failed.length > 0 ? ` · failed: ${failed.join('; ')}` : ''}`
            : Array.isArray(b.rows)
              ? b.rows.filter(Boolean).map(String).join(' · ')
              : '';
  return (
    <HStack gap={2} vAlign="start">
      <Token label={words(ui)} size="sm" />
      <Text type="supporting">{[str(b.title), text].filter(Boolean).join(' — ') || '—'}</Text>
    </HStack>
  );
}

function ChatDetail({step}: {step: CustomerBotStep}) {
  const d = step.detail;
  const said = str((step.request?.body as Record<string, unknown> | undefined)?.message);
  const blocks = list(d.blocks);
  const tools = Array.isArray(d.tools) ? d.tools.map(String) : [];
  const memories = Array.isArray(d.memories) ? d.memories.map(String) : [];
  return (
    <VStack gap={2}>
      {said ? (
        <Text>
          <Text weight="semibold">Shopper: </Text>
          {said}
        </Text>
      ) : null}
      {str(d.reply) ? <Blockquote>{str(d.reply)}</Blockquote> : null}
      {str(d.error) ? (
        <Text type="supporting" color="secondary">
          {`The turn failed: ${str(d.error)}`}
        </Text>
      ) : null}
      {blocks.length > 0 ? (
        <VStack gap={1}>
          {blocks.map((b, i) => (
            <BlockLine key={i} b={b} />
          ))}
        </VStack>
      ) : null}
      {tools.length > 0 || memories.length > 0 ? (
        <HStack gap={1} wrap="wrap" vAlign="center">
          {tools.map(t => (
            <Token key={t} label={t} size="sm" />
          ))}
          {memories.map(m => (
            <Token key={m} label={`Remembered: ${m}`} size="sm" color="blue" />
          ))}
        </HStack>
      ) : null}
      {str(d.conversation_id) ? (
        <Text type="supporting" color="secondary">
          {`Conversation ${str(d.conversation_id)}`}
        </Text>
      ) : null}
    </VStack>
  );
}

function ThinkDetail({step}: {step: CustomerBotStep}) {
  const d = step.detail;
  return (
    <VStack gap={1}>
      {str(d.message) ? <Text>{`“${str(d.message)}”`}</Text> : null}
      {str(d.reason) ? (
        <Text type="supporting" color="secondary">
          {`Why: ${str(d.reason)}`}
        </Text>
      ) : null}
      {str(d.fallback) ? (
        <Text type="supporting" color="secondary">
          {`The shopper model did not answer (${str(d.fallback)}), so the plain plan chose this.`}
        </Text>
      ) : null}
    </VStack>
  );
}

function StepRow({step}: {step: CustomerBotStep}) {
  const st = STATUS[step.status] ?? {state: step.status, label: words(step.status)};
  const persona = step.kind === 'persona' && step.detail.persona && typeof step.detail.persona === 'object' ? (step.detail.persona as ShopperPersonaBody) : null;
  const hasRequest = step.request !== null || step.response !== null;
  return (
    <TargetBlock
      target={{
        kind: 'customer_bot_step',
        label: `Step ${step.position}: ${step.title}`,
        ids: {position: String(step.position)},
        term: 'acom:CustomerBotStep',
      }}>
      <Card variant={step.status === 'failed' ? 'default' : 'muted'}>
        <VStack gap={2}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Text type="supporting" color="secondary" hasTabularNumbers>
              {String(step.position).padStart(2, '0')}
            </Text>
            <Token label={KIND[step.kind] ?? words(step.kind)} size="sm" />
            <StackItem size="fill">
              <Text weight="semibold">{step.title}</Text>
            </StackItem>
            <StateToken state={st.state} label={st.label} />
            <Text type="supporting" color="secondary" hasTabularNumbers>
              {`${dateTime(step.recorded)}${step.ms ? ` · ${step.ms >= 1000 ? `${(step.ms / 1000).toFixed(1)} s` : `${step.ms} ms`}` : ''}`}
            </Text>
          </HStack>
          {step.kind === 'chat' ? <ChatDetail step={step} /> : null}
          {step.kind === 'think' ? <ThinkDetail step={step} /> : null}
          {persona ? <PersonaFacts p={persona} /> : null}
          {hasRequest || Object.keys(step.detail).length > 0 ? (
            <Collapsible defaultIsOpen={false} trigger={<Text type="supporting">What it sent and got back</Text>}>
              <CodeBlock code={json({request: step.request, response: step.response, detail: step.detail})} language="json" maxHeight={360} isWrapped />
            </Collapsible>
          ) : null}
        </VStack>
      </Card>
    </TargetBlock>
  );
}

export function SessionDetail({session}: {session: CustomerBotSessionDetail}) {
  const isRunning = session.state === 'running';
  useCustomerBotRefresh(isRunning, 4);
  const o = session.outcome;
  const orders = o.order_ids ?? [];
  return (
    <PageFrame
      title={session.customer_id ? `Session as ${session.customer_id}` : 'Session (signing up)'}
      back={{href: '/bo/customer-bot/sessions', label: 'Sessions'}}
      meta={
        <>
          <StateToken state={session.state} size="md" />
          <Text type="supporting" color="secondary">
            {`Started ${dateTime(session.started)}${session.finished ? ` · ended ${dateTime(session.finished)}` : ''} · ${count(session.steps.length)} steps`}
          </Text>
          {isRunning ? <Spinner size="sm" /> : null}
        </>
      }
      toolbar={<CustomerBotTabs value="sessions" />}>
      <Grid columns={{minWidth: 280}} gap={4}>
        <Card>
          <VStack gap={2}>
            <Text type="label" weight="semibold">
              Shopper
            </Text>
            <HStack gap={2} vAlign="center" wrap="wrap">
              <MemberLink id={session.customer_id} />
              <Token label={ACCOUNT_LABEL[session.account] ?? words(session.account)} size="sm" />
              {session.persona_built ? <Token label="Persona made in this session" size="sm" color="blue" /> : null}
            </HStack>
            {session.persona ? <Text>{session.persona.description}</Text> : <Text color="secondary">No persona yet.</Text>}
            {session.persona ? <PersonaFacts p={session.persona.persona} /> : null}
          </VStack>
        </Card>
        <Card>
          <VStack gap={2}>
            <Text type="label" weight="semibold">
              This visit
            </Text>
            <Text>{session.goal ?? (isRunning ? 'Deciding what to shop for…' : 'No goal recorded.')}</Text>
            <Text type="supporting" color="secondary">
              {isRunning ? 'Still shopping.' : didLine(o)}
            </Text>
            {orders.length > 0 ? (
              <VStack gap={1}>
                {orders.map(x => (
                  <Text key={x.order_id} type="supporting">
                    {'Order '}
                    <Link href={`/bo/orders/${encodeURIComponent(x.order_id)}`}>{x.order_id}</Link>
                    {` · ${x.total}`}
                  </Text>
                ))}
                <Text type="supporting" color="secondary">{`Spent ${spent(o.spent)}`}</Text>
              </VStack>
            ) : null}
            {o.fallbacks ? (
              <Text type="supporting" color="secondary">
                {`${count(o.fallbacks)} decision${o.fallbacks === 1 ? '' : 's'} came from the plain plan because the shopper model did not answer.`}
              </Text>
            ) : null}
            {session.error ? <Text color="secondary">{`Stopped: ${session.error}`}</Text> : null}
          </VStack>
        </Card>
      </Grid>

      <PageSection title="Every step" count={session.steps.length} description="In the order they happened. Requests went through the storefront's own front door with the member's session, like a browser.">
        <VStack gap={2}>
          {session.steps.map(s => (
            <StepRow key={s.position} step={s} />
          ))}
          {isRunning ? (
            <HStack gap={2} vAlign="center">
              <Spinner size="sm" />
              <Text type="supporting" color="secondary">
                Shopping…
              </Text>
            </HStack>
          ) : null}
        </VStack>
      </PageSection>
    </PageFrame>
  );
}
