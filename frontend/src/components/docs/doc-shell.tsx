import { useEffect, useState } from "react";

import { chartCsv, chartPng, download, type ChartSeries } from "@/components/charts/export.ts";
import { useChatRefs } from "@/lib/store/chat-refs.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { DocFooter } from "@/components/docs/doc-footer.tsx";
import { DocHeader } from "@/components/docs/doc-header.tsx";
import { HistoryDrawer, LineageDrawer } from "@/components/docs/history-drawer.tsx";
import { SaveChangeset } from "@/components/docs/save-changeset.tsx";
import { DocBody, rawSnapshot } from "@/components/docs/viewers.tsx";
import { Button } from "@/components/ui/actions.tsx";
import { fetchProvenance, saveDraft } from "@/lib/api/focus.ts";
import { useFocusChannel } from "@/lib/api/focus-channel.ts";
import { activeDocument, useDocuments } from "@/lib/store/documents.ts";
import { useFocusStore } from "@/lib/store/focus.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

function draftFrom(doc: FocusDoc) {
  const snapshot = (doc.snapshot ?? {}) as { markdown?: string; text?: string; title?: string };
  return snapshot.markdown ?? snapshot.text ?? snapshot.title ?? "";
}

export function DocShell() {
  const doc = useDocuments((state) => activeDocument(state));
  const docs = useDocuments((state) => state.docs);
  const open = useDocuments((state) => state.open);
  const close = useDocuments((state) => state.close);
  const cycle = useDocuments((state) => state.cycle);
  const historyOpen = useDocuments((state) => state.historyOpen);
  const lineageOpen = useDocuments((state) => state.lineageOpen);
  const wizardOpen = useDocuments((state) => state.wizardOpen);
  const setHistory = useDocuments((state) => state.setHistory);
  const setLineage = useDocuments((state) => state.setLineage);
  const setWizard = useDocuments((state) => state.setWizard);
  const mode = useFocusStore((state) => state.mode);
  const liveId = doc?.capabilities.includes("live") ? doc.id : null;
  useFocusChannel(liveId, doc?.version ?? 0);
  const [lens, setLens] = useState<"body" | "raw">("body");
  const [draft, setDraft] = useState("");
  const [dirty, setDirty] = useState(0);
  const [createdBy, setCreatedBy] = useState("nav");
  const [events, setEvents] = useState(0);
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!doc) return;
    setDraft(draftFrom(doc));
    setDirty(0);
    setLens("body");
    setWizard(false);
  }, [doc, setWizard]);

  useEffect(() => {
    if (!doc) return;
    void fetchProvenance(doc.id).then((value) => {
      setCreatedBy(value.createdBy);
      setEvents(value.events);
    }).catch(() => undefined);
  }, [doc]);

  useEffect(() => {
    const onSave = () => {
      if (doc && (doc.snapshot as { production?: boolean } | null)?.production) setWizard(true);
    };
    const onClose = () => {
      if (doc) close(doc.id);
    };
    const onCycle = () => cycle();
    window.addEventListener("mm-save-draft", onSave);
    window.addEventListener("mm-close-tab", onClose);
    window.addEventListener("mm-quick-switch", onCycle);
    return () => {
      window.removeEventListener("mm-save-draft", onSave);
      window.removeEventListener("mm-close-tab", onClose);
      window.removeEventListener("mm-quick-switch", onCycle);
    };
  }, [close, cycle, doc, setWizard]);

  if (!doc) return null;
  const editing = mode === "edit" && doc.capabilities.includes("edit");

  async function onExport(format: "json" | "csv" | "png" | "md") {
    const snapshot = (doc!.snapshot ?? {}) as { series?: ChartSeries[]; note?: string; markdown?: string };
    if (format === "csv" && snapshot.series) {
      download(`${doc!.id}.csv`, new Blob([chartCsv(snapshot.series)], { type: "text/csv" }));
      return;
    }
    if (format === "png" && snapshot.series) {
      const blob = await chartPng(snapshot.series, snapshot.note ?? doc!.id);
      if (blob) download(`${doc!.id}.png`, blob);
      return;
    }
    if (format === "md" && snapshot.markdown) {
      download(`${doc!.id}.md`, new Blob([snapshot.markdown], { type: "text/markdown" }));
      return;
    }
    download(`${doc!.id}.json`, new Blob([JSON.stringify(doc, null, 2)], { type: "application/json" }));
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-doc-shell>
      <div className="border-border flex items-center gap-2 border-b px-2 py-1">
        <div className="flex min-w-0 flex-1 gap-1 overflow-auto">
          {docs.map((item) => (
            <button
              key={item.id}
              type="button"
              data-doc-tab={item.id}
              aria-current={item.id === doc.id ? "page" : undefined}
              className="text-small hover:bg-muted max-w-40 truncate rounded-sm px-2 py-1"
              onClick={() => open(item)}
            >
              {item.title}
            </button>
          ))}
        </div>
        <Button size="xs" variant="ghost" data-export-bundle onClick={() => {
          const bundle = docs.map((item) => ({ id: item.id, title: item.title, kind: item.kind, status: item.status, reason: item.reason ?? null }));
          download("audit-bundle.json", new Blob([JSON.stringify(bundle, null, 2)], { type: "application/json" }));
        }}>
          Export open documents
        </Button>
        <Button size="xs" variant="ghost" onClick={() => setLens(lens === "raw" ? "body" : "raw")}>
          {lens === "raw" ? "View" : "Raw"}
        </Button>
      </div>
      <div className="border-border border-b px-3 py-2">
        <DocHeader
          doc={doc}
          onAsk={() => {
            useChatRefs.getState().add(`@doc:${doc.id}`);
            usePaneStore.getState().setActivePane("chat");
          }}
        />
      </div>
      <div className="pane-scroll min-h-0 flex-1 overflow-auto">
        {notice ? <p className="text-small px-4 pt-3 font-mono" data-changeset-id>{notice}</p> : null}
        {wizardOpen ? (
          <div className="p-4">
            <SaveChangeset
              doc={doc}
              content={draft}
              onClose={() => setWizard(false)}
              onSaved={(saved, id) => {
                open(saved);
                setWizard(false);
                setNotice(id);
              }}
            />
          </div>
        ) : null}
        {lens === "raw" ? <div className="p-4">{rawSnapshot(doc)}</div> : <DocBody doc={doc} editing={editing} draft={draft} onDraft={(value) => { setDraft(value); setDirty(1); }} />}
        <HistoryDrawer id={doc.id} open={historyOpen} />
        <LineageDrawer open={lineageOpen} events={events} createdBy={createdBy} />
      </div>
      <DocFooter
        doc={doc}
        dirty={dirty}
        onSaveDraft={() => {
          if (!doc) return;
          void saveDraft(doc.id, draft)
            .then((saved) => {
              open(saved);
              setDirty(0);
            })
            .catch((error: unknown) => {
              setNotice(error instanceof Error ? error.message : "The draft was not saved.");
            });
        }}
        onChangeset={() => setWizard(true)}
        onHistory={() => setHistory(true)}
        onLineage={() => setLineage(true)}
        onExport={(format) => void onExport(format)}
      />
    </div>
  );
}
