import { PageFrame } from "@/routes/frame.tsx";

export function WorkspacePlaceholder() {
  return (
    <PageFrame
      eyebrow="Metamind Console · Milestone 0"
      title="Workspace"
      lede="The two-pane workspace is reserved. The menu, split, document pane, and chat pane are later milestones."
    >
      <div className="border-border bg-card rounded-xs grid min-h-80 grid-cols-1 border md:grid-cols-[1fr_1fr]">
        <div className="border-border p-4 md:border-r">
          <p className="micro-label text-muted-foreground">Document pane</p>
          <p className="text-body mt-2">No focus document yet.</p>
        </div>
        <div className="p-4">
          <p className="micro-label text-muted-foreground">Chat pane</p>
          <p className="text-body mt-2">The transcript arrives with the chat pane.</p>
        </div>
      </div>
    </PageFrame>
  );
}
