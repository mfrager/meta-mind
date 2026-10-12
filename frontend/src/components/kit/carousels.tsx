import { Carousel } from "@/components/ui/media.tsx";
import { Stepper } from "@/components/ui/navigation.tsx";

export function CardCarousel() {
  return (
    <Carousel
      label="Recent documents"
      slides={["Doctor", "Logs", "Budgets"].map((title) => (
        <div key={title} className="border-border h-24 rounded-xs border p-3">
          <p className="text-h3">{title}</p>
        </div>
      ))}
    />
  );
}

export function StepCarousel() {
  return (
    <div data-token="step-carousel">
      <Stepper steps={["Read", "Ask", "Focus"]} current={1} />
      <p className="text-small mt-2">Ask the kernel, then focus what it returns.</p>
    </div>
  );
}

export function ScreenshotCarousel() {
  return (
    <Carousel
      label="Captures"
      slides={["Gate", "Diff"].map((title) => (
        <figure key={title} className="border-border rounded-xs border-2 p-2">
          <div className="bg-muted grid h-20 place-items-center" role="img" aria-label={title}>
            {title}
          </div>
        </figure>
      ))}
    />
  );
}
