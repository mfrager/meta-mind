import useEmblaCarousel from "embla-carousel-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils.ts";

export function Carousel({ slides, label }: { slides: ReactNode[]; label: string }) {
  const [ref, api] = useEmblaCarousel({ align: "start" });
  return (
    <div data-token="carousel" aria-roledescription="carousel" aria-label={label}>
      <div ref={ref} className="overflow-hidden">
        <div className="flex">
          {slides.map((slide, index) => (
            <div key={index} className="min-w-0 shrink-0 grow-0 basis-4/5 pr-3" role="group" aria-roledescription="slide" aria-label={`${index + 1} of ${slides.length}`}>
              {slide}
            </div>
          ))}
        </div>
      </div>
      <div className="mt-2 flex justify-end gap-1">
        <button type="button" className="border-border h-7 rounded-md border px-2 text-small" aria-label="Previous slide" onClick={() => api?.scrollPrev()}>
          Prev
        </button>
        <button type="button" className="border-border h-7 rounded-md border px-2 text-small" aria-label="Next slide" onClick={() => api?.scrollNext()}>
          Next
        </button>
      </div>
    </div>
  );
}

export function FrameImage({ alt, caption }: { alt: string; caption?: string }) {
  return (
    <figure data-token="image">
      <div className="bg-muted grid h-24 place-items-center rounded-xs" role="img" aria-label={alt}>
        <span className="text-small text-muted-foreground">{alt}</span>
      </div>
      {caption ? <figcaption className="text-small text-muted-foreground mt-1">{caption}</figcaption> : null}
    </figure>
  );
}

export function ScreenshotFrame({ alt, caption }: { alt: string; caption: string }) {
  return (
    <figure data-token="screenshot-frame" className="border-border rounded-xs border-2 p-1">
      <FrameImage alt={alt} caption={caption} />
    </figure>
  );
}

export function carouselClass(active: boolean) {
  return cn(active && "ring-1 ring-azure-500");
}
