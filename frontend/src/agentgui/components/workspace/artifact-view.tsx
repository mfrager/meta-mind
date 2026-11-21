"use client";

import { CodeView } from "@/agentgui/components/workspace/code-view";
import { ChartView, TableView } from "@/agentgui/components/workspace/data-view";
import { DocumentView } from "@/agentgui/components/workspace/document-view";
import { FilesView } from "@/agentgui/components/workspace/files-view";
import { PlanView } from "@/agentgui/components/workspace/plan-view";
import { PreviewView } from "@/agentgui/components/workspace/preview-view";
import { TerminalView } from "@/agentgui/components/workspace/terminal-view";
import type { Artifact } from "@/agentgui/types/workspace";

export function ArtifactView({ artifact }: { artifact: Artifact }) {
  switch (artifact.kind) {
    case "document":
      return <DocumentView artifact={artifact} />;
    case "code":
    case "json":
      return <CodeView artifact={artifact} />;
    case "table":
      return <TableView artifact={artifact} />;
    case "chart":
      return <ChartView artifact={artifact} />;
    case "terminal":
      return <TerminalView artifact={artifact} />;
    case "plan":
      return <PlanView artifact={artifact} />;
    case "files":
      return <FilesView artifact={artifact} />;
    case "preview":
      return <PreviewView artifact={artifact} />;
    default:
      return <DocumentView artifact={artifact} />;
  }
}
