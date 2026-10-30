'use client';

// Tabs that are views of one page, kept in the URL so the server page reads the right tool.

import {useRouter} from 'next/navigation';
import {Tab, TabList} from '@astryxdesign/core/TabList';

export function NavTabs({value, tabs}: {value: string; tabs: {value: string; label: string; href: string}[]}) {
  const router = useRouter();
  return (
    <TabList value={value} onChange={v => router.push(tabs.find(t => t.value === v)?.href ?? '#')} hasDivider isFullBleed>
      {tabs.map(t => (
        <Tab key={t.value} value={t.value} label={t.label} />
      ))}
    </TabList>
  );
}
