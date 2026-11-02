import HelpPage from '../../help/page';

type Params = Promise<{topic: string}>;

export const metadata = {title: 'Help'};

// Store pages that popups and banners link to (`/pages/shipping`) are the help section's topics:
// the notices and FAQs operators keep under that word.
export default async function StorePage({params}: {params: Params}) {
  const topic = decodeURIComponent((await params).topic);
  return <HelpPage searchParams={Promise.resolve({topic})} />;
}
