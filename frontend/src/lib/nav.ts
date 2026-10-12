import { useLocation } from "react-router-dom";

import type { Tone } from "@/lib/tone.ts";

export type NavSection = "Work" | "Explore" | "Build" | "Operate" | "Admin";

export type DocKind = "health" | "about" | "empty" | "unavailable" | "tree";

export type DocumentMeta = {
  id: string;
  title: string;
  crumb: [string, string];
  section: NavSection;
  tone: Tone;
  kind: DocKind;
  /** Verbatim reason when this surface is not built yet. */
  unavailable?: string;
};

const DOC = "This document arrives with the document pane.";

export const DOCUMENTS: Record<string, DocumentMeta> = {
  overview: {
    id: "overview",
    title: "Overview",
    crumb: ["Work", "Overview"],
    section: "Work",
    tone: "azure",
    kind: "health",
  },
  workspace: {
    id: "workspace",
    title: "Workspace",
    crumb: ["Work", "Workspace"],
    section: "Work",
    tone: "violet",
    kind: "empty",
  },
  pinned: {
    id: "pinned",
    title: "Pinned",
    crumb: ["Work", "Pinned"],
    section: "Work",
    tone: "amber",
    kind: "unavailable",
    unavailable: "Pinned documents arrive with the document pane.",
  },
  drafts: {
    id: "drafts",
    title: "Drafts",
    crumb: ["Work", "Drafts"],
    section: "Work",
    tone: "amber",
    kind: "unavailable",
    unavailable: "Drafts arrive with the document pane.",
  },
  being: {
    id: "being",
    title: "Being",
    crumb: ["Explore", "Being"],
    section: "Explore",
    tone: "rose",
    kind: "tree",
  },
  memory: {
    id: "memory",
    title: "Memory",
    crumb: ["Explore", "Memory"],
    section: "Explore",
    tone: "teal",
    kind: "tree",
  },
  epistemic: {
    id: "epistemic",
    title: "Epistemics",
    crumb: ["Explore", "Epistemics"],
    section: "Explore",
    tone: "amber",
    kind: "tree",
  },
  library: {
    id: "library",
    title: "Library",
    crumb: ["Explore", "Library"],
    section: "Explore",
    tone: "teal",
    kind: "tree",
  },
  ontology: {
    id: "ontology",
    title: "Ontology",
    crumb: ["Explore", "Ontology"],
    section: "Explore",
    tone: "cyan",
    kind: "tree",
  },
  changesets: {
    id: "changesets",
    title: "Change sets",
    crumb: ["Build", "Change sets"],
    section: "Build",
    tone: "amber",
    kind: "tree",
  },
  modules: {
    id: "modules",
    title: "Modules",
    crumb: ["Build", "Modules"],
    section: "Build",
    tone: "indigo",
    kind: "tree",
  },
  policies: {
    id: "policies",
    title: "Policies",
    crumb: ["Build", "Policies"],
    section: "Build",
    tone: "amber",
    kind: "tree",
  },
  pi: {
    id: "pi",
    title: "Pi",
    crumb: ["Build", "Pi"],
    section: "Build",
    tone: "violet",
    kind: "unavailable",
    unavailable: DOC,
  },
  regressions: {
    id: "regressions",
    title: "Regressions",
    crumb: ["Build", "Regressions"],
    section: "Build",
    tone: "red",
    kind: "unavailable",
    unavailable: DOC,
  },
  runs: {
    id: "runs",
    title: "Runs",
    crumb: ["Operate", "Runs"],
    section: "Operate",
    tone: "violet",
    kind: "unavailable",
    unavailable: "No run is recorded.",
  },
  logs: {
    id: "logs",
    title: "Logs",
    crumb: ["Operate", "Logs"],
    section: "Operate",
    tone: "slate",
    kind: "unavailable",
    unavailable: "The log viewer arrives with the document pane.",
  },
  provenance: {
    id: "provenance",
    title: "Provenance",
    crumb: ["Operate", "Provenance"],
    section: "Operate",
    tone: "slate",
    kind: "tree",
  },
  tools: {
    id: "tools",
    title: "Tools",
    crumb: ["Operate", "Tools"],
    section: "Operate",
    tone: "indigo",
    kind: "unavailable",
    unavailable: DOC,
  },
  budgets: {
    id: "budgets",
    title: "Budgets",
    crumb: ["Operate", "Budgets"],
    section: "Operate",
    tone: "amber",
    kind: "tree",
  },
  doctor: {
    id: "doctor",
    title: "Doctor",
    crumb: ["Operate", "Doctor"],
    section: "Operate",
    tone: "green",
    kind: "health",
  },
  settings: {
    id: "settings",
    title: "Settings",
    crumb: ["Admin", "Settings"],
    section: "Admin",
    tone: "slate",
    kind: "unavailable",
    unavailable: "Kernel settings are on the Settings page.",
  },
  catalog: {
    id: "catalog",
    title: "Catalog",
    crumb: ["Admin", "Catalog"],
    section: "Admin",
    tone: "azure",
    kind: "unavailable",
  },
  about: {
    id: "about",
    title: "About",
    crumb: ["Admin", "About"],
    section: "Admin",
    tone: "slate",
    kind: "about",
  },
  llm: {
    id: "llm",
    title: "LLM cache",
    crumb: ["Operate", "LLM cache"],
    section: "Operate",
    tone: "violet",
    kind: "unavailable",
    unavailable: "The LLM cache meter arrives with the chat pane.",
  },
};

export type NavItem = {
  id: string;
  href: string;
};

export const NAV_SECTIONS: { section: NavSection; ids: string[] }[] = [
  { section: "Work", ids: ["overview", "workspace", "pinned", "drafts"] },
  { section: "Explore", ids: ["being", "memory", "epistemic", "library", "ontology"] },
  { section: "Build", ids: ["changesets", "modules", "policies", "pi", "regressions"] },
  { section: "Operate", ids: ["runs", "logs", "provenance", "tools", "budgets", "doctor"] },
  { section: "Admin", ids: ["settings", "catalog", "about"] },
];

export function hrefFor(id: string): string {
  if (id === "overview") return "/";
  if (id === "workspace") return "/chat";
  if (id === "catalog") return "/dev/catalog";
  if (id === "settings") return "/settings";
  return `/?doc=${id}`;
}

export function useFocusedDocument(): DocumentMeta {
  const { pathname, search } = useLocation();
  if (pathname === "/dev/catalog") return DOCUMENTS.catalog;
  if (pathname === "/settings") return DOCUMENTS.settings;
  if (pathname.startsWith("/chat")) return DOCUMENTS.workspace;
  if (pathname.startsWith("/map/")) {
    const id = pathname.split("/")[2] ?? "";
    if (DOCUMENTS[id]) return DOCUMENTS[id];
  }
  const doc = new URLSearchParams(search).get("doc");
  if (doc && DOCUMENTS[doc]) return DOCUMENTS[doc];
  return DOCUMENTS.overview;
}
