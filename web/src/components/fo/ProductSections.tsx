'use client';

// A product's reference sections, on its page and on the canvas: details, measurements and fit,
// reviews, questions, delivery and returns. A section whose read failed says why.

import {Collapsible, CollapsibleGroup} from '@astryxdesign/core/Collapsible';
import {VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';

import {words} from '@/app/(storefront)/_lib/format';
import type {Loaded} from '@/app/(storefront)/_lib/loaded';
import type {Measurements, Product, ProductQuestion, ReturnPolicy, Review, ReviewSummary} from '@/app/(storefront)/_lib/types';
import type {Page} from '@/lib/api';

import {MeasurementsTable, ProductFacts, ReviewList, ReviewSummaryView} from './ProductDetails';
import {QuestionList} from './StoreContent';

/** Air between a section's content and the next section's title. */
const SECTION_BODY = {paddingBottom: 'var(--spacing-5)'};

export interface ProductReads {
  measurements: Loaded<Measurements>;
  summary: Loaded<ReviewSummary>;
  reviews: Loaded<Page<Review>>;
  policy: Loaded<ReturnPolicy>;
  questions: Loaded<Page<ProductQuestion>>;
}

export function ProductSections({product: p, reads}: {product: Product; reads: ProductReads}) {
  const {measurements, summary, reviews, policy, questions} = reads;
  const rp = policy.ok ? policy.data : null;
  const nonReturnable = rp ? (rp['acom:non_returnable_when'] ?? rp.non_returnable_when ?? []) : [];
  const sizeExchangeDays = rp ? (rp['acom:size_exchange_days'] ?? rp.size_exchange_days ?? null) : null;
  return (
    <CollapsibleGroup type="multiple" defaultValue={['details', 'fit', 'reviews', 'questions', 'returns']} hasDividers>
      {p.additional_property && p.additional_property.length > 0 ? (
        <Collapsible value="details" trigger={<Heading level={2}>Details</Heading>}>
          <div style={SECTION_BODY}>
          <ProductFacts productId={p.id} facts={p.additional_property} />
          </div>
        </Collapsible>
      ) : null}
      <Collapsible value="fit" trigger={<Heading level={2}>Measurements and fit</Heading>}>
        <div style={SECTION_BODY}>
        {measurements.ok ? (
          <MeasurementsTable productName={p.name} measurements={measurements.data} />
        ) : (
          <Text color="secondary">{measurements.status === 404 ? 'No measurements for this product yet.' : measurements.error.message}</Text>
        )}
        </div>
      </Collapsible>
      <Collapsible value="reviews" trigger={<Heading level={2}>Reviews</Heading>}>
        <div style={SECTION_BODY}>
        <VStack gap={4}>
          {summary.ok ? (
            <ReviewSummaryView productName={p.name} summary={summary.data} />
          ) : (
            <Text color="secondary">{summary.error.code === 'no_reviews' ? 'No reviews yet.' : summary.error.message}</Text>
          )}
          {reviews.ok && reviews.data.items.length > 0 ? <ReviewList reviews={reviews.data.items} /> : null}
        </VStack>
        </div>
      </Collapsible>
      <Collapsible value="questions" trigger={<Heading level={2}>{questions.ok && questions.data.total > 0 ? `Questions (${questions.data.total})` : 'Questions'}</Heading>}>
        <div style={SECTION_BODY}>
        {!questions.ok ? (
          <Text color="secondary">{questions.error.message}</Text>
        ) : questions.data.items.length === 0 ? (
          <Text color="secondary">No questions yet.</Text>
        ) : (
          <QuestionList questions={questions.data.items} />
        )}
        </div>
      </Collapsible>
      <Collapsible value="returns" trigger={<Heading level={2}>Delivery and returns</Heading>}>
        <div style={SECTION_BODY}>
        {rp ? (
          <MetadataList>
            <MetadataListItem label="Returns">Within {rp.merchant_return_days} days of delivery</MetadataListItem>
            <MetadataListItem label="Return shipping">{words(rp.return_fees)}</MetadataListItem>
            {sizeExchangeDays ? <MetadataListItem label="Size exchange">Within {sizeExchangeDays} days</MetadataListItem> : null}
            {nonReturnable.length > 0 ? <MetadataListItem label="Not returnable when">{nonReturnable.map(words).join(', ')}</MetadataListItem> : null}
          </MetadataList>
        ) : (
          <Text color="secondary">{policy.ok ? '' : policy.error.message}</Text>
        )}
        </div>
      </Collapsible>
    </CollapsibleGroup>
  );
}
