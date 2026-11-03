'use client';

// Ailism Bot's settings (PUT /v1/platform/bot/settings, person-only): how far it goes, how often
// it runs, for which operator its drafts are made, which model analyses, the areas it watches,
// how many proposals it may open, and the directives it always follows. Saved into the store's
// policy override, so every reader sees the same values.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {SegmentedControl, SegmentedControlItem} from '@astryxdesign/core/SegmentedControl';
import {Selector} from '@astryxdesign/core/Selector';
import {Switch} from '@astryxdesign/core/Switch';
import {Text} from '@astryxdesign/core/Text';
import {TextArea} from '@astryxdesign/core/TextArea';
import {TextInput} from '@astryxdesign/core/TextInput';

import {PersonButton} from '@/components/PersonButton';
import {BOT_AREAS, type BotMode, type BotSettings, type BotStatus} from '@/app/bo/_lib/types';

import {useBoSession} from '../BoSession';
import {PageFrame, PageSection} from '../PageFrame';
import {TargetField} from '../targets';
import {AREA_LABEL, BotTabs, canUseBot} from './parts';

const AREA_NOTE: Record<string, string> = {
  pricing: 'Prices under cost or the price floor, discounts that go too deep.',
  inventory: 'Sizes out of stock or about to be, reorders.',
  promotions: 'Promotions running or ending, stock that does not sell, yesterday’s sales.',
  merchandising: 'Main page slots and collections.',
  catalog_content: 'New products without a photo or a description.',
  returns: 'Return requests that wait for a decision.',
  questions_reviews: 'Product questions without an answer, low reviews.',
  community: 'Reported posts on the community boards.',
};

const EVERY = [30, 60, 120, 240, 360, 720, 1440];

function everyLabel(m: number): string {
  if (m < 60) return `Every ${m} minutes`;
  if (m === 60) return 'Every hour';
  if (m === 1440) return 'Once a day';
  return `Every ${m / 60} hours`;
}

export function BotSettingsForm({status}: {status: BotStatus}) {
  const router = useRouter();
  const session = useBoSession();
  const s = status.settings;
  const [mode, setMode] = useState<BotMode>(s.mode);
  const [every, setEvery] = useState(s.every_minutes);
  const [actsAs, setActsAs] = useState(s.acts_as ?? '');
  const [model, setModel] = useState(s.model);
  const [areas, setAreas] = useState<Record<string, boolean>>(s.areas);
  const [maxPerCycle, setMaxPerCycle] = useState<number | null>(s.max_proposals_per_cycle);
  const [maxOpen, setMaxOpen] = useState<number | null>(s.max_open_decisions);
  const [cooldown, setCooldown] = useState<number | null>(s.cooldown_days);
  const [directives, setDirectives] = useState(s.directives.join('\n'));
  const may = canUseBot(session?.scopes);

  const body: Partial<BotSettings> = {
    mode,
    every_minutes: every,
    acts_as: actsAs.trim(),
    model: model.trim(),
    areas,
    max_proposals_per_cycle: maxPerCycle ?? s.max_proposals_per_cycle,
    max_open_decisions: maxOpen ?? s.max_open_decisions,
    cooldown_days: cooldown ?? s.cooldown_days,
    directives: directives
      .split('\n')
      .map(d => d.trim())
      .filter(Boolean),
  };
  const field = (label: string, value: string) => ({kind: 'input', label: `${label}: ${value}`, ids: {form: 'bot_settings', field: label}, term: 'acom:Bot'});

  return (
    <PageFrame
      title="Settings"
      back={{href: '/bo/bot', label: 'Ailism Bot'}}
      description="How Ailism Bot runs. Whatever the settings, it only proposes: every change waits for a person's approval (or a standing approval the store already gave)."
      actions={
        may ? (
          <PersonButton
            action={{
              label: 'Save settings',
              method: 'PUT',
              path: '/v1/platform/bot/settings',
              body,
              confirm: `Save these settings? The bot runs in ${mode} mode, ${everyLabel(every).toLowerCase()}, in ${Object.values(areas).filter(Boolean).length} areas.`,
            }}
            onDone={() => router.refresh()}
          />
        ) : undefined
      }
      toolbar={<BotTabs value="settings" />}>
      {!may ? (
        <Text type="supporting" color="secondary">
          Your account can read these settings but not change them.
        </Text>
      ) : null}

      <PageSection title="How it runs">
        <VStack gap={4}>
          <TargetField target={field('Mode', mode)}>
            <SegmentedControl label="Mode" value={mode} onChange={v => setMode(v as BotMode)}>
              <SegmentedControlItem value="off" label="Off" />
              <SegmentedControlItem value="observe" label="Observe" />
              <SegmentedControlItem value="propose" label="Propose" />
            </SegmentedControl>
          </TargetField>
          <Text type="supporting" color="secondary">
            {mode === 'off'
              ? 'No cycle runs, on schedule or by Run now.'
              : mode === 'observe'
                ? 'Cycles run and write a report; nothing is opened for you.'
                : 'Cycles open what they propose as tasks; the operations agent drafts each as a change set for you to decide.'}
          </Text>
          <HStack gap={4} wrap="wrap" vAlign="end">
            <TargetField target={field('Schedule', everyLabel(every))}>
              <Selector
                label="Schedule"
                width={200}
                options={EVERY.map(m => ({value: String(m), label: everyLabel(m)}))}
                value={String(every)}
                onChange={(v: string | null) => v && setEvery(Number(v))}
              />
            </TargetField>
            <TargetField target={field('Drafts for', actsAs || status.acts_as || 'nobody')}>
              <TextInput
                label="Drafts made for"
                description={status.acts_as && !actsAs ? `Empty: the task pickup's operator (${status.acts_as}).` : 'An active operator id, like adm_sj.'}
                value={actsAs}
                onChange={setActsAs}
                placeholder={status.acts_as ?? 'adm_…'}
                width={220}
              />
            </TargetField>
            <TargetField target={field('Model', model)}>
              <TextInput label="Analysis model" description="Its alias in agents/models.yml." value={model} onChange={setModel} width={180} />
            </TargetField>
          </HStack>
        </VStack>
      </PageSection>

      <PageSection title="What it watches" description="An area that is off is neither read nor proposed in.">
        <Grid columns={{minWidth: 300}} gap={3}>
          {BOT_AREAS.map(a => (
            <Card key={a} variant="muted">
              <TargetField target={field(AREA_LABEL[a], areas[a] ? 'on' : 'off')}>
                <Switch label={AREA_LABEL[a]} description={AREA_NOTE[a]} value={areas[a] ?? false} onChange={on => setAreas({...areas, [a]: on})} />
              </TargetField>
            </Card>
          ))}
        </Grid>
      </PageSection>

      <PageSection title="How much it proposes">
        <HStack gap={4} wrap="wrap">
          <NumberInput label="Proposals per cycle" value={maxPerCycle} onChange={v => setMaxPerCycle(Number.isFinite(v) ? v : null)} min={1} max={10} isIntegerOnly width={180} />
          <NumberInput
            label="Waiting at most"
            description="No new proposal while this many wait."
            value={maxOpen}
            onChange={v => setMaxOpen(Number.isFinite(v) ? v : null)}
            min={1}
            max={50}
            isIntegerOnly
            width={180}
          />
          <NumberInput
            label="Days before asking again"
            description="After you turn a proposal down."
            value={cooldown}
            onChange={v => setCooldown(Number.isFinite(v) ? v : null)}
            min={0}
            max={60}
            isIntegerOnly
            width={200}
          />
        </HStack>
      </PageSection>

      <PageSection title="Directives" description="Rules the bot always follows, one per line (up to 20). Open directive feedback counts too.">
        <TargetField target={field('Directives', directives.split('\n').filter(Boolean).join('; ') || 'none')}>
          <TextArea label="Directives" isLabelHidden value={directives} onChange={setDirectives} rows={4} placeholder="Never discount new arrivals." />
        </TargetField>
      </PageSection>
    </PageFrame>
  );
}
