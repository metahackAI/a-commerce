'use client';

// Daily revenue as a line, with the comparison basis as a dashed line and the day being
// explained marked. Astryx 0.7 has no chart component in core (charts are canary-only), so this
// is plain SVG drawn with the Astryx data-visualization tokens. Every value plotted comes from
// `get_sales_trend` as sent; the only arithmetic here is placing points on the canvas.

import {type Ref} from 'react';
import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {money} from '@/app/bo/_lib/format';
import type {Money} from '@/app/bo/_lib/types';
import {useAgentDock} from '@/components/bo/AgentDock';
import {TargetRegion} from '@/components/bo/targets';

export interface SalesPoint {
  date: string;
  revenue: Money | null;
}

const W = 720;
const H = 240;
const PAD = {left: 56, right: 20, top: 20, bottom: 30};
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** `$22.9K`: a figure shortened for an axis or a label. */
function compact(m: Money): string {
  try {
    return new Intl.NumberFormat('en-US', {style: 'currency', currency: m.currency, notation: 'compact', maximumFractionDigits: 1}).format(m.value);
  } catch {
    return String(m.value);
  }
}

/** `2026-11-05` → `Nov 5`, and its weekday. */
function day(s: string): {month: string; day: number; weekday: string} {
  const [y, mo, d] = s.split('-').map(Number);
  const weekday = DAYS[new Date(Date.UTC(y, mo - 1, d)).getUTCDay()];
  return {month: MONTHS[mo - 1], day: d, weekday};
}

/** Axis lines at round values that cover `lo`–`hi`. */
function ticks(lo: number, hi: number): number[] {
  const span = Math.max(hi - lo, 1);
  const raw = span / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map(f => f * mag).find(s => s >= raw) ?? raw;
  const out: number[] = [];
  for (let v = Math.floor(lo / step) * step; v <= hi + step / 2; v += step) out.push(v);
  return out;
}

function Point({
  point,
  x,
  y,
  marked,
  blockId,
}: {
  point: SalesPoint;
  x: number;
  y: number;
  marked: boolean;
  blockId?: string;
}) {
  const dock = useAgentDock();
  const d = day(point.date);
  const label = `Revenue on ${d.weekday} ${d.month} ${d.day}: ${money(point.revenue)}`;
  const t = useAiTarget({kind: 'series_point', label, ids: {date: point.date}, blockId, term: 'schema:Observation'});
  return (
    <g
      ref={t.ref as unknown as Ref<SVGGElement>}
      {...t.props}
      role="button"
      tabIndex={0}
      aria-label={`${label}. Ask the agent about this day.`}
      style={{cursor: 'pointer'}}
      onClick={() => {
        t.ask();
        dock.open();
      }}
      onKeyDown={e => {
        if (e.key === 'Enter' || e.key === ' ') {
          t.ask();
          dock.open();
        }
      }}>
      <title>{label}</title>
      <circle cx={x} cy={y} r={10} fill="transparent" />
      <circle
        cx={x}
        cy={y}
        r={marked ? 5 : 3}
        fill={marked ? 'var(--color-data-categorical-blue)' : 'var(--color-background-primary, #fff)'}
        stroke="var(--color-data-categorical-blue)"
        strokeWidth={2}
      />
    </g>
  );
}

export function SalesChart({
  points,
  baseline,
  baselineLabel,
  marked,
  blockId,
  title = 'Daily revenue',
}: {
  points: SalesPoint[];
  baseline?: Money | null;
  baselineLabel?: string;
  /** The day being explained. */
  marked?: string;
  blockId?: string;
  title?: string;
}) {
  const shown = points.filter((p): p is {date: string; revenue: Money} => !!p.revenue && typeof p.revenue.value === 'number');
  if (shown.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No daily revenue to show.
      </Text>
    );
  }
  const values = shown.map(p => p.revenue.value);
  if (baseline) values.push(baseline.value);
  const axis = ticks(Math.min(...values), Math.max(...values));
  const lo = axis[0];
  const hi = axis[axis.length - 1];
  const x = (i: number) => PAD.left + (shown.length === 1 ? 0 : (i * (W - PAD.left - PAD.right)) / (shown.length - 1));
  const y = (v: number) => PAD.top + ((hi - v) * (H - PAD.top - PAD.bottom)) / Math.max(hi - lo, 1);
  const currency = shown[0].revenue.currency;
  const line = shown.map((p, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(p.revenue.value).toFixed(1)}`).join(' ');
  const markedIndex = shown.findIndex(p => p.date === marked);
  const first = day(shown[0].date);
  const range = `${first.month} ${first.day} – ${day(shown[shown.length - 1].date).month} ${day(shown[shown.length - 1].date).day}`;

  return (
    <TargetRegion target={{kind: 'chart', label: `${title}, ${range}`, ids: {chart: `${shown[0].date}_${shown[shown.length - 1].date}`}, blockId}}>
      <VStack gap={1}>
        <Text type="supporting" color="secondary">
          {title}, {range}
        </Text>
        <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label={`${title}, ${range}`} style={{maxWidth: W, overflow: 'visible'}}>
          {axis.map(v => (
            <g key={v}>
              <line x1={PAD.left} x2={W - PAD.right} y1={y(v)} y2={y(v)} stroke="var(--color-data-neutral)" strokeOpacity={0.25} />
              <text x={PAD.left - 8} y={y(v) + 4} textAnchor="end" fontSize={11} fill="var(--color-data-neutral)">
                {compact({value: v, currency})}
              </text>
            </g>
          ))}
          {shown.map((p, i) => {
            const d = day(p.date);
            const show = i === 0 || i === shown.length - 1 || i % 2 === 0;
            return show ? (
              <text key={p.date} x={x(i)} y={H - 8} textAnchor="middle" fontSize={11} fill="var(--color-data-neutral)">
                {i === 0 || d.day === 1 ? `${d.month} ${d.day}` : d.day}
              </text>
            ) : null;
          })}
          {baseline ? (
            <g>
              <line
                x1={PAD.left}
                x2={W - PAD.right}
                y1={y(baseline.value)}
                y2={y(baseline.value)}
                stroke="var(--color-data-neutral)"
                strokeDasharray="4 4"
              />
              <text x={W - PAD.right} y={y(baseline.value) - 6} textAnchor="end" fontSize={11} fill="var(--color-data-neutral)">
                {baselineLabel ? `${baselineLabel} ${compact(baseline)}` : compact(baseline)}
              </text>
            </g>
          ) : null}
          <path d={line} fill="none" stroke="var(--color-data-categorical-blue)" strokeWidth={2} strokeLinejoin="round" />
          {shown.map((p, i) => (
            <Point key={p.date} point={p} x={x(i)} y={y(p.revenue.value)} marked={i === markedIndex} blockId={blockId} />
          ))}
          {markedIndex >= 0 ? (
            <text x={x(markedIndex)} y={y(shown[markedIndex].revenue.value) + 20} textAnchor="end" fontSize={11} fontWeight={600} fill="currentColor">
              {`${day(shown[markedIndex].date).weekday} ${day(shown[markedIndex].date).month} ${day(shown[markedIndex].date).day} · ${compact(shown[markedIndex].revenue)}`}
            </text>
          ) : null}
        </svg>
      </VStack>
    </TargetRegion>
  );
}
