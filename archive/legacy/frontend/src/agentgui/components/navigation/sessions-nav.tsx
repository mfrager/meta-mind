"use client";

import { Button } from "@/agentgui/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/agentgui/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import { Input } from "@/agentgui/components/ui/input";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn, formatRelative } from "@/agentgui/lib/utils";
import type { Session } from "@/agentgui/types/workspace";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ChevronRightIcon,
  EllipsisIcon,
  PencilIcon,
  PinIcon,
  PinOffIcon,
  PlusIcon,
  Trash2Icon,
} from "lucide-react";
import { useMemo, useState } from "react";

function matches(session: Session, query: string) {
  if (!query) return true;
  const needle = query.toLowerCase();
  return (
    session.title.toLowerCase().includes(needle) ||
    (session.summary ?? "").toLowerCase().includes(needle) ||
    (session.tags ?? []).some((tag) => tag.toLowerCase().includes(needle))
  );
}

interface SessionRowProps {
  session: Session;
  active: boolean;
  compact: boolean;
  onSelect: () => void;
  onRename: () => void;
}

function SessionRow({
  session,
  active,
  compact,
  onSelect,
  onRename,
}: SessionRowProps) {
  const archiveSession = useWorkspaceStore((state) => state.archiveSession);
  const deleteSession = useWorkspaceStore((state) => state.deleteSession);
  const togglePinSession = useWorkspaceStore(
    (state) => state.togglePinSession
  );

  return (
    <div
      className={cn(
        "group/row relative flex items-start gap-2 rounded-lg px-2 py-1.5 transition-colors",
        active
          ? "active-nav-item"
          : "hover:bg-sidebar-accent/60 hover-lift border border-transparent"
      )}
    >
      <button
        className="min-w-0 flex-1 text-left"
        onClick={onSelect}
        type="button"
      >
        <div className="flex items-center gap-1.5">
          <span
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              session.status === "active"
                ? "bg-primary"
                : active
                  ? "bg-positive"
                  : "bg-muted-foreground/35"
            )}
          />
          <span
            className={cn(
              "truncate text-[14px] leading-tight",
              active ? "font-medium text-foreground" : "text-foreground/85"
            )}
          >
            {session.title}
          </span>
          {session.pinned && (
            <PinIcon className="size-2.5 shrink-0 text-primary/70" />
          )}
        </div>
        {!compact && (
          <div className="mt-1 flex items-center gap-1.5 pl-3">
            <span className="truncate text-[12px] text-muted-foreground">
              {session.summary ?? `${session.messageCount} messages`}
            </span>
            <span className="shrink-0 font-mono text-[11px] text-muted-foreground/70">
              {formatRelative(session.updatedAt)}
            </span>
          </div>
        )}
      </button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label="Session actions"
            className="mt-0.5 size-5 shrink-0 rounded-md opacity-0 transition-opacity group-hover/row:opacity-100 data-[state=open]:opacity-100"
            size="icon-xs"
            variant="ghost"
          >
            <EllipsisIcon className="size-3" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem onClick={onRename}>
            <PencilIcon className="size-3.5" /> Rename
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => togglePinSession(session.id)}>
            {session.pinned ? (
              <>
                <PinOffIcon className="size-3.5" /> Unpin
              </>
            ) : (
              <>
                <PinIcon className="size-3.5" /> Pin
              </>
            )}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => archiveSession(session.id)}>
            {session.status === "archived" ? (
              <>
                <ArchiveRestoreIcon className="size-3.5" /> Restore
              </>
            ) : (
              <>
                <ArchiveIcon className="size-3.5" /> Archive
              </>
            )}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => deleteSession(session.id)}
            variant="destructive"
          >
            <Trash2Icon className="size-3.5" /> Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

function SessionGroup({
  id,
  label,
  sessions,
  compact,
  activeId,
  defaultOpen,
  onRename,
}: {
  id: string;
  label: string;
  sessions: Session[];
  compact: boolean;
  activeId: string;
  defaultOpen?: boolean;
  onRename: (session: Session) => void;
}) {
  const { expandedGroups, toggleGroup, selectSession } = useWorkspaceStore();
  const open = expandedGroups[id] ?? defaultOpen ?? false;

  if (sessions.length === 0) return null;

  return (
    <div className="mb-1">
      <button
        className="flex w-full items-center gap-1 px-2 py-1 text-[11px] font-medium tracking-wider text-muted-foreground/80 uppercase transition-colors hover:text-foreground"
        onClick={() => toggleGroup(id)}
        type="button"
      >
        <ChevronRightIcon
          className={cn("size-3 transition-transform", open && "rotate-90")}
        />
        {label}
        <span className="ml-auto font-mono text-[11px] normal-case opacity-70">
          {sessions.length}
        </span>
      </button>
      {open &&
        sessions.map((session) => (
          <SessionRow
            active={session.id === activeId}
            compact={compact}
            key={session.id}
            onRename={() => onRename(session)}
            onSelect={() => selectSession(session.id)}
            session={session}
          />
        ))}
    </div>
  );
}

export function SessionsNav({ compact }: { compact: boolean }) {
  const sessions = useWorkspaceStore((state) => state.sessions);
  const activeSessionId = useWorkspaceStore((state) => state.activeSessionId);
  const createSession = useWorkspaceStore((state) => state.createSession);
  const query = useWorkspaceStore((state) => state.searchQuery);
  const [renaming, setRenaming] = useState<Session | null>(null);
  const [draft, setDraft] = useState("");
  const renameSession = useWorkspaceStore((state) => state.renameSession);

  const filtered = useMemo(
    () => sessions.filter((session) => matches(session, query)),
    [sessions, query]
  );

  const pinned = filtered.filter(
    (session) => session.pinned && session.status !== "archived"
  );
  const recent = filtered.filter(
    (session) => !session.pinned && session.status !== "archived"
  );
  const archived = filtered.filter((session) => session.status === "archived");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!compact && (
        <div className="flex items-center justify-between px-2 pb-2">
          <span className="text-[11px] font-medium tracking-wider text-muted-foreground uppercase">
            Conversations
          </span>
          <Button
            className="h-6 gap-1 px-1.5 text-[12px]"
            onClick={() => createSession()}
            size="sm"
            variant="ghost"
          >
            <PlusIcon className="size-3" /> New
          </Button>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-1">
        <SessionGroup
          activeId={activeSessionId}
          compact={compact}
          defaultOpen
          id="pinned"
          label="Pinned"
          onRename={(session) => {
            setRenaming(session);
            setDraft(session.title);
          }}
          sessions={pinned}
        />
        <SessionGroup
          activeId={activeSessionId}
          compact={compact}
          defaultOpen
          id="recent"
          label="Recent"
          onRename={(session) => {
            setRenaming(session);
            setDraft(session.title);
          }}
          sessions={recent}
        />
        <SessionGroup
          activeId={activeSessionId}
          compact={compact}
          id="archived"
          label="Archived"
          onRename={(session) => {
            setRenaming(session);
            setDraft(session.title);
          }}
          sessions={archived}
        />
        {filtered.length === 0 && !compact && (
          <p className="px-2 py-6 text-[13px] text-muted-foreground">
            {query ? `No sessions match “${query}”.` : "No conversations yet."}
          </p>
        )}
      </div>

      <Dialog
        onOpenChange={(open) => !open && setRenaming(null)}
        open={renaming !== null}
      >
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>Rename session</DialogTitle>
          </DialogHeader>
          <Input
            autoFocus
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && renaming && draft.trim()) {
                renameSession(renaming.id, draft.trim());
                setRenaming(null);
              }
            }}
            value={draft}
          />
          <DialogFooter>
            <Button onClick={() => setRenaming(null)} variant="ghost">
              Cancel
            </Button>
            <Button
              onClick={() => {
                if (renaming && draft.trim()) {
                  renameSession(renaming.id, draft.trim());
                  setRenaming(null);
                }
              }}
            >
              Save
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
