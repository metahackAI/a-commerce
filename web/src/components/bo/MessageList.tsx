'use client';

// What the store sent a member (list_notifications): when, how, what it said, and what it was
// about. A sent message cannot be taken back, so this is the record of what customers were told.

import {VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';

import {dateTime, words} from '@/app/bo/_lib/format';
import {entityHref} from '@/app/bo/_lib/links';
import type {StoreMessage} from '@/app/bo/_lib/types';

import {DataTable} from './tables';

export function MessageList({
  messages,
  customerId,
  empty = 'The store has not sent this member anything.',
}: {
  messages: StoreMessage[];
  customerId: string;
  /** What an empty list means here. */
  empty?: string;
}) {
  return (
    <DataTable
      rows={messages}
      rowKey={m => m.id}
      empty={{title: 'No messages', description: empty}}
      target={m => ({
        kind: 'message',
        label: `Message to ${customerId} on ${dateTime(m.date_sent)}: ${m.headline}`,
        ids: {message_id: m.id, customer_id: customerId, ...(m.about ? {about: m.about} : {})},
        term: 'schema:Message',
      })}
      columns={[
        {header: 'Sent', cell: m => <Text>{dateTime(m.date_sent)}</Text>},
        {
          header: 'Message',
          cell: m => (
            <VStack gap={0.5}>
              <Text weight="medium">{m.headline}</Text>
              <Text type="supporting" color="secondary">
                {m.text}
              </Text>
            </VStack>
          ),
        },
        {header: 'How', cell: m => <Text>{words(m.channel)}</Text>},
        {
          header: 'About',
          cell: m => {
            if (!m.about) return <Text color="secondary">—</Text>;
            const href = entityHref(m.about);
            return href ? <Link href={href}>{m.about}</Link> : <Text>{m.about}</Text>;
          },
        },
      ]}
    />
  );
}
