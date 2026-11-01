'use client';

// The frame of every back-office page, after the Astryx `detail-page` and `table-page` templates:
// a header with the way back, the title, a line of facts and the page's actions; the content
// scrolls under it; an optional side panel holds supporting facts. A client component: the way
// back passes an icon component to Astryx's Icon, which a server page cannot hand to the client.
//
// Responsive contract (Astryx layout/responsive):
//   >1024  content | side panel 340
//   <=1024 the panel's facts follow the content in one column  (useMediaQuery)

import type {ReactNode} from 'react';
import {HStack, Layout, LayoutContent, LayoutHeader, LayoutPanel, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Icon} from '@astryxdesign/core/Icon';
import {ArrowLeftIcon} from '@heroicons/react/24/outline';
import {useMediaQuery} from '@astryxdesign/core/hooks';

export interface PageFrameProps {
  title: ReactNode;
  /** One line under the title: facts, chips. */
  meta?: ReactNode;
  description?: ReactNode;
  back?: {href: string; label: string};
  actions?: ReactNode;
  /** Filters or tabs under the title. */
  toolbar?: ReactNode;
  panel?: ReactNode;
  children: ReactNode;
  /** Wider content for dense tables. */
  width?: number;
}

export function PageFrame({title, meta, description, back, actions, toolbar, panel, children, width = 1200}: PageFrameProps) {
  const isNarrow = useMediaQuery('(max-width: 1024px)');
  const beside = panel && !isNarrow;
  return (
    <Layout
      height="fill"
      contentWidth={width}
      defaultHasDividers
      header={
        <LayoutHeader hasDivider padding={4}>
          <VStack gap={3}>
            <HStack gap={4} vAlign="start" wrap="wrap">
              <StackItem size="fill">
                <VStack gap={1}>
                  {back ? (
                    <Link href={back.href} color="secondary">
                      <HStack gap={1} vAlign="center">
                        <Icon icon={ArrowLeftIcon} size="sm" color="inherit" />
                        {back.label}
                      </HStack>
                    </Link>
                  ) : null}
                  <Heading level={1}>{title}</Heading>
                  {meta ? (
                    <HStack gap={2} vAlign="center" wrap="wrap">
                      {meta}
                    </HStack>
                  ) : null}
                  {description ? (
                    <Text type="supporting" color="secondary">
                      {description}
                    </Text>
                  ) : null}
                </VStack>
              </StackItem>
              {actions ? (
                <HStack gap={2} vAlign="center" wrap="wrap">
                  {actions}
                </HStack>
              ) : null}
            </HStack>
            {toolbar}
          </VStack>
        </LayoutHeader>
      }
      content={
        <LayoutContent role="main">
          <VStack gap={6} paddingBlock={2}>
            {children}
            {panel && !beside ? (
              <VStack gap={4} role="complementary">
                {panel}
              </VStack>
            ) : null}
          </VStack>
        </LayoutContent>
      }
      end={
        beside ? (
          <LayoutPanel width={340} padding={4} role="complementary">
            <VStack gap={4}>{panel}</VStack>
          </LayoutPanel>
        ) : undefined
      }
    />
  );
}

/** A titled section of a page. */
export function PageSection({
  title,
  count,
  actions,
  description,
  children,
}: {
  title: string;
  count?: ReactNode;
  actions?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <VStack gap={3} as="section">
      <HStack gap={2} vAlign="center" wrap="wrap">
        <StackItem size="fill">
          <HStack gap={2} vAlign="center">
            <Heading level={2}>{title}</Heading>
            {count !== undefined ? (
              <Text type="supporting" color="secondary">
                {count}
              </Text>
            ) : null}
          </HStack>
        </StackItem>
        {actions}
      </HStack>
      {description ? (
        <Text type="supporting" color="secondary">
          {description}
        </Text>
      ) : null}
      {children}
    </VStack>
  );
}
