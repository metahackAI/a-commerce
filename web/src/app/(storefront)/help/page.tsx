import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {FaqGroups, NoticeList} from '@/components/fo/StoreContent';
import type {Page} from '@/lib/api';

import {words} from '../_lib/format';
import {load} from '../_lib/server';
import type {Faq, FaqSummary, NoticeSummary} from '../_lib/types';

export const metadata = {title: 'Help'};

type SearchParams = Promise<{topic?: string | string[]}>;

// The store's notices and FAQs as operators keep them (list_notices, list_faqs, get_faq for the
// answers). `?topic=shipping` narrows both to one section; popups and banners link here with it.
export default async function HelpPage({searchParams}: {searchParams: SearchParams}) {
  const raw = (await searchParams).topic;
  const topic = (Array.isArray(raw) ? raw[0] : raw)?.trim() || undefined;
  const [notices, faqList, allFaqs] = await Promise.all([
    load<Page<NoticeSummary>>('/v1/customer/notices', {category: topic, limit: 20}),
    load<Page<FaqSummary>>('/v1/customer/faqs', {category: topic, limit: 100}),
    // The section words for the topic chips, whatever the current topic is.
    topic ? load<Page<FaqSummary>>('/v1/customer/faqs', {limit: 100}) : null,
  ]);
  const faqs: Faq[] = faqList.ok
    ? await Promise.all(
        faqList.data.items.map(async f => {
          const one = await load<Faq>(`/v1/customer/faqs/${encodeURIComponent(f.id)}`);
          return one.ok ? one.data : f;
        }),
      )
    : [];
  const sectionSource = allFaqs ?? faqList;
  const sections = sectionSource.ok ? [...new Set(sectionSource.data.items.map(f => f.category).filter((c): c is string => !!c))] : [];

  return (
    <Layout
      height="auto"
      contentWidth={960}
      content={
        <LayoutContent padding={6}>
          <VStack gap={6}>
            <VStack gap={2}>
              <Heading level={1}>{topic ? `Help: ${words(topic)}` : 'Help'}</Heading>
              {sections.length > 0 ? (
                <HStack gap={1} wrap="wrap">
                  <Token label="All topics" href="/help" />
                  {sections.map(s => (
                    <Token key={s} label={words(s)} href={`/help?topic=${encodeURIComponent(s)}`} />
                  ))}
                </HStack>
              ) : null}
            </VStack>
            <VStack gap={3}>
              <Heading level={2}>Notices</Heading>
              {!notices.ok ? (
                <ErrorNotice error={notices.error} title="Notices could not be loaded." />
              ) : notices.data.items.length === 0 ? (
                <Text color="secondary">No notices here.</Text>
              ) : (
                <NoticeList notices={notices.data.items} />
              )}
            </VStack>
            <VStack gap={3}>
              <Heading level={2}>Questions people ask</Heading>
              {!faqList.ok ? (
                <ErrorNotice error={faqList.error} title="The FAQs could not be loaded." askAbout="I have a question about my order." />
              ) : faqs.length === 0 ? (
                <Text color="secondary">No questions in this section yet.</Text>
              ) : (
                <FaqGroups faqs={faqs} />
              )}
            </VStack>
          </VStack>
        </LayoutContent>
      }
    />
  );
}
