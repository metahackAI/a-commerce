import {Text} from '@astryxdesign/core/Text';

import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame, PageSection} from '@/components/bo/PageFrame';
import {SalesChart} from '@/components/bo/SalesChart';
import {SalesExplanationView} from '@/components/bo/SalesExplanationView';

import {isoDate, param, read, readPost, type SearchParams} from '../../_lib/server';
import type {Money, Page, SalesExplanation, SalesTrendPoint, StoreClock} from '../../_lib/types';

export const metadata = {title: 'Insights'};

const COMPARE = [
  {value: 'same_weekday_last_4_weeks', label: 'Same weekday, last 4 weeks'},
  {value: 'previous_period', label: 'The day before'},
  {value: 'custom', label: 'Days I choose'},
];
const DATE = /^\d{4}-\d{2}-\d{2}$/;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
/** Days the chart shows, ending on the chosen day (the B3 view: two weeks). */
const CHART_DAYS = 14;

/** A calendar day moved by `days`: only for the filter defaults and the chart's window. */
function shift(day: string, days: number): string {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
}

/** `2026-11-05` → `Thu Nov 5`. */
function label(day: string): string {
  const [y, m, d] = day.split('-').map(Number);
  return `${DAYS[new Date(Date.UTC(y, m - 1, d)).getUTCDay()]} ${MONTHS[m - 1]} ${d}`;
}

// Insights: daily revenue over two weeks, and why the chosen day differs from its basis
// (get_sales_trend, explain_sales_change). The day defaults to yesterday on the store's clock
// (the demo clock in demo mode). Every figure is the tools' own; the page only lays them out.
export default async function InsightsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const store = await read<StoreClock>('/v1/platform/store');
  const today = store.ok ? store.data.now.slice(0, 10) : isoDate();
  const asked = param(sp, 'day');
  const day = asked && DATE.test(asked) ? asked : shift(today, -1);
  const askedCompare = param(sp, 'compare');
  const compare = COMPARE.some(c => c.value === askedCompare) ? (askedCompare as string) : COMPARE[0].value;
  const compareFrom = param(sp, 'compare_from');
  const compareThrough = param(sp, 'compare_through');
  const custom = compare === 'custom';

  const [trend, explanation] = await Promise.all([
    readPost<Page<SalesTrendPoint>>('/v1/analytics/sales/trend', {
      valid_from: shift(day, -(CHART_DAYS - 1)),
      valid_through: day,
      'acom:granularity': 'day',
      'acom:metrics': ['revenue'],
    }),
    custom && !(compareFrom && compareThrough)
      ? Promise.resolve(null)
      : readPost<SalesExplanation>('/v1/analytics/sales/explain', {
          valid_from: day,
          valid_through: day,
          compare_mode: compare,
          ...(custom ? {compare_from: compareFrom, compare_through: compareThrough} : {}),
        }),
  ]);

  const points = trend.ok
    ? trend.data.items
        .filter(i => i['acom:metric'] === 'revenue')
        .map(i => ({date: i.observation_date, revenue: (i.value ?? null) as Money | null}))
    : [];
  const basis = explanation?.ok ? explanation.data.compare_to.revenue : null;
  const [y, m, d] = day.split('-').map(Number);
  const basisLabel = compare === COMPARE[0].value ? `${WEEKDAYS[new Date(Date.UTC(y, m - 1, d)).getUTCDay()]} average` : 'Basis';

  return (
    <PageFrame
      title="Insights"
      description="How sales are doing, read from the data: daily revenue, and why a day differs from its basis. Nothing here changes the store; ask the agent to act on what you see."
      toolbar={
        <FilterBar
          values={{day, compare, compare_from: compareFrom, compare_through: compareThrough}}
          filters={[
            {kind: 'date', key: 'day', label: 'Day'},
            {kind: 'select', key: 'compare', label: 'Compare with', options: COMPARE},
            ...(custom
              ? ([
                  {kind: 'date', key: 'compare_from', label: 'Compare from'},
                  {kind: 'date', key: 'compare_through', label: 'Compare through'},
                ] as const)
              : []),
          ]}
        />
      }>
      <PageSection title="Daily revenue" count={`${label(shift(day, -(CHART_DAYS - 1)))} – ${label(day)}`}>
        {trend.ok ? (
          <SalesChart points={points} baseline={basis} baselineLabel={basisLabel} marked={day} title="Daily revenue" />
        ) : (
          <ErrorState error={trend.error} title="The daily revenue could not be read." />
        )}
      </PageSection>
      <PageSection
        title={`Sales on ${label(day)}`}
        description="Causes come from the data (stock that ran out, campaign sends that reached nobody); what they do not cover stays unexplained.">
        {explanation === null ? (
          <Text color="secondary">Choose the days to compare with.</Text>
        ) : explanation.ok ? (
          <SalesExplanationView explanation={explanation.data} dayLabel={label(day)} question={`Why did sales change on ${label(day)}?`} />
        ) : (
          <ErrorState error={explanation.error} title={`Sales on ${label(day)} could not be explained.`} />
        )}
      </PageSection>
    </PageFrame>
  );
}
