import { Button } from "@/components/ui/button.tsx";

export function ChatHeader({
  title,
  requestId,
  onStop,
  onRegenerate,
  onBranch,
}: {
  title: string;
  requestId: string | null;
  onStop: () => void;
  onRegenerate: () => void;
  onBranch: () => void;
}) {
  return (
    <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
      <h2 className="text-sm font-medium">{title}</h2>
      <span className="font-mono text-xs text-muted-foreground">{requestId ?? "no run"}</span>
      <Button type="button" variant="outline" onClick={onStop} disabled={!requestId}>
        Stop
      </Button>
      <Button type="button" variant="outline" onClick={onRegenerate} disabled={!requestId}>
        Regenerate
      </Button>
      <Button type="button" variant="outline" onClick={onBranch} disabled={!requestId}>
        Branch
      </Button>
    </header>
  );
}
