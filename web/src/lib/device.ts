// The kind of device a request comes from, read from its User-Agent. The server knows nothing of
// the window's width, so without this it drew the wide layout and the compact one replaced it after
// hydration (the top bar's links, then the menu button; the filter panel beside the results, then
// folded). A desktop window made narrow still reads as a desktop, and so does an iPad that asks for
// the desktop site (iPadOS's default), so those keep the switch.

export type Device = 'phone' | 'tablet' | 'desktop';

export function deviceOf(userAgent: string | null | undefined): Device {
  const ua = userAgent ?? '';
  // Android phones say "Mobile"; Android tablets do not. An iPad's "Mobile/15E148" is not "Mobi".
  if (/iPhone|iPod|Windows Phone|Android.*Mobile|\bMobi\b/i.test(ua)) return 'phone';
  if (/iPad|Tablet|Android|Silk|Kindle/i.test(ua)) return 'tablet';
  return 'desktop';
}
