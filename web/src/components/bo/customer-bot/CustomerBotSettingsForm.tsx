'use client';

// Ailism Customer Bot's settings (PUT /v1/platform/customer-bot/settings, person-only): on or
// off, how often and how many shoppers at once, how many sign up as new members, how much each
// does, and the model that plays them. Saved into the store's policy override.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {SegmentedControl, SegmentedControlItem} from '@astryxdesign/core/SegmentedControl';
import {Selector} from '@astryxdesign/core/Selector';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';

import {PersonButton} from '@/components/PersonButton';
import type {CustomerBotMode, CustomerBotSettings, CustomerBotStatus} from '@/app/bo/_lib/types';

import {useBoSession} from '../BoSession';
import {PageFrame, PageSection} from '../PageFrame';
import {TargetField} from '../targets';
import {CustomerBotTabs, canUseCustomerBot} from './parts';

const EVERY = [5, 10, 15, 30, 60, 120, 240, 1440];

function everyLabel(m: number): string {
  if (m < 60) return `Every ${m} minutes`;
  if (m === 60) return 'Every hour';
  if (m === 1440) return 'Once a day';
  return `Every ${m / 60} hours`;
}

export function CustomerBotSettingsForm({status}: {status: CustomerBotStatus}) {
  const router = useRouter();
  const session = useBoSession();
  const s = status.settings;
  const [mode, setMode] = useState<CustomerBotMode>(s.mode);
  const [every, setEvery] = useState(s.every_minutes);
  const [workers, setWorkers] = useState<number | null>(s.workers);
  const [newShare, setNewShare] = useState<number | null>(s.new_account_percent);
  const [actions, setActions] = useState<number | null>(s.max_actions);
  const [reviews, setReviews] = useState<number | null>(s.reviews_per_session);
  const [model, setModel] = useState(s.model);
  const may = canUseCustomerBot(session?.scopes);
  const body: Partial<CustomerBotSettings> = {
    mode,
    every_minutes: every,
    workers: workers ?? s.workers,
    new_account_percent: newShare ?? s.new_account_percent,
    max_actions: actions ?? s.max_actions,
    reviews_per_session: reviews ?? s.reviews_per_session,
    model: model.trim(),
  };
  const field = (label: string, value: string) => ({kind: 'input', label: `${label}: ${value}`, ids: {form: 'customer_bot_settings', field: label}, term: 'acom:CustomerBot'});
  const options = EVERY.includes(every) ? EVERY : [...EVERY, every].sort((a, b) => a - b);
  return (
    <PageFrame
      title="Settings"
      back={{href: '/bo/customer-bot', label: 'Ailism Customer Bot'}}
      description="How Ailism Customer Bot runs. Its shoppers only do what a member can do on the storefront; their orders and reviews go through the store's own rules."
      actions={
        may ? (
          <PersonButton
            action={{
              label: 'Save settings',
              method: 'PUT',
              path: '/v1/platform/customer-bot/settings',
              body,
              confirm: `Save these settings? ${mode === 'on' ? `${body.workers} shoppers ${everyLabel(every).toLowerCase()}.` : 'The bot runs only when you press Run now.'}`,
            }}
            onDone={() => router.refresh()}
          />
        ) : undefined
      }
      toolbar={<CustomerBotTabs value="settings" />}>
      {!may ? (
        <Text type="supporting" color="secondary">
          Your account can read these settings but not change them.
        </Text>
      ) : null}
      <PageSection title="When it runs">
        <VStack gap={4}>
          <TargetField target={field('Mode', mode)}>
            <SegmentedControl label="Mode" value={mode} onChange={v => setMode(v as CustomerBotMode)}>
              <SegmentedControlItem value="off" label="Off" />
              <SegmentedControlItem value="on" label="On" />
            </SegmentedControl>
          </TargetField>
          <Text type="supporting" color="secondary">
            {mode === 'off' ? 'No run on schedule; Run now still works.' : 'A run on schedule, counted in real minutes even while the demo clock stands still.'}
          </Text>
          <HStack gap={4} wrap="wrap" vAlign="end">
            <TargetField target={field('Schedule', everyLabel(every))}>
              <Selector
                label="Schedule"
                width={200}
                options={options.map(m => ({value: String(m), label: everyLabel(m)}))}
                value={String(every)}
                onChange={(v: string | null) => v && setEvery(Number(v))}
              />
            </TargetField>
            <NumberInput label="Shoppers at once" description="Workers in one run." value={workers} onChange={v => setWorkers(Number.isFinite(v) ? v : null)} min={1} max={10} isIntegerOnly width={180} />
            <NumberInput
              label="New members"
              description="Percent of shoppers who sign up first."
              value={newShare}
              onChange={v => setNewShare(Number.isFinite(v) ? v : null)}
              min={0}
              max={100}
              isIntegerOnly
              width={200}
            />
          </HStack>
        </VStack>
      </PageSection>
      <PageSection title="What each shopper does">
        <HStack gap={4} wrap="wrap" vAlign="end">
          <NumberInput label="Actions per visit" description="Chats, orders and reviews." value={actions} onChange={v => setActions(Number.isFinite(v) ? v : null)} min={1} max={20} isIntegerOnly width={180} />
          <NumberInput label="Reviews per visit" description="For delivered items." value={reviews} onChange={v => setReviews(Number.isFinite(v) ? v : null)} min={0} max={5} isIntegerOnly width={180} />
          <TargetField target={field('Model', model)}>
            <TextInput label="Shopper model" description="Its alias in agents/models.yml." value={model} onChange={setModel} width={180} />
          </TargetField>
        </HStack>
        <Text type="supporting" color="secondary">
          {`Never shops as the demo scenes' cast: ${s.never_shop_as.join(', ') || 'nobody'}.`}
        </Text>
      </PageSection>
    </PageFrame>
  );
}
