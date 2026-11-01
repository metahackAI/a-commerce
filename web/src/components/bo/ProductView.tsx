'use client';

// One product for an operator: the sales view (get_product: list price, member price, cost,
// stock), the catalog view (get_product_group: approval, options, pending changes), stock per
// variant (get_inventory), reviews (get_review_summary) and its change history
// (get_change_history). Prices and stock of a variant are edited with preview → apply.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {Token} from '@astryxdesign/core/Token';

import type {ApiErrorBody} from '@/lib/api';
import {actor, count, date, dateTime, price, value, words} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {Brand, Category, HistoryEntry, Inventory, Product, ProductGroup, ReviewSummary, Variant} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {DataTable, Figure} from './tables';
import {TargetFact, TargetInline} from './targets';
import {VariantEditor, type EditMode} from './VariantEditor';
import {ProductEditor, type EditorTab} from './ProductEditor';
import {WriteFlow} from './WriteFlow';

type InvVariant = Inventory['variants'][number];

export interface ProductViewProps {
  productId: string;
  product: Product | null;
  productError: ApiErrorBody | null;
  group: ProductGroup | null;
  inventory: Inventory | null;
  inventoryError: ApiErrorBody | null;
  reviews: ReviewSummary | null;
  reviewsError: ApiErrorBody | null;
  history: HistoryEntry[] | null;
  historyError: ApiErrorBody | null;
  /** For the editor: the store's categories and brands. */
  categories?: Category[];
  brands?: Brand[];
  /** Opens the editor on this tab (a new product goes straight to its variants). */
  initialEdit?: EditorTab | null;
}

/** One open reorder: its units and arrival, and taking it in once it is here (receive_reorder). */
function ReorderLine({reorder: r, canReceive}: {reorder: NonNullable<Inventory['reorders']>[number]; canReceive: boolean}) {
  const router = useRouter();
  const [receiving, setReceiving] = useState(false);
  return (
    <VStack gap={1}>
      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text hasTabularNumbers>{`${count(r.total_units)} arriving by ${dateTime(r.expected_arrival_until)}`}</Text>
        {canReceive && !receiving ? <Button label="Receive…" size="sm" variant="ghost" onClick={() => setReceiving(true)} /> : null}
      </HStack>
      {receiving ? (
        <WriteFlow
          method="POST"
          path={`/v1/catalog/reorders/${encodeURIComponent(r.id)}/receive`}
          input={{reorder_id: r.id}}
          previewLabel="Preview the receipt"
          applyLabel="Receive"
          askWhenApprovalNeeded={`Prepare a change set to receive reorder ${r.id}.`}
          onApplied={() => {
            setReceiving(false);
            router.refresh();
          }}
        />
      ) : null}
    </VStack>
  );
}

/** A brand's or category's name from the lists the page loaded, or the id when it is not there. */
function nameIn(list: {id: string; name: string}[] | undefined, id: string | null | undefined): string {
  if (!id) return '—';
  return list?.find(x => x.id === id)?.name ?? id;
}

export function ProductView(p: ProductViewProps) {
  const session = useBoSession();
  const canWrite = session?.scopes.includes('tools:write:catalog') ?? false;
  const [editing, setEditing] = useState<{variant: InvVariant; mode: EditMode} | null>(null);
  const [editor, setEditor] = useState<EditorTab | null>(p.initialEdit ?? null);
  const draft = p.group?.['acom:draft'];
  const product = p.product;
  const currency = product?.price_currency ?? p.group?.has_variant?.find(v => v.price_currency)?.price_currency ?? p.group?.price_currency ?? null;
  const name = product?.name ?? p.group?.name ?? p.productId;
  const ids = {product_id: p.productId};

  // Inventory rows carry stock; the product's variant list carries size and color. Shown together.
  const options = new Map<string, Variant>((product?.has_variant ?? p.group?.has_variant ?? []).map(v => [v.variant_id, v]));
  const reorders = p.inventory?.reorders ?? [];
  const variants: InvVariant[] = p.inventory
    ? p.inventory.variants.map(v => ({...options.get(v.variant_id), ...v}))
    : [...options.values()];

  return (
    <PageFrame
      back={{href: '/bo/products', label: 'Products'}}
      title={name}
      meta={
        <>
          <Text type="supporting" color="secondary">
            {p.productId}
          </Text>
          {product?.approval_status ?? p.group?.approval_status ? <StateToken state={product?.approval_status ?? p.group?.approval_status} /> : null}
          {product?.availability ? <StateToken state={product.availability} /> : null}
        </>
      }
      actions={
        <>
          {canWrite && p.group ? <Button label="Edit details" onClick={() => setEditor('details')} /> : null}
          {canWrite && p.group ? <Button label="Sizes, colors and prices" onClick={() => setEditor('variants')} /> : null}
          {draft ? <Button label="Product drafts" variant="ghost" href={withParams('/bo/products/drafts', {import_id: draft.import_id})} /> : null}
          <Button label="Change history in activity" variant="ghost" href={withParams('/bo/activity', {object_id: p.productId})} />
        </>
      }
      panel={
        <MetadataList title="Catalog">
          <MetadataListItem label="Brand">{nameIn(p.brands, product?.brand_id ?? p.group?.brand_id)}</MetadataListItem>
          <MetadataListItem label="Category">{nameIn(p.categories, product?.category_id ?? p.group?.category_id)}</MetadataListItem>
          <MetadataListItem label="Options">{(product?.varies_by ?? p.group?.varies_by ?? []).map(words).join(', ') || '—'}</MetadataListItem>
          <MetadataListItem label="Approval">{words(p.group?.approval_status ?? product?.approval_status)}</MetadataListItem>
          <MetadataListItem label="Pending changes">{p.group?.pending_changes?.length ? value(p.group.pending_changes) : 'None'}</MetadataListItem>
          {p.group?.material ? <MetadataListItem label="Material">{p.group.material}</MetadataListItem> : null}
          {p.group?.description ? <MetadataListItem label="Description">{p.group.description}</MetadataListItem> : null}
          {draft ? (
            <MetadataListItem label="Draft">
              <VStack gap={1}>
                <StateToken state={draft.draft_status} />
                <Text type="supporting" color="secondary">
                  {[`Row ${draft.row} · ${draft.supplier_code}`, draft.open_fields.length ? `Open: ${draft.open_fields.map(words).join(', ')}` : null, draft.note]
                    .filter(Boolean)
                    .join(' · ')}
                </Text>
              </VStack>
            </MetadataListItem>
          ) : null}
        </MetadataList>
      }>
      {editor && p.group ? (
        <ProductEditor
          group={p.group}
          categories={p.categories ?? []}
          brands={p.brands ?? []}
          currency={currency}
          tab={editor}
          onTabChange={setEditor}
          onClose={() => setEditor(null)}
        />
      ) : null}
      {p.productError ? <ErrorState error={p.productError} title="The product’s sales view could not be loaded." /> : null}

      {product ? (
        <PageSection title="Price and stock">
          <MetadataList columns="multi">
            <TargetFact label="List price" target={{kind: 'price', label: `${name} list price ${price(product.price, product.price_currency)}`, ids: {price: 'list', ...ids}, term: 'schema:Offer'}}>
              <Text weight="semibold" hasTabularNumbers>
                {price(product.price, product.price_currency)}
              </Text>
            </TargetFact>
            <TargetFact
              label="Member price now"
              target={{
                kind: 'price',
                label: `${name} member price ${product.member_price ? price(product.member_price.price, product.price_currency) : 'none'}`,
                ids: {price: 'member', ...ids},
                term: 'schema:UnitPriceSpecification',
              }}>
              {product.member_price ? (
                <VStack gap={0}>
                  <Text weight="semibold" hasTabularNumbers>
                    {price(product.member_price.price, product.price_currency)}
                  </Text>
                  <Text type="supporting" color="secondary">
                    {[product.member_price.name, product.member_price.valid_for_member_tier, product.member_price.valid_through ? `until ${dateTime(product.member_price.valid_through)}` : null]
                      .filter(Boolean)
                      .join(' · ')}
                  </Text>
                </VStack>
              ) : (
                <Text color="secondary">None for everyone</Text>
              )}
            </TargetFact>
            {product.cost !== undefined ? (
              <TargetFact label="Cost" target={{kind: 'price', label: `${name} cost ${price(product.cost, product.price_currency)}`, ids: {price: 'cost', ...ids}}}>
                <Text hasTabularNumbers>{price(product.cost, product.price_currency)}</Text>
              </TargetFact>
            ) : null}
            <TargetFact label="Stock" target={{kind: 'stock', label: `${name} stock ${count(product.inventory_level)}`, ids, term: 'schema:inventoryLevel'}}>
              <Text weight="semibold" hasTabularNumbers>
                {count(product.inventory_level)}
              </Text>
            </TargetFact>
            {reorders.length > 0 ? (
              <TargetFact
                label="On order"
                target={{
                  kind: 'reorder',
                  label: `${name} on order: ${reorders.map(r => `${count(r.total_units)} arriving ${dateTime(r.expected_arrival_until)}`).join('; ')}`,
                  ids: {reorder_id: reorders[0].id, ...ids},
                  term: 'acom:Reorder',
                }}>
                <VStack gap={0}>
                  {reorders.map(r => (
                    <Text key={r.id} hasTabularNumbers>{`${count(r.total_units)} arriving by ${dateTime(r.expected_arrival_until)}`}</Text>
                  ))}
                </VStack>
              </TargetFact>
            ) : null}
          </MetadataList>
        </PageSection>
      ) : null}

      {reorders.length > 0 && canWrite ? (
        <PageSection title="On order" description="Reorders from the supplier that have not arrived. Receive one when it is here: its units become sellable stock.">
          <VStack gap={3}>
            {reorders.map(r => (
              <ReorderLine key={r.id} reorder={r} canReceive={canWrite} />
            ))}
          </VStack>
        </PageSection>
      ) : null}

      <PageSection
        title="Variants"
        count={variants.length ? `${count(variants.length)} shown` : undefined}
        description={canWrite ? 'Change a variant’s price or stock: preview, check, then apply. Price changes usually need a person’s approval.' : undefined}>
        {p.inventoryError ? <ErrorState error={p.inventoryError} title="Stock per variant could not be loaded." /> : null}
        <DataTable
          rows={variants}
          rowKey={v => v.variant_id}
          empty={{title: 'No variants'}}
          target={v => ({
            kind: 'variant_row',
            label: `${name} ${[v.size, v.color].filter(Boolean).join(' ')} · stock ${count(v.inventory_level)} · ${v.availability ?? ''}`,
            ids: {variant_id: v.variant_id, ...ids},
            term: 'schema:Product',
          })}
          columns={[
            {
              header: 'Variant',
              cell: v => (
                <VStack gap={0}>
                  <Text weight="medium">{[v.size, v.color].filter(Boolean).join(' · ') || v.variant_id}</Text>
                  <Text type="supporting" color="secondary">
                    {`${v.variant_id}${v.sku ? ` · ${v.sku}` : ''}`}
                  </Text>
                </VStack>
              ),
            },
            {header: 'Stock', cell: v => <Figure>{count(v.inventory_level)}</Figure>},
            {header: 'Safety stock', cell: v => <Figure muted>{count(v['acom:safety_stock'])}</Figure>},
            ...(reorders.length > 0 ? [{header: 'On order', cell: (v: InvVariant) => <Figure muted={!v.on_order}>{count(v.on_order ?? 0)}</Figure>}] : []),
            {header: 'Availability', cell: v => <StateToken state={v.availability} />},
            {
              header: 'Edit',
              cell: v => (
                <VStack gap={1} hAlign="start">
                  <Button label="Price" size="sm" onClick={() => setEditing({variant: v, mode: 'price'})} />
                  <Button label="Stock" size="sm" onClick={() => setEditing({variant: v, mode: 'stock'})} />
                </VStack>
              ),
            },
          ]}
        />
      </PageSection>

      {product?.additional_property?.length ? (
        <PageSection title="Properties">
          <MetadataList columns="multi">
            {product.additional_property.map((a, i) => (
              <TargetFact
                key={`${a.name}-${i}`}
                label={words(a.name)}
                target={{kind: 'property', label: `${a.name}: ${value(a.value)}${a.unit_code ? ` ${a.unit_code}` : ''}`, ids: {property: `${a.name}${a.size ? `_${a.size}` : ''}`, ...ids}}}>
                <Text>{`${value(a.value)}${a.unit_code ? ` ${a.unit_code}` : ''}${a.size ? ` (size ${a.size})` : ''}`}</Text>
              </TargetFact>
            ))}
          </MetadataList>
        </PageSection>
      ) : null}

      <PageSection title="Reviews">
        {p.reviewsError ? (
          p.reviewsError.code === 'no_reviews' ? (
            <Text color="secondary">No reviews yet.</Text>
          ) : (
            <ErrorState error={p.reviewsError} title="The reviews could not be summarized." />
          )
        ) : null}
        {p.reviews ? <Reviews r={p.reviews} name={name} /> : null}
      </PageSection>

      <PageSection title="Change history" description="Every change to this product, newest first.">
        {p.historyError ? <ErrorState error={p.historyError} title="The history could not be loaded." /> : null}
        {p.history ? (
          <DataTable
            rows={p.history}
            rowKey={h => h.action_id}
            empty={{title: 'No changes recorded'}}
            target={h => ({kind: 'history_row', label: `${dateTime(h.end_time)} ${h.change_type}: ${value(h.from)} → ${value(h.to)}`, ids: {action_id: h.action_id, ...ids}})}
            columns={[
              {header: 'When', cell: h => <Text>{dateTime(h.end_time)}</Text>},
              {header: 'Who', cell: h => <Text>{actor(h.agent)}</Text>},
              {header: 'Kind', cell: h => <StateToken state={h.change_type === 'structural' ? 'needs_approval' : 'gray'} label={words(h.change_type)} />},
              {header: 'Before', cell: h => <Text color="secondary">{value(h.from)}</Text>},
              {header: 'After', cell: h => <Text>{value(h.to)}</Text>},
            ]}
          />
        ) : null}
      </PageSection>

      {editing ? (
        <VariantEditor
          productName={name}
          variant={editing.variant}
          mode={editing.mode}
          onModeChange={mode => setEditing(e => (e ? {...e, mode} : e))}
          onClose={() => setEditing(null)}
          canWrite={canWrite}
        />
      ) : null}
    </PageFrame>
  );
}

function Reviews({r, name}: {r: ReviewSummary; name: string}) {
  const ids = {product_id: r.product_id};
  return (
    <VStack gap={3}>
      <TargetInline target={{kind: 'rating', label: `${name}: ${r.rating_value ?? '—'} from ${r.review_count} reviews`, ids, term: 'schema:AggregateRating'}}>
        <Text weight="semibold">{r.rating_value !== null ? `${r.rating_value} / 5` : 'No rating'}</Text>
        <Text color="secondary">{`from ${count(r.review_count)} reviews`}</Text>
      </TargetInline>
      <MetadataList columns="multi" title="Fit">
        {r.fit_votes.map(f => (
          <TargetFact key={f.fit} label={f.label} target={{kind: 'fit_vote', label: `${f.label}: ${f.count}`, ids: {fit: f.fit, ...ids}}}>
            <Text hasTabularNumbers>{count(f.count)}</Text>
          </TargetFact>
        ))}
      </MetadataList>
      {r.mentions.length > 0 ? (
        <HStack gap={1} wrap="wrap">
          {r.mentions.map(m => (
            <Token key={m.text} label={`“${m.text}” · ${m.count}`} size="sm" />
          ))}
        </HStack>
      ) : null}
      {r.sample.map(s => (
        <TargetInline
          key={s.review_id}
          target={{kind: 'review', label: `${s.review_rating}/5 ${s.excerpt}`, ids: {review_id: s.review_id, ...ids}, term: 'schema:Review'}}>
          <VStack gap={0.5}>
            <Blockquote>{s.excerpt}</Blockquote>
            <Text type="supporting" color="secondary">
              {[`${s.review_rating}/5`, s.size_bought ? `bought ${s.size_bought}` : null, s.height_cm ? `${s.height_cm} cm` : null, date(s.date_published)]
                .filter(Boolean)
                .join(' · ')}
            </Text>
          </VStack>
        </TargetInline>
      ))}
    </VStack>
  );
}
