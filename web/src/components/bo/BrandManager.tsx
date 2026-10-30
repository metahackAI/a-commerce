'use client';

// Brands (catalog.md#brand): add one (create_brand) or change its name or logo (update_brand),
// each previewed first. Names are unique in the store; the server says so when one is taken.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Text} from '@astryxdesign/core/Text';

import {withParams} from '@/app/bo/_lib/links';
import type {Brand} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {DataTable, RecordCell} from './tables';
import {TargetField} from './targets';
import {WriteFlow} from './WriteFlow';

export function BrandManager({brands}: {brands: Brand[]}) {
  const router = useRouter();
  const session = useBoSession();
  const canWrite = session?.scopes.includes('tools:write:catalog') ?? false;
  const [open, setOpen] = useState<{kind: 'new'} | {kind: 'edit'; b: Brand} | null>(null);
  const [name, setName] = useState('');
  const [logo, setLogo] = useState('');
  const start = (o: typeof open) => {
    setName(o?.kind === 'edit' ? o.b.name : '');
    setLogo(o?.kind === 'edit' ? (o.b.logo ?? '') : '');
    setOpen(o);
  };
  const done = () => {
    setOpen(null);
    router.refresh();
  };

  let input: Record<string, unknown> | null = null;
  if (open?.kind === 'new' && name.trim()) {
    input = {name: name.trim(), ...(logo.trim() ? {logo: logo.trim()} : {})};
  } else if (open?.kind === 'edit') {
    const changes: Record<string, unknown> = {};
    if (name.trim() && name.trim() !== open.b.name) changes.name = name.trim();
    if (logo.trim() !== (open.b.logo ?? '')) changes.logo = logo.trim();
    input = Object.keys(changes).length ? {brand_id: open.b.id, ...changes} : null;
  }

  return (
    <VStack gap={4}>
      {canWrite ? (
        <HStack gap={2}>
          <Button label="New brand" variant="primary" onClick={() => start({kind: 'new'})} />
        </HStack>
      ) : null}
      <DataTable
        rows={brands}
        rowKey={b => b.id}
        empty={{title: 'No brands yet'}}
        target={b => ({kind: 'brand', label: `Brand ${b.name}`, ids: {brand_id: b.id}, term: 'schema:Brand'})}
        columns={[
          {header: 'Brand', cell: b => <RecordCell href={withParams('/bo/products', {brand_id: b.id})} name={b.name} id={b.id} />},
          {header: 'Logo', cell: b => <Text color="secondary">{b.logo ?? '—'}</Text>},
          {header: 'About', cell: b => <Text color="secondary">{b.description ?? '—'}</Text>},
          {header: 'Change', cell: b => (canWrite ? <Button label="Edit" size="sm" onClick={() => start({kind: 'edit', b})} /> : null)},
        ]}
      />
      {open ? (
        <Dialog isOpen onOpenChange={o => !o && setOpen(null)} width={640} purpose="form">
          <DialogHeader
            title={open.kind === 'new' ? 'New brand' : `Edit ${open.b.name}`}
            subtitle={open.kind === 'edit' ? open.b.id : undefined}
            onOpenChange={o => !o && setOpen(null)}
          />
          <VStack gap={4} padding={4}>
            <WriteFlow
              method={open.kind === 'new' ? 'POST' : 'PATCH'}
              path={open.kind === 'new' ? '/v1/catalog/brands' : `/v1/catalog/brands/${encodeURIComponent(open.b.id)}`}
              input={input}
              previewLabel="Preview"
              applyLabel={open.kind === 'new' ? 'Add the brand' : 'Apply'}
              askWhenApprovalNeeded={`Prepare a change set for the brand ${name}.`}
              onApplied={done}>
              <TargetField target={{kind: 'input', label: `Brand name: ${name || 'empty'}`, ids: {form: 'brand'}, term: 'schema:name'}}>
                <TextInput label="Name" value={name} onChange={setName} placeholder="Harbor Knit Co." width={320} />
              </TargetField>
              <TargetField target={{kind: 'input', label: 'Brand logo URL', ids: {form: 'brand'}, term: 'schema:logo'}}>
                <TextInput label="Logo URL" isOptional value={logo} onChange={setLogo} placeholder="https://…" width={420} />
              </TargetField>
            </WriteFlow>
          </VStack>
        </Dialog>
      ) : null}
    </VStack>
  );
}
