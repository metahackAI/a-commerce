'use client';

// A new product, made the way every write is (create_product_group: preview → apply). It starts
// as a draft shoppers do not see; once it exists the page opens its sizes, colors and prices
// (set_variants), and publishing is submit for review → approve_product in a change set.

import {useMemo, useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {TextInput} from '@astryxdesign/core/TextInput';
import {TextArea} from '@astryxdesign/core/TextArea';
import {Selector} from '@astryxdesign/core/Selector';
import {MultiSelector} from '@astryxdesign/core/MultiSelector';
import {Text} from '@astryxdesign/core/Text';

import type {Brand, Category} from '@/app/bo/_lib/types';

import {TargetField} from './targets';
import {WriteFlow} from './WriteFlow';

export function NewProduct({categories, brands}: {categories: Category[]; brands: Brand[]}) {
  const router = useRouter();
  const [name, setName] = useState('');
  const [categoryId, setCategoryId] = useState<string | null>(null);
  const [brandId, setBrandId] = useState<string | null>(null);
  const [axes, setAxes] = useState<string[]>(['size']);
  const [material, setMaterial] = useState('');
  const [image, setImage] = useState('');
  const [description, setDescription] = useState('');

  const input = useMemo(() => {
    if (!name.trim() || !categoryId) return null;
    return {
      name: name.trim(),
      category_id: categoryId,
      ...(brandId ? {brand_id: brandId} : {}),
      ...(axes.length ? {varies_by: axes} : {}),
      ...(material.trim() ? {material: material.trim()} : {}),
      ...(image.trim() ? {image: [image.trim()]} : {}),
      ...(description.trim() ? {description: description.trim()} : {}),
    };
  }, [name, categoryId, brandId, axes, material, image, description]);

  return (
    <WriteFlow
      method="POST"
      path="/v1/catalog/product-groups"
      input={input}
      previewLabel="Preview the new product"
      applyLabel="Create the draft"
      askWhenApprovalNeeded={`Prepare a change set to create the product ${name}.`}
      onApplied={r => router.push(`/bo/products/${encodeURIComponent(r.id)}?edit=variants`)}>
      <Text type="supporting" color="secondary">
        The product starts as a draft that shoppers do not see. Next you add its sizes, colors and prices.
      </Text>
      <TargetField target={{kind: 'input', label: `New product name: ${name || 'empty'}`, ids: {form: 'new_product'}, term: 'schema:name'}}>
        <TextInput label="Name" value={name} onChange={setName} placeholder="Trail shell" width={420} />
      </TargetField>
      <HStack gap={3} wrap="wrap" vAlign="end">
        <TargetField target={{kind: 'input', label: `New product category: ${categoryId ?? 'none'}`, ids: {form: 'new_product'}, term: 'acom:category'}}>
          <Selector
            label="Category"
            width={280}
            options={categories.filter(c => c.term_set === 'standard' && c.active).map(c => ({value: c.id, label: c.name}))}
            value={categoryId ?? ''}
            placeholder="Choose a standard category"
            onChange={(v: string | null) => setCategoryId(v)}
          />
        </TargetField>
        <TargetField target={{kind: 'input', label: `New product brand: ${brandId ?? 'none'}`, ids: {form: 'new_product'}, term: 'schema:brand'}}>
          <Selector
            label="Brand"
            width={240}
            options={brands.map(b => ({value: b.id, label: b.name}))}
            value={brandId ?? ''}
            placeholder="Optional"
            hasClear
            onChange={(v: string | null) => setBrandId(v)}
          />
        </TargetField>
        <TargetField target={{kind: 'input', label: `New product options: ${axes.join(', ') || 'none'}`, ids: {form: 'new_product'}, term: 'schema:variesBy'}}>
          <MultiSelector
            label="Varies by"
            width={240}
            options={[
              {value: 'size', label: 'Size'},
              {value: 'color', label: 'Color'},
            ]}
            value={axes}
            placeholder="One size, one color"
            triggerDisplay="labels"
            onChange={setAxes}
          />
        </TargetField>
      </HStack>
      <HStack gap={3} wrap="wrap" vAlign="end">
        <TargetField target={{kind: 'input', label: `New product material: ${material || 'empty'}`, ids: {form: 'new_product'}, term: 'schema:material'}}>
          <TextInput label="Material" isOptional value={material} onChange={setMaterial} placeholder="Recycled nylon" width={320} />
        </TargetField>
        <TargetField target={{kind: 'input', label: 'New product main image', ids: {form: 'new_product'}, term: 'schema:image'}}>
          <TextInput label="Main image URL" isOptional value={image} onChange={setImage} placeholder="https://…" width={320} />
        </TargetField>
      </HStack>
      <TargetField target={{kind: 'input', label: 'New product description', ids: {form: 'new_product'}, term: 'schema:description'}}>
        <TextArea
          label="Description"
          isOptional
          description="In the store’s tone. Publishing needs one; you can write it later."
          value={description}
          onChange={setDescription}
          rows={3}
          width={640}
        />
      </TargetField>
    </WriteFlow>
  );
}
