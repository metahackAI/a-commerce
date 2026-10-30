import type {Metadata} from 'next';
import {Center} from '@astryxdesign/core/Center';

import {LoginPanel} from '@/components/fo/LoginPanel';

export const metadata: Metadata = {title: 'Sign in'};

/** Only paths inside the store, so a link can't send a shopper elsewhere after sign-in. */
function safeNext(next: string | string[] | undefined): string {
  const n = Array.isArray(next) ? next[0] : next;
  return n && n.startsWith('/') && !n.startsWith('//') ? n : '/';
}

export default async function LoginPage({searchParams}: {searchParams: Promise<Record<string, string | string[] | undefined>>}) {
  const {next} = await searchParams;
  return (
    <Center axis="both" padding={6}>
      <LoginPanel next={safeNext(next)} />
    </Center>
  );
}
