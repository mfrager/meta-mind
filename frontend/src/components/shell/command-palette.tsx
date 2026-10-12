import { useEffect, useState } from "react";
import { Command, useCommandState } from "cmdk";
import { useNavigate } from "react-router-dom";

import { COMMANDS, COMMAND_GROUPS, type ConsoleCommand } from "@/lib/commands.ts";
import { DOCUMENTS, hrefFor } from "@/lib/nav.ts";
import { modLabel } from "@/lib/shortcuts.ts";
import { useChatPending } from "@/lib/store/chat-pending.ts";
import { usePaneStore } from "@/lib/store/pane.ts";

export function CommandPalette() {
  const open = usePaneStore((state) => state.paletteOpen);
  const setOpen = usePaneStore((state) => state.setPaletteOpen);
  const navigate = useNavigate();
  const [search, setSearch] = useState("");

  useEffect(() => {
    if (open) setSearch("");
  }, [open]);

  const close = () => setOpen(false);

  const run = (command: ConsoleCommand) => {
    useChatPending.getState().queue(command.argv);
    usePaneStore.getState().setActivePane("chat");
    close();
    navigate(command.doc ? hrefFor(command.doc) : "/chat");
  };

  const ask = (text: string) => {
    const trimmed = text.trim();
    if (trimmed) useChatPending.getState().queue(trimmed);
    usePaneStore.getState().setActivePane("chat");
    close();
    navigate("/chat");
  };

  return (
    <Command.Dialog
      open={open}
      onOpenChange={setOpen}
      label="Command palette"
      vimBindings={false}
      overlayClassName="mm-overlay"
      contentClassName="mm-palette"
      className="flex max-h-[min(32rem,70vh)] flex-col"
    >
      <Command.Input
        value={search}
        onValueChange={setSearch}
        placeholder="Search or run a command…"
      />
      <Command.List>
        <Command.Empty>
          <button type="button" className="text-body w-full px-3 py-4 text-left" onClick={() => ask(search)}>
            No match — ask the kernel instead
          </button>
        </Command.Empty>
        {COMMAND_GROUPS.map((group) => (
          <Command.Group key={group} heading={group}>
            {COMMANDS.filter((command) => command.group === group).map((command) => (
              <Command.Item
                key={command.id}
                value={`${command.title} ${command.argv} ${command.summary}`}
                keywords={[command.id, command.argv]}
                data-command={command.id}
                onSelect={() => run(command)}
              >
                <span className="min-w-0 flex-1">
                  <span className="text-body block truncate">{command.title}</span>
                  <span className="text-small text-muted-foreground block truncate">{command.summary}</span>
                </span>
                <span className="text-muted-foreground font-mono text-value shrink-0">{command.argv}</span>
              </Command.Item>
            ))}
          </Command.Group>
        ))}
        <Command.Group heading="Navigation">
          {(["overview", "workspace", "catalog"] as const).map((id) => (
            <Command.Item
              key={id}
              value={`go ${DOCUMENTS[id].title}`}
              data-nav={id}
              onSelect={() => {
                close();
                navigate(hrefFor(id));
              }}
            >
              <span className="text-body">{DOCUMENTS[id].title}</span>
              <span className="micro-label text-muted-foreground ml-auto">{DOCUMENTS[id].section}</span>
            </Command.Item>
          ))}
          <Command.Item
            value="reset layout"
            onSelect={() => {
              usePaneStore.getState().resetLayout();
              close();
            }}
          >
            <span className="text-body">Reset layout</span>
          </Command.Item>
          <Command.Item
            value="keyboard shortcuts"
            onSelect={() => {
              close();
              usePaneStore.getState().setShortcutsOpen(true);
            }}
          >
            <span className="text-body">Keyboard shortcuts</span>
            <span className="text-muted-foreground ml-auto font-mono text-value">{modLabel()}+/</span>
          </Command.Item>
        </Command.Group>
        <AskItem onAsk={ask} />
      </Command.List>
    </Command.Dialog>
  );
}

function AskItem({ onAsk }: { onAsk: (text: string) => void }) {
  const search = useCommandState((state) => state.search);
  const label = search.trim() ? `Ask the kernel: ${search.trim()}` : "Ask the kernel…";
  return (
    <Command.Group heading="Ask" forceMount>
      <Command.Item value={`ask the kernel ${search}`} forceMount onSelect={() => onAsk(search)}>
        <span className="text-body">{label}</span>
      </Command.Item>
    </Command.Group>
  );
}
