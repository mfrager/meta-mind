import { CodeCard, FeedCard, ListCard, MeterCard, ProofCard, ProseCard, ScreenshotCard, StatCard, VerdictCard } from "@/components/kit/cards.tsx";
import { ChartCard } from "@/components/kit/data.tsx";

export function CardsGallery() {
  return (
    <div data-token="cards-gallery" className="grid gap-3 md:grid-cols-2">
      <StatCard />
      <MeterCard />
      <ChartCard />
      <ListCard />
      <FeedCard />
      <ProseCard />
      <CodeCard />
      <VerdictCard />
      <ProofCard />
      <ScreenshotCard />
    </div>
  );
}
