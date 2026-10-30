'use client';

// The product page's reference sections: measurements and fit (get_product_measurements), the
// review summary and reviews (get_review_summary, list_reviews), and the facts that describe the
// product (get_product additional_property). Each is an AI target so a shopper can ask about it.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {ProgressBar} from '@astryxdesign/core/ProgressBar';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {longDate, words} from '@/app/(storefront)/_lib/format';
import type {AdditionalProperty, Measurements, Review, ReviewSummary} from '@/app/(storefront)/_lib/types';

import {AskButton} from './Ask';

function MeasurementRow({productId, name, unit, sizes}: {productId: string; name: string; unit: string; sizes: Measurements['sizes']}) {
  const target = useAiTarget({kind: 'measurement', label: `${words(name)} (${unit})`, ids: {measurement: name, product_id: productId}});
  return (
    <TableRow ref={target.ref} {...target.props}>
      <TableCell>{words(name)}</TableCell>
      {sizes.map(s => (
        <TableCell key={s.size}>{s[name] === null || s[name] === undefined ? '—' : `${s[name]} ${unit}`}</TableCell>
      ))}
    </TableRow>
  );
}

export function MeasurementsTable({productName, measurements}: {productName: string; measurements: Measurements}) {
  const target = useAiTarget({
    kind: 'measurement_table',
    label: `${productName} measurements`,
    ids: {product_id: measurements.product_id},
    term: 'schema:QuantitativeValue',
  });
  return (
    <VStack gap={2} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" justify="between">
        <Text type="supporting" color="secondary">
          {measurements.how_measured ?? `Garment measurements in ${measurements.unit}.`}
        </Text>
        <AskButton target={target} label={`${productName} measurements`} />
      </HStack>
      <Table density="compact">
        <TableHeader>
          <TableRow>
            <TableHeaderCell>Size</TableHeaderCell>
            {measurements.sizes.map(s => (
              <TableHeaderCell key={s.size}>{s.size}</TableHeaderCell>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {measurements.measurements.map(m => (
            <MeasurementRow key={m} productId={measurements.product_id} name={m} unit={measurements.unit} sizes={measurements.sizes} />
          ))}
        </TableBody>
      </Table>
    </VStack>
  );
}

export function ReviewSummaryView({productName, summary}: {productName: string; summary: ReviewSummary}) {
  const target = useAiTarget({
    kind: 'review_summary',
    label: `${productName} reviews`,
    ids: {product_id: summary.product_id},
    term: 'schema:AggregateRating',
  });
  const votes = summary.fit_votes.reduce((n, v) => n + v.count, 0);
  return (
    <VStack gap={3} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" justify="between">
        <Text type="large" weight="semibold">
          {summary.rating_value !== null ? `${summary.rating_value} out of 5` : 'Not rated yet'}
          <Text type="body" color="secondary">
            {' '}
            · {summary.review_count === 1 ? '1 review' : `${summary.review_count} reviews`}
          </Text>
        </Text>
        <AskButton target={target} label={`${productName} reviews`} />
      </HStack>
      {votes > 0 ? (
        <VStack gap={2}>
          <Text type="label">Fit</Text>
          {summary.fit_votes.map(v => (
            <ProgressBar key={v.fit} label={`${v.label} · ${v.count}`} value={v.count} max={votes} />
          ))}
        </VStack>
      ) : null}
      {summary.mentions.length > 0 ? (
        <MetadataList title="People mention">
          {summary.mentions.map(m => (
            <MetadataListItem key={m.text} label={`${m.count}×`}>
              “{m.text}”
            </MetadataListItem>
          ))}
        </MetadataList>
      ) : null}
    </VStack>
  );
}

function ReviewRow({review}: {review: Review}) {
  const author = typeof review.author === 'string' ? review.author : (review.author?.name ?? 'A shopper');
  const target = useAiTarget({kind: 'review', label: `Review by ${author}`, ids: {review_id: review.id}, term: 'schema:Review'});
  const meta = [`${review.review_rating} out of 5`, author, review.size_bought ? `bought ${review.size_bought}` : null, review.date_published ? longDate(review.date_published) : null]
    .filter(Boolean)
    .join(' · ');
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={review.name ?? meta}
      description={
        <VStack gap={1}>
          {review.name ? (
            <Text type="supporting" color="secondary">
              {meta}
            </Text>
          ) : null}
          {review.review_body ? <Text>{review.review_body}</Text> : null}
        </VStack>
      }
    />
  );
}

export function ReviewList({reviews}: {reviews: Review[]}) {
  return (
    <List hasDividers density="spacious">
      {reviews.map(r => (
        <ReviewRow key={r.id} review={r} />
      ))}
    </List>
  );
}

function valueText(p: AdditionalProperty): string {
  const unit = p.unit_code ?? p.unit_text ?? '';
  const v = p.value;
  if (Array.isArray(v)) return `${v.join(' × ')}${unit ? ` ${unit}` : ''}`;
  if (typeof v === 'boolean') return v ? 'Yes' : 'No';
  if (typeof v === 'number') return `${v}${unit ? ` ${unit}` : ''}${p.size ? ` (size ${p.size})` : ''}`;
  if (typeof v === 'string') return words(v);
  return '';
}

export function ProductFacts({productId, facts}: {productId: string; facts: AdditionalProperty[]}) {
  const target = useAiTarget({kind: 'product_facts', label: 'Product details', ids: {product_id: productId}, term: 'schema:PropertyValue'});
  return (
    <VStack ref={target.ref} {...target.props}>
      <MetadataList>
        {facts.map(f => (
          <MetadataListItem key={`${f.name}:${f.size ?? ''}`} label={words(f.name)}>
            {valueText(f)}
          </MetadataListItem>
        ))}
      </MetadataList>
    </VStack>
  );
}
