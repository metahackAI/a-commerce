'use client';

// The two category trees (merchandising.md#category): the standard tree products are filed in,
// and the display tree shoppers browse. Create, rename, move or switch off a category
// (create_category, update_category), and show a standard category's products in display
// categories (map_category, unmap_category). Every change is previewed with its rule checks
// (depth, unique term code, no cycle, nothing left under a category being switched off).

import {useEffect, useMemo, useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TextInput} from '@astryxdesign/core/TextInput';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {Selector} from '@astryxdesign/core/Selector';
import {MultiSelector} from '@astryxdesign/core/MultiSelector';
import {Switch} from '@astryxdesign/core/Switch';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';

import {api, ApiError, type ApiErrorBody} from '@/lib/api';
import {count} from '@/app/bo/_lib/format';
import type {Category, CategoryDetail} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {ErrorState} from './ErrorState';
import {PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {DataTable, Figure} from './tables';
import {TargetField, TargetInline} from './targets';
import {WriteFlow} from './WriteFlow';

type Open =
  | {kind: 'new'}
  | {kind: 'edit'; c: Category}
  | {kind: 'map'; c: Category}
  | {kind: 'mappings'; c: Category}
  | null;

function toError(e: unknown): ApiErrorBody {
  if (e instanceof ApiError) return e.body;
  return {code: 'upstream_unavailable', class: 'upstream_unavailable', message: 'The store could not be reached.', next_tool: null};
}

/** How deep a category sits, for indenting its name (layout only). */
function depthOf(c: Category, byId: Map<string, Category>): number {
  let d = 0;
  let p = c.parent_id ? byId.get(c.parent_id) : undefined;
  while (p && d < 10) {
    d += 1;
    p = p.parent_id ? byId.get(p.parent_id) : undefined;
  }
  return d;
}

export function CategoryManager({categories}: {categories: Category[]}) {
  const router = useRouter();
  const session = useBoSession();
  const canWrite = session?.scopes.includes('tools:write:merchandising') ?? false;
  const [open, setOpen] = useState<Open>(null);
  const byId = useMemo(() => new Map(categories.map(c => [c.id, c])), [categories]);
  const close = () => setOpen(null);
  const done = () => {
    setOpen(null);
    router.refresh();
  };

  const table = (set: 'standard' | 'display') => (
    <DataTable
      rows={categories.filter(c => c.term_set === set)}
      rowKey={c => c.id}
      empty={{title: 'No categories yet'}}
      target={c => ({kind: 'category', label: `Category ${c.name} (${c.term_set})${c.active ? '' : ', inactive'}`, ids: {category_id: c.id}, term: 'schema:DefinedTerm'})}
      columns={[
        {
          header: 'Category',
          cell: c => (
            <VStack gap={0}>
              <Text weight="medium">{`${'— '.repeat(depthOf(c, byId))}${c.name}`}</Text>
              <Text type="supporting" color="secondary">
                {c.id}
              </Text>
            </VStack>
          ),
        },
        {header: 'Term code', cell: c => <Text color="secondary">{c.term_code ?? '—'}</Text>},
        {header: 'Order', numeric: true, cell: c => <Figure>{count(c.display_order)}</Figure>},
        {header: 'State', cell: c => <StateToken state={c.active ? 'active' : 'inactive'} />},
        {
          header: 'Change',
          cell: c =>
            canWrite ? (
              <HStack gap={1} wrap="wrap">
                <Button label="Edit" size="sm" onClick={() => setOpen({kind: 'edit', c})} />
                {set === 'standard' ? <Button label="Show in menu" size="sm" variant="ghost" onClick={() => setOpen({kind: 'map', c})} /> : null}
                <Button label="Mappings" size="sm" variant="ghost" onClick={() => setOpen({kind: 'mappings', c})} />
              </HStack>
            ) : null,
        },
      ]}
    />
  );

  return (
    <VStack gap={6}>
      {canWrite ? (
        <HStack gap={2}>
          <Button label="New category" variant="primary" onClick={() => setOpen({kind: 'new'})} />
        </HStack>
      ) : null}
      <PageSection title="Standard categories" description="The store’s own classification. Every product is filed in one.">
        {table('standard')}
      </PageSection>
      <PageSection title="Display categories" description="The menu shoppers browse. It shows standard categories’ products through mappings.">
        {table('display')}
      </PageSection>

      {open?.kind === 'new' ? <NewCategory categories={categories} onClose={close} onDone={done} /> : null}
      {open?.kind === 'edit' ? <EditCategory c={open.c} categories={categories} onClose={close} onDone={done} /> : null}
      {open?.kind === 'map' ? <MapCategory c={open.c} categories={categories} onClose={close} onDone={done} /> : null}
      {open?.kind === 'mappings' ? <Mappings c={open.c} byId={byId} onClose={close} onDone={done} /> : null}
    </VStack>
  );
}

function Frame({title, subtitle, onClose, children}: {title: string; subtitle?: string; onClose: () => void; children: React.ReactNode}) {
  return (
    <Dialog isOpen onOpenChange={o => !o && onClose()} width={720} purpose="form">
      <DialogHeader title={title} subtitle={subtitle} onOpenChange={o => !o && onClose()} />
      <VStack gap={4} padding={4}>
        {children}
      </VStack>
    </Dialog>
  );
}

function NewCategory({categories, onClose, onDone}: {categories: Category[]; onClose: () => void; onDone: () => void}) {
  const [set, setSet] = useState<string>('standard');
  const [name, setName] = useState('');
  const [parent, setParent] = useState<string | null>(null);
  const [order, setOrder] = useState<number | null>(null);
  const [code, setCode] = useState('');
  const input = name.trim()
    ? {
        term_set: set,
        name: name.trim(),
        ...(parent ? {parent_id: parent} : {}),
        ...(order !== null ? {display_order: order} : {}),
        ...(code.trim() ? {term_code: code.trim()} : {}),
      }
    : null;
  return (
    <Frame title="New category" onClose={onClose}>
      <WriteFlow method="POST" path="/v1/merchandising/categories" input={input} previewLabel="Preview" applyLabel="Create" askWhenApprovalNeeded={`Prepare a change set to create the category ${name}.`} onApplied={onDone}>
        <HStack gap={3} wrap="wrap" vAlign="end">
          <Selector
            label="Tree"
            width={200}
            options={[
              {value: 'standard', label: 'Standard'},
              {value: 'display', label: 'Display (menu)'},
            ]}
            value={set}
            onChange={(v: string | null) => {
              setSet(v ?? 'standard');
              setParent(null);
            }}
          />
          <TargetField target={{kind: 'input', label: `New category name: ${name || 'empty'}`, ids: {form: 'new_category'}, term: 'schema:name'}}>
            <TextInput label="Name" value={name} onChange={setName} placeholder="Cardigans" width={280} />
          </TargetField>
        </HStack>
        <HStack gap={3} wrap="wrap" vAlign="end">
          <TargetField target={{kind: 'input', label: `New category parent: ${parent ?? 'top level'}`, ids: {form: 'new_category'}, term: 'schema:broader'}}>
            <Selector
              label="Under"
              width={260}
              options={categories.filter(c => c.term_set === set).map(c => ({value: c.id, label: c.name}))}
              value={parent ?? ''}
              placeholder="Top level"
              hasClear
              onChange={(v: string | null) => setParent(v)}
            />
          </TargetField>
          <NumberInput label="Order" isOptional value={order} onChange={v => setOrder(Number.isFinite(v) ? v : null)} min={1} isIntegerOnly width={120} />
          <TargetField target={{kind: 'input', label: `New category term code: ${code || 'from the name'}`, ids: {form: 'new_category'}, term: 'schema:termCode'}}>
            <TextInput label="Term code" isOptional value={code} onChange={setCode} placeholder="cardigans" width={200} />
          </TargetField>
        </HStack>
      </WriteFlow>
    </Frame>
  );
}

function EditCategory({c, categories, onClose, onDone}: {c: Category; categories: Category[]; onClose: () => void; onDone: () => void}) {
  const [name, setName] = useState(c.name);
  const [order, setOrder] = useState<number | null>(c.display_order ?? null);
  const [parent, setParent] = useState<string | null>(c.parent_id);
  const [active, setActive] = useState(c.active);
  const input: Record<string, unknown> = {category_id: c.id};
  if (name.trim() && name.trim() !== c.name) input.name = name.trim();
  if (order !== null && order !== c.display_order) input.display_order = order;
  if (parent && parent !== c.parent_id) input.parent_id = parent;
  if (active !== c.active) input.active = active;
  return (
    <Frame title={`Edit ${c.name}`} subtitle={`${c.id} · ${c.term_set}`} onClose={onClose}>
      <WriteFlow
        method="PATCH"
        path={`/v1/merchandising/categories/${encodeURIComponent(c.id)}`}
        input={Object.keys(input).length > 1 ? input : null}
        previewLabel="Preview"
        applyLabel="Apply"
        askWhenApprovalNeeded={`Prepare a change set to change the category ${c.name}.`}
        onApplied={onDone}>
        <HStack gap={3} wrap="wrap" vAlign="end">
          <TargetField target={{kind: 'input', label: `Name of ${c.id}: ${name}`, ids: {category_id: c.id}, term: 'schema:name'}}>
            <TextInput label="Name" value={name} onChange={setName} width={280} />
          </TargetField>
          <NumberInput label="Order" value={order} onChange={v => setOrder(Number.isFinite(v) ? v : null)} min={1} isIntegerOnly width={120} />
        </HStack>
        <TargetField target={{kind: 'input', label: `Parent of ${c.id}: ${parent ?? 'top level'}`, ids: {category_id: c.id}, term: 'schema:broader'}}>
          <Selector
            label="Under"
            width={280}
            options={categories.filter(x => x.term_set === c.term_set && x.id !== c.id).map(x => ({value: x.id, label: x.name}))}
            value={parent ?? ''}
            placeholder="Top level"
            onChange={(v: string | null) => setParent(v)}
          />
        </TargetField>
        <TargetField target={{kind: 'input', label: `${c.name} is ${active ? 'active' : 'switched off'}`, ids: {category_id: c.id}, term: 'acom:active'}}>
          <Switch label="Active" description="Switching it off needs nothing under it, mapped to it or filed in it." value={active} onChange={setActive} />
        </TargetField>
      </WriteFlow>
    </Frame>
  );
}

function MapCategory({c, categories, onClose, onDone}: {c: Category; categories: Category[]; onClose: () => void; onDone: () => void}) {
  const [displays, setDisplays] = useState<string[]>([]);
  const [primary, setPrimary] = useState(false);
  const input = displays.length ? {standard_category_id: c.id, display_category_ids: displays, ...(primary ? {is_primary: true} : {})} : null;
  return (
    <Frame title={`Show ${c.name} in the menu`} subtitle={c.id} onClose={onClose}>
      <WriteFlow method="POST" path="/v1/merchandising/category-mappings" input={input} previewLabel="Preview" applyLabel="Map" askWhenApprovalNeeded={`Prepare a change set to map ${c.name}.`} onApplied={onDone}>
        <Text type="supporting" color="secondary">
          The preview says how many products start showing under each display category. Clear the display cache afterwards so shoppers see it.
        </Text>
        <TargetField target={{kind: 'input', label: `Display categories for ${c.name}: ${displays.join(', ') || 'none'}`, ids: {category_id: c.id}, term: 'acom:displayCategory'}}>
          <MultiSelector
            label="Display categories"
            width={420}
            options={categories.filter(x => x.term_set === 'display' && x.active).map(x => ({value: x.id, label: x.name}))}
            value={displays}
            triggerDisplay="labels"
            onChange={setDisplays}
          />
        </TargetField>
        <Switch label="Make the first one its primary path" description="A standard category has one primary display category; product pages show it as their path." value={primary} onChange={setPrimary} />
      </WriteFlow>
    </Frame>
  );
}

function Mappings({c, byId, onClose, onDone}: {c: Category; byId: Map<string, Category>; onClose: () => void; onDone: () => void}) {
  const [detail, setDetail] = useState<CategoryDetail | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    api<CategoryDetail>(`/v1/merchandising/categories/${encodeURIComponent(c.id)}`)
      .then(d => alive && setDetail(d))
      .catch(e => alive && setError(toError(e)));
    return () => {
      alive = false;
    };
  }, [c.id]);
  const name = (id: string) => byId.get(id)?.name ?? id;
  return (
    <Frame title={`Mappings of ${c.name}`} subtitle={detail ? `${count(detail.product_count)} products in it or under it` : c.id} onClose={onClose}>
      {error ? <ErrorState error={error} title="The mappings could not be read." /> : null}
      {!detail && !error ? <Spinner size="sm" label="Reading the mappings…" /> : null}
      {detail && detail.mappings.length === 0 ? <Text color="secondary">No mappings.</Text> : null}
      {detail?.mappings.map(m => (
        <VStack key={m.id} gap={2}>
          <TargetInline target={{kind: 'category_mapping', label: `${name(m.standard_category_id)} shows in ${name(m.display_category_id)}${m.is_primary ? ' (primary)' : ''}`, ids: {category_mapping_id: m.id}}}>
            <Text>{`${name(m.standard_category_id)} → ${name(m.display_category_id)}`}</Text>
            {m.is_primary ? <StateToken state="active" label="Primary" /> : null}
            <Button label="Remove" size="sm" variant="ghost" onClick={() => setRemoving(m.id)} />
          </TargetInline>
          {removing === m.id ? (
            <WriteFlow
              method="DELETE"
              path={`/v1/merchandising/category-mappings/${encodeURIComponent(m.id)}`}
              input={{category_mapping_id: m.id}}
              previewLabel="Preview the removal"
              applyLabel="Remove"
              askWhenApprovalNeeded={`Prepare a change set to remove the mapping ${m.id}.`}
              onApplied={onDone}
            />
          ) : null}
        </VStack>
      ))}
    </Frame>
  );
}
