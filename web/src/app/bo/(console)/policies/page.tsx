import {Text} from '@astryxdesign/core/Text';

import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame, PageSection} from '@/components/bo/PageFrame';
import {PolicyFileList, PolicyValueList} from '@/components/bo/lists/platform';

import {param, read, type SearchParams} from '../../_lib/server';
import type {Page, PolicyIndex, PolicyValue} from '../../_lib/types';

export const metadata = {title: 'Policies'};

const FILES = ['approval', 'catalog', 'community', 'customer', 'fulfillment', 'integration', 'merchandising', 'messaging', 'order', 'partner', 'platform', 'promotion', 'search'];
const KEY = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$/;

// Policies: the loaded policy files (list_policies) and the values under a key (get_policy). A
// check's rule is a policy key, so every check links here with its key filled in.
export default async function PoliciesPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const key = param(sp, 'key');
  const file = param(sp, 'file');
  const validKey = key && KEY.test(key) ? key : undefined;
  const [index, values] = await Promise.all([
    read<PolicyIndex>('/v1/platform/policies/index', {limit: 100}),
    validKey ? read<Page<PolicyValue>>('/v1/platform/policies', {key: validKey, file}) : Promise.resolve(null),
  ]);

  return (
    <PageFrame
      title="Policies"
      description="The rules the agents and the store follow, as data: defaults with this store's overrides on top. Look a key up to see its values and what they mean."
      toolbar={
        <FilterBar
          values={{key, file}}
          filters={[
            {kind: 'text', key: 'key', label: 'Policy key', placeholder: 'max_discount or thresholds.refund', width: 320},
            {kind: 'select', key: 'file', label: 'File', options: FILES.map(f => ({value: f, label: f}))},
          ]}
        />
      }>
      {key && !validKey ? (
        <Text color="secondary">A policy key is lowercase words joined by dots, like max_discount.by_tier.vip.</Text>
      ) : null}
      {values ? (
        <PageSection title={`Values under ${validKey}`} count={values.ok ? values.data.total : undefined}>
          {values.ok ? <PolicyValueList items={values.data.items} /> : <ErrorState error={values.error} title={`No policy values under “${validKey}”.`} />}
        </PageSection>
      ) : (
        <Text color="secondary">Type a policy key to see its values. Rule names on checks link here.</Text>
      )}
      <PageSection
        title="Policy files"
        count={index.ok && index.data.policy_version ? `Snapshot ${index.data.policy_version}` : undefined}
        description="Choosing a file narrows the key lookup to it.">
        {index.ok ? <PolicyFileList items={index.data.items} selected={file} /> : <ErrorState error={index.error} title="The policy files could not be loaded." />}
      </PageSection>
    </PageFrame>
  );
}
