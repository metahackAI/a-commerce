'use client';

// The store's time in the console header (docs/api/platform.md#데모-시계): the time every request
// of the store runs on, which in demo mode is the demo clock. A session with `admin:demo` can
// move it to the scenes' times or to a chosen time. Moving first runs the scheduled jobs that come
// due in between (watchers refresh the tasks, promotions start and end), then the pages reload
// their data. Moving time is a person's decision, so it goes through PersonButton only.

import {useCallback, useEffect, useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {DateTimeInput, type ISODateTimeString} from '@astryxdesign/core/DateTimeInput';
import {StatusDot} from '@astryxdesign/core/StatusDot';

import {PersonButton} from '@/components/PersonButton';
import {api} from '@/lib/api';
import {dateTime, words} from '@/app/bo/_lib/format';

import {useBoSession} from './BoSession';
import {TargetField, TargetInline, TargetRegion} from './targets';

/** `GET /v1/platform/demo-clock`. */
export interface StoreClockState {
  now: string;
  is_running: boolean;
  demo_mode: boolean;
}

/** One job a move ran (`ran[]` of `PUT /v1/platform/demo-clock`). */
export interface RanJob {
  job_id: string;
  kind: string;
  run_at: string;
  state: 'done' | 'retrying' | string;
  attempts: number;
  error?: string;
  tasks?: {task_id: string; state: string; created: boolean; change_set_id?: string}[];
  next_run_at?: string;
}

/** `PUT /v1/platform/demo-clock`. */
export interface ClockMoved extends StoreClockState {
  previous: string;
  ran: RanJob[];
}

/** The scenes of docs/scenarios.md, in store time. */
const SCENES: {now: string; label: string; note: string}[] = [
  {now: '2026-11-06T11:00', label: 'Fri Nov 6, 11:00', note: 'Friday operations: the VIP sale and the returns waiting over $200'},
  {now: '2026-11-07T10:00', label: 'Sat Nov 7, 10:00', note: 'Saturday shopping, with the weekend sale live'},
  {now: '2026-11-09T09:00', label: 'Mon Nov 9, 09:00', note: 'After the weekend: what ran, what is left to undo'},
];

const PATH = '/v1/platform/demo-clock';
const MOVE_SCOPE = 'admin:demo';

/** What a move did, in one line. */
function movedLine(m: ClockMoved): string {
  const n = m.ran.length;
  if (n === 0) return `Now ${dateTime(m.now)}. Nothing was due.`;
  const failed = m.ran.filter(j => j.state !== 'done').length;
  const touched = m.ran.reduce((sum, j) => sum + (j.tasks?.length ?? 0), 0);
  const parts = [`${n} ${n === 1 ? 'job' : 'jobs'} ran`];
  if (touched > 0) parts.push(`${touched} ${touched === 1 ? 'task' : 'tasks'} updated`);
  if (failed > 0) parts.push(`${failed} will retry`);
  return `Now ${dateTime(m.now)}. ${parts.join(', ')}.`;
}

function SceneRow({scene, onMoved}: {scene: (typeof SCENES)[number]; onMoved: (m: ClockMoved) => void}) {
  return (
    <TargetRegion target={{kind: 'clock_scene', label: `Scene time ${scene.label}: ${scene.note}`, ids: {scene: scene.now}}}>
      <HStack gap={3} vAlign="center">
        <StackItem size="fill">
          <VStack gap={0}>
            <Text type="label" weight="semibold">
              {scene.label}
            </Text>
            <Text type="supporting" color="secondary">
              {scene.note}
            </Text>
          </VStack>
        </StackItem>
        <PersonButton
          variant="secondary"
          action={{
            label: `Go to ${scene.label}`,
            method: 'PUT',
            path: PATH,
            body: {now: scene.now, is_running: false},
            confirm: `Move the store's time to ${scene.label}? Scheduled jobs that come due before then run now, once each. Moving back does not undo what ran.`,
          }}
          onDone={r => onMoved(r as ClockMoved)}
        />
      </HStack>
    </TargetRegion>
  );
}

function MoveDialog({clock, onClose, onMoved}: {clock: StoreClockState; onClose: () => void; onMoved: (m: ClockMoved) => void}) {
  const [chosen, setChosen] = useState<ISODateTimeString | undefined>(undefined);
  return (
    <Dialog isOpen onOpenChange={open => !open && onClose()} width={560} purpose="form">
      <DialogHeader
        title="Move the store's time"
        subtitle={`It is ${dateTime(clock.now)} in the store. Jobs that come due on the way run first: watchers refresh the tasks, promotions start and end.`}
        onOpenChange={open => !open && onClose()}
      />
      <VStack gap={4} padding={4}>
        {SCENES.map(s => (
          <SceneRow key={s.now} scene={s} onMoved={onMoved} />
        ))}
        <TargetField target={{kind: 'clock_input', label: `Chosen store time: ${chosen ? dateTime(chosen) : 'empty'}`, ids: {clock_input: 'demo_clock'}}}>
          <DateTimeInput label="Another time" description="Store time" value={chosen} onChange={setChosen} hourFormat="24h" />
          <PersonButton
            variant="secondary"
            isDisabled={!chosen}
            action={{
              label: 'Go to this time',
              method: 'PUT',
              path: PATH,
              body: {now: chosen ?? '', is_running: false},
              confirm: `Move the store's time to ${chosen ? dateTime(chosen) : 'the chosen time'}? Scheduled jobs that come due before then run now, once each.`,
            }}
            onDone={r => onMoved(r as ClockMoved)}
          />
        </TargetField>
      </VStack>
    </Dialog>
  );
}

/** The store time, and for the demo administrator a way to move it. */
export function StoreClock() {
  const session = useBoSession();
  const router = useRouter();
  const [clock, setClock] = useState<StoreClockState | null>(null);
  const [isMoving, setIsMoving] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setClock(await api<StoreClockState>(PATH));
    } catch {
      setClock(null);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // A running clock moves with real time: read it again now and then.
  useEffect(() => {
    if (!clock?.is_running) return;
    const id = window.setInterval(() => void load(), 60_000);
    return () => window.clearInterval(id);
  }, [clock?.is_running, load]);

  if (!clock) return null;
  const canMove = clock.demo_mode && (session?.scopes.includes(MOVE_SCOPE) ?? false);
  const state = clock.demo_mode ? (clock.is_running ? 'demo clock, running' : 'demo clock, paused') : 'real time';

  return (
    <VStack gap={1}>
      <TargetInline target={{kind: 'store_clock', label: `Store time ${dateTime(clock.now)} (${state})`, ids: {store_clock: 'store'}, term: 'acom:demoClock'}}>
        <VStack gap={0}>
          <Text type="supporting" color="secondary">
            Store time
          </Text>
          <HStack gap={1} vAlign="center">
            {clock.demo_mode ? <StatusDot variant={clock.is_running ? 'accent' : 'neutral'} label={words(state)} /> : null}
            <Text type="label">{dateTime(clock.now)}</Text>
          </HStack>
        </VStack>
      </TargetInline>
      {canMove ? <Button label="Move time" size="sm" variant="ghost" onClick={() => setIsMoving(true)} /> : null}
      {notice ? (
        <Text type="supporting" color="secondary">
          {notice}
        </Text>
      ) : null}
      {isMoving ? (
        <MoveDialog
          clock={clock}
          onClose={() => setIsMoving(false)}
          onMoved={m => {
            setIsMoving(false);
            setClock({now: m.now, is_running: m.is_running, demo_mode: m.demo_mode});
            setNotice(movedLine(m));
            router.refresh();
          }}
        />
      ) : null}
    </VStack>
  );
}
