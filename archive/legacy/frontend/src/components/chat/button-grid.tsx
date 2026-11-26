import { Button } from "@/components/ui/button.tsx";
import type { ButtonRequest } from "@/lib/transcript/reducer.ts";

export function ButtonGrid({
  request,
  answered,
  onAnswer,
}: {
  request: ButtonRequest;
  answered?: string;
  onAnswer: (value: string) => void;
}) {
  const columns = request.columns ?? 2;
  return (
    <div data-button-grid className="space-y-2">
      {request.prompt ? <p>{request.prompt}</p> : null}
      <div className="grid gap-2" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
        {request.options.map((option) => (
          <Button
            key={option.value}
            type="button"
            variant={answered === option.value ? "default" : "outline"}
            disabled={Boolean(answered)}
            onClick={() => onAnswer(option.value)}
          >
            {option.label}
          </Button>
        ))}
      </div>
    </div>
  );
}
