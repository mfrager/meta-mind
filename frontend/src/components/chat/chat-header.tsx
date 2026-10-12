import { Button } from "@/components/ui/button.tsx";
import { useSessionStore } from "@/lib/store/session.ts";

export function ChatHeader({
  requestId,
  onStop,
  onRegenerate,
  onBranch,
}: {
  requestId: string | null;
  onStop: () => void;
  onRegenerate: () => void;
  onBranch: () => void;
}) {
  const title = useSessionStore((state) => state.title);
  return (
    <header className="border-border flex flex-wrap items-center gap-2 border-b px-3 py-2">
      <div className="min-w-0 flex-1">
        <h1 className="text-sm font-medium">{title}</h1>
        <p className="text-muted-foreground text-xs">Metamind</p>
      </div>
      <span className="text-muted-foreground font-mono text-xs">{requestId ?? "no run"}</span>
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
