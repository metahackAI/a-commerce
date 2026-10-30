'use client';

// Edit one product the way every write goes: details through update_product_group, sizes, colors
// and prices through set_variants. Each is previewed first; the page shows the changes and the
// rule checks, applies it when no approval is needed, and otherwise says why and offers to have
// the agent put it in a change set. Prices are typed by the person and checked by the server
// (price floor); the web computes nothing.

import {useMemo, useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TabList, Tab} from '@astryxdesign/core/TabList';
import {TextInput} from '@astryxdesign/core/TextInput';
import {TextArea} from '@astryxdesign/core/TextArea';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {Selector} from '@astryxdesign/core/Selector';
import {Button} from '@astryxdesign/core/Button';
import {Banner} from '@astryxdesign/core/Banner';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {Text} from '@astryxdesign/core/Text';

import {useAi} from '@/ai/AiProvider';
import {words} from '@/app/bo/_lib/format';
import type {Brand, Category, ProductGroup} from '@/app/bo/_lib/types';

import {useAgentDock} from './AgentDock';
import {TargetField} from './targets';
import {WriteFlow} from './WriteFlow';

export type EditorTab = 'details' | 'variants';

interface Row {
  key: string;
  sku: string;
  color: string;
  size: string;
  price: number | null;
  cost: number | null;
}

function rowsOf(g: ProductGroup): Row[] {
  return (g.has_variant ?? []).map(v => ({
    key: v.variant_id,
    sku: v.sku ?? '',
    color: v.color ?? '',
    size: v.size ?? '',
    price: typeof v.price === 'number' ? v.price : null,
    cost: typeof v.cost === 'number' ? v.cost : null,
  }));
}

/** The values of one option axis the rows use, in first-seen order. */
function axis(rows: Row[], key: 'color' | 'size'): string[] {
  const out: string[] = [];
  for (const r of rows) {
    const v = r[key].trim();
    if (v && !out.includes(v)) out.push(v);
  }
  return out;
}

export function ProductEditor({
  group,
  categories,
  brands,
  currency,
  tab,
  onTabChange,
  onClose,
}: {
  group: ProductGroup;
  categories: Category[];
  brands: Brand[];
  /** The currency prices are shown in, when known; the server prices in the store's currency. */
  currency: string | null;
  tab: EditorTab;
  onTabChange: (t: EditorTab) => void;
  onClose: () => void;
}) {
  const router = useRouter();
  const ai = useAi();
  const dock = useAgentDock();
  const draft = group['acom:draft'];
  const id = group.id;
  const enc = encodeURIComponent(id);

  const [name, setName] = useState(group.name);
  const [description, setDescription] = useState(group.description ?? '');
  const [material, setMaterial] = useState(group.material ?? '');
  const [image, setImage] = useState(group.image ?? '');
  const [categoryId, setCategoryId] = useState<string | null>(group.category_id ?? null);
  const [brandId, setBrandId] = useState<string | null>(group.brand_id ?? null);

  const details = useMemo(() => {
    const input: Record<string, unknown> = {product_id: id};
    if (name.trim() && name.trim() !== group.name) input.name = name.trim();
    if (description.trim() !== (group.description ?? '')) input.description = description.trim();
    if (material.trim() && material.trim() !== (group.material ?? '')) input.material = material.trim();
    if (image.trim() && image.trim() !== (group.image ?? '')) input.image = [image.trim()];
    if (categoryId && categoryId !== group.category_id) input.category_id = categoryId;
    if (brandId && brandId !== group.brand_id) input.brand_id = brandId;
    return Object.keys(input).length > 1 ? input : null;
  }, [id, name, description, material, image, categoryId, brandId, group]);

  const [rows, setRows] = useState<Row[]>(() => rowsOf(group));
  const setRow = (key: string, patch: Partial<Row>) => setRows(rs => rs.map(r => (r.key === key ? {...r, ...patch} : r)));
  const variants = useMemo(() => {
    const listed = rows.filter(r => r.sku.trim());
    if (listed.length === 0) return null;
    const colors = axis(listed, 'color');
    const sizes = axis(listed, 'size');
    return {
      product_id: id,
      varies_by: {...(colors.length ? {color: colors} : {}), ...(sizes.length ? {size: sizes} : {})},
      has_variant: listed.map(r => ({
        sku: r.sku.trim(),
        additional_property: {...(r.color.trim() ? {color: r.color.trim()} : {}), ...(r.size.trim() ? {size: r.size.trim()} : {})},
        ...(r.price !== null ? {offers: {price: r.price}} : {}),
        ...(r.cost !== null ? {'acom:cost': {price: r.cost}} : {}),
      })),
    };
  }, [rows, id]);

  const standard = categories.filter(c => c.term_set === 'standard');

  return (
    <Dialog isOpen onOpenChange={open => !open && onClose()} width={880} purpose="form">
      <DialogHeader title={`Edit ${group.name}`} subtitle={`${id} · ${words(group.approval_status)}`} onOpenChange={open => !open && onClose()} />
      <VStack gap={4} padding={4}>
        <TabList value={tab} onChange={v => onTabChange(v === 'variants' ? 'variants' : 'details')} hasDivider>
          <Tab value="details" label="Details" />
          <Tab value="variants" label="Sizes, colors and prices" />
        </TabList>

        {tab === 'details' ? (
          <WriteFlow
            method="PATCH"
            path={`/v1/catalog/product-groups/${enc}`}
            input={details}
            previewLabel="Preview the changes"
            applyLabel="Apply"
            askWhenApprovalNeeded={`Prepare a change set to change ${group.name} (${id}): ${JSON.stringify(details ?? {})}`}
            onApplied={() => router.refresh()}>
            {draft ? (
              <Banner
                status={draft.draft_status === 'needs_you' ? 'warning' : 'info'}
                title={draft.draft_status === 'needs_you' ? `Needs you: ${draft.open_fields.map(words).join(', ')}` : 'A draft from a supplier sheet'}
                description={[`Row ${draft.row} (${draft.supplier_code}) of import ${draft.import_id}.`, draft.note].filter(Boolean).join(' ')}
              />
            ) : null}
            <TargetField target={{kind: 'input', label: `Name of ${id}: ${name}`, ids: {product_id: id}, term: 'schema:name'}}>
              <TextInput label="Name" value={name} onChange={setName} width={480} />
            </TargetField>
            {draft?.supplier_text ? (
              <VStack gap={1}>
                <Text type="label">The supplier’s text</Text>
                <Blockquote>{draft.supplier_text}</Blockquote>
                <HStack gap={2}>
                  <Button
                    label="Ask the agent to write it in our tone"
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      dock.open();
                      void ai.send(`Write the description of ${group.name} (${id}) in our tone from the supplier's text.`);
                    }}
                  />
                </HStack>
              </VStack>
            ) : null}
            <TargetField target={{kind: 'input', label: `Description of ${id}`, ids: {product_id: id}, term: 'schema:description'}}>
              <TextArea
                label="Description"
                description="In the store’s tone: two sentences on what it is and how it wears, then a care line. Publishing needs one."
                value={description}
                onChange={setDescription}
                rows={4}
                width={640}
              />
            </TargetField>
            <HStack gap={3} wrap="wrap" vAlign="end">
              <TargetField target={{kind: 'input', label: `Material of ${id}: ${material || 'empty'}`, ids: {product_id: id}, term: 'schema:material'}}>
                <TextInput label="Material" value={material} onChange={setMaterial} placeholder="60% wool, 40% polyamide" width={320} />
              </TargetField>
              <TargetField target={{kind: 'input', label: `Main image of ${id}`, ids: {product_id: id}, term: 'schema:image'}}>
                <TextInput label="Main image URL" value={image} onChange={setImage} placeholder="https://…" width={320} />
              </TargetField>
            </HStack>
            <HStack gap={3} wrap="wrap" vAlign="end">
              <TargetField target={{kind: 'input', label: `Category of ${id}: ${categoryId ?? 'none'}`, ids: {product_id: id}, term: 'acom:category'}}>
                <Selector
                  label="Category"
                  width={280}
                  options={standard.map(c => ({value: c.id, label: c.name}))}
                  value={categoryId ?? ''}
                  placeholder="Choose a standard category"
                  onChange={(v: string | null) => setCategoryId(v)}
                />
              </TargetField>
              <TargetField target={{kind: 'input', label: `Brand of ${id}: ${brandId ?? 'none'}`, ids: {product_id: id}, term: 'schema:brand'}}>
                <Selector
                  label="Brand"
                  width={240}
                  options={brands.map(b => ({value: b.id, label: b.name}))}
                  value={brandId ?? ''}
                  placeholder="Choose a brand"
                  onChange={(v: string | null) => setBrandId(v)}
                />
              </TargetField>
            </HStack>
          </WriteFlow>
        ) : (
          <WriteFlow
            method="PUT"
            path={`/v1/catalog/product-groups/${enc}/variants`}
            input={variants}
            previewLabel="Preview the variants"
            applyLabel="Apply"
            askWhenApprovalNeeded={`Prepare a change set to set the variants of ${group.name} (${id}).`}
            onApplied={() => router.refresh()}>
            <Text type="supporting" color="secondary">
              One row per SKU. A SKU this product already has is updated; a variant you remove stops being sold (it is never deleted). Stock is set per variant from the product page.
            </Text>
            <VStack gap={2}>
              {rows.map(r => (
                <TargetField key={r.key} target={{kind: 'variant_row', label: `Variant ${r.sku || 'new'} ${r.color} ${r.size}`, ids: {product_id: id, sku: r.sku}}}>
                  <HStack gap={2} vAlign="end" wrap="wrap">
                    <TextInput label="SKU" size="sm" value={r.sku} onChange={v => setRow(r.key, {sku: v})} width={180} />
                    <TextInput label="Color" size="sm" isOptional value={r.color} onChange={v => setRow(r.key, {color: v})} width={120} />
                    <TextInput label="Size" size="sm" isOptional value={r.size} onChange={v => setRow(r.key, {size: v})} width={90} />
                    <NumberInput label="List price" size="sm" value={r.price} onChange={v => setRow(r.key, {price: Number.isFinite(v) ? v : null})} min={0} step={0.01} units={currency ?? undefined} width={140} />
                    <NumberInput label="Cost" size="sm" value={r.cost} onChange={v => setRow(r.key, {cost: Number.isFinite(v) ? v : null})} min={0} step={0.01} units={currency ?? undefined} width={140} />
                    <StackItem>
                      <Button label="Remove" size="sm" variant="ghost" onClick={() => setRows(rs => rs.filter(x => x.key !== r.key))} />
                    </StackItem>
                  </HStack>
                </TargetField>
              ))}
              <HStack gap={2}>
                <Button
                  label="Add a variant"
                  size="sm"
                  onClick={() => setRows(rs => [...rs, {key: `new_${rs.length}_${Date.now()}`, sku: '', color: '', size: '', price: null, cost: null}])}
                />
              </HStack>
            </VStack>
          </WriteFlow>
        )}
      </VStack>
    </Dialog>
  );
}
