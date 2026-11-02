'use client';

// The store's popups for this page (merchandising.md popup): `list_popups` gives a shopper only
// the ones open now, so the page never decides by date. The first by priority shows as a banner
// above the page; closing it keeps it away for the popup's `suppress_days` on this browser.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {useEffect, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import type {Popup} from '@/app/(storefront)/_lib/types';

const KEY = 'acom_popup_closed:';
const DAY_MS = 86_400_000;

function closedUntil(id: string): number {
  try {
    return Number(window.localStorage.getItem(KEY + id) ?? 0);
  } catch {
    return 0;
  }
}

function PopupBanner({popup, onClose}: {popup: Popup; onClose: () => void}) {
  const target = useAiTarget({kind: 'popup', label: popup.name, ids: {popup_id: popup.id}, term: 'schema:WebPageElement'});
  const text = popup.content.text ?? popup.name;
  return (
    <div ref={target.ref} {...target.props}>
      <Banner
        status="info"
        title={text}
        isDismissable
        dismissLabel={popup.suppress_days ? `Close for ${popup.suppress_days} days` : 'Close'}
        onDismiss={onClose}
        endContent={popup.content.url ? <Button label="Learn more" variant="secondary" size="sm" href={popup.content.url} /> : undefined}
      />
    </div>
  );
}

export function StorePopup({popups}: {popups: Popup[]}) {
  // Read the browser's closed list after mounting, so the server's markup and the first client
  // render agree.
  const [shown, setShown] = useState<Popup | null>(null);
  useEffect(() => {
    const now = Date.now();
    const open = [...popups]
      .sort((a, b) => (a.priority ?? 99) - (b.priority ?? 99))
      .find(p => closedUntil(p.id) < now && (p.content.text || p.name));
    setShown(open ?? null);
  }, [popups]);

  if (!shown) return null;
  const close = () => {
    try {
      window.localStorage.setItem(KEY + shown.id, String(Date.now() + Math.max(shown.suppress_days ?? 1, 1) * DAY_MS));
    } catch {
      // Storage is off: close for this page view only.
    }
    setShown(null);
  };
  return <PopupBanner popup={shown} onClose={close} />;
}
