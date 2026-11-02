import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';

import {longDate, words} from '../../../_lib/format';
import {load} from '../../../_lib/server';
import type {Notice} from '../../../_lib/types';

type Params = Promise<{notice_id: string}>;

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = decodeURIComponent((await params).notice_id);
  const notice = await load<Notice>(`/v1/customer/notices/${encodeURIComponent(id)}`);
  return {title: notice.ok ? notice.data.headline : 'Notice'};
}

// One store notice (get_notice). A shopper gets only published notices; a draft is not found.
export default async function NoticePage({params}: {params: Params}) {
  const id = decodeURIComponent((await params).notice_id);
  const notice = await load<Notice>(`/v1/customer/notices/${encodeURIComponent(id)}`);
  return (
    <Layout
      height="auto"
      contentWidth={960}
      content={
        <LayoutContent padding={6}>
          {!notice.ok ? (
            <ErrorNotice error={notice.error} title="This notice is not available." />
          ) : (
            <VStack gap={4}>
              <Breadcrumbs variant="supporting">
                <BreadcrumbItem href="/help">Help</BreadcrumbItem>
                {notice.data.category ? (
                  <BreadcrumbItem href={`/help?topic=${encodeURIComponent(notice.data.category)}`}>{words(notice.data.category)}</BreadcrumbItem>
                ) : null}
                <BreadcrumbItem isCurrent>{notice.data.headline}</BreadcrumbItem>
              </Breadcrumbs>
              <Heading level={1}>{notice.data.headline}</Heading>
              {notice.data.date_published ? <Text color="secondary">{longDate(notice.data.date_published)}</Text> : null}
              {notice.data.article_body ? <Text>{notice.data.article_body}</Text> : null}
            </VStack>
          )}
        </LayoutContent>
      }
    />
  );
}
