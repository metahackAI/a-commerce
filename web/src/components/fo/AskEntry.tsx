'use client';

// The way in to the assistant from the main page: say what you are looking for and the full
// workspace (/ask) opens with the answer. The conversation lives in the layout's AiProvider, so
// it carries over to every page.

import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {ChatBubbleLeftEllipsisIcon} from '@heroicons/react/24/outline';
import {useRouter} from 'next/navigation';
import {useState} from 'react';

import {AILISM} from '@/components/brand/Ailism';
import {useAi} from '@/ai/AiProvider';
import {useAiTarget} from '@/ai/useAiTarget';

const STARTERS = ['A light jacket for spring trips', 'A gift for my dad, under $100', 'Will M fit me in the wool overcoat?'];

export function AskEntry({name}: {name?: string | null}) {
  const ai = useAi();
  const router = useRouter();
  const [text, setText] = useState('');
  const target = useAiTarget({kind: 'natural_input', label: 'Ask Ailism', ids: {input: 'home_ask'}});

  function send(message: string) {
    const m = message.trim();
    if (!m) return;
    setText('');
    void ai.send(m);
    router.push('/ask');
  }

  return (
    <Card padding={6}>
      <VStack gap={3}>
        <VStack gap={1}>
          <Heading level={2}>{name ? `What are you looking for today, ${name}?` : 'What are you looking for today?'}</Heading>
          <Text type="body" color="secondary">
            {`${AILISM.tagline} Ask in your own words: Ailism finds, compares and prepares; only you place the order.`}
          </Text>
        </VStack>
        <HStack gap={2} vAlign="end" ref={target.ref} {...target.props}>
          <StackItem size="fill">
            <TextInput
              label="Ask Ailism"
              isLabelHidden
              placeholder="Ask about fit, fabric, delivery, returns…"
              startIcon={ChatBubbleLeftEllipsisIcon}
              value={text}
              onChange={setText}
              onEnter={() => send(text)}
              width="100%"
              size="lg"
            />
          </StackItem>
          <Button label="Ask" variant="primary" size="lg" onClick={() => send(text)} isDisabled={!text.trim()} />
        </HStack>
        <HStack gap={2} wrap="wrap">
          {STARTERS.map(s => (
            <Button key={s} label={s} size="sm" variant="secondary" onClick={() => send(s)} />
          ))}
        </HStack>
      </VStack>
    </Card>
  );
}
