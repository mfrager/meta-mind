# MetaMind Web Console — UI design plan (index)

> **What this is.** The complete, build-ready design plan for the **MetaMind Web Console**: a two-pane,
> chat-driven operator console for the Metamind cognitive kernel, modeled on (and an improvement over) the
> web UI of `~/Scripts/rust_symbolic`, templated on **shadcn blocks + Tailwind CSS v4**, and specified to a
> colorful, business-like, professional standard that does **not** overuse rounded corners.
>
> **Status:** design complete, awaiting build. **Target stack:** React 19 + Vite + TypeScript, Tailwind
> CSS v4, shadcn/ui (`base-nova`, Base UI primitives), served by a new `mm-serve` (axum) crate that embeds
> the kernel.
>
> **Reading order:** `00` → `01` → `02` → `03` → `04` → `05` → `06` → `07` → `08` → `09` → `10` → `11`.
> If you only read three: `02` (the styling law), `05` (the left pane), `08` (the chat pane).

---

## Documents

| # | File | What it specifies |
|---|---|---|
| 00 | [`00_context_and_objectives.md`](./00_context_and_objectives.md) | The rust_symbolic chat UI reviewed in detail (what to keep, what to fix), MetaMind context, objectives, scope, UX principles, personas, research foundation with locked decisions |
| 01 | [`01_architecture.md`](./01_architecture.md) | Workspace layout (`crates/mm-serve`, `frontend/`), the SPA rendering + routing architecture, state management, streaming transport, single-writer kernel discipline, config surface, wire-type generation |
| 02 | [`02_design_system.md`](./02_design_system.md) | **The styling law.** Tokens (oklch), the 10-hue semantic color taxonomy, typography, the radius policy (explicit anti-roundness rules + lint), elevation, motion, density, chart palette, dark mode, accessibility |
| 03 | [`03_shell_and_layout.md`](./03_shell_and_layout.md) | App shell: left menu (excluded from the 2-pane count), the 2-pane split (document pane ‖ chat pane), top command bar, status bar, resizable/collapsible behavior, keyboard map, responsive tiers |
| 04 | [`04_component_library.md`](./04_component_library.md) | The component library: shadcn block inventory plus the custom rich layer — cards, tabs, accordions, carousels, forms, tables, data-viz, feedback, overlays, status/provenance components — with prop-level specs and states |
| 05 | [`05_document_pane.md`](./05_document_pane.md) | The left pane: the Focus Document model, the document-kind registry (docs, charts, logs, tables, diffs, proofs, …), viewer/editor modes, document status semantics, live documents, chat⇄focus protocol |
| 06 | [`06_tree_editors.md`](./06_tree_editors.md) | The tree engine (virtualized, DnD, inline edit, undo/redo, tree-table mode) and the ten domain tree editors/viewers (ontology, goals, memory, modules, policies, decisions, provenance, change sets, library, budgets) |
| 07 | [`07_mind_map.md`](./07_mind_map.md) | The high-quality mind-map viewer: dual layout engines (radial tidy mind map + React Flow graph), node/edge language, interaction model, LOD rendering, chat-driven mapping, export, performance and a11y |
| 08 | [`08_chat_pane.md`](./08_chat_pane.md) | The right pane: transcript model with typed content blocks, turn lifecycle rails, tool/operation cards with per-op renderers, the rich input set (button grids, image chooser, chat forms, slash commands), the rich output set |
| 09 | [`09_api_and_streaming.md`](./09_api_and_streaming.md) | `mm-serve` route table, the resumable SSE protocol (sequence-numbered, reconnect-safe), the event taxonomy (extension of rust_symbolic's), wire types, error taxonomy, security posture |
| 10 | [`10_build_and_test.md`](./10_build_and_test.md) | Build sequence M0–M7 with exact deliverable paths and acceptance criteria, test strategy (unit/integration/e2e/visual/a11y/perf), CI additions, and the pass gate commands |
| 11 | [`11_risks_and_references.md`](./11_risks_and_references.md) | Risks and mitigations, external references (all with code), open questions with recorded defaults |

---

## The plan in one page

**The product.** A local-first operator console. A left **menu** (navigation, not counted as a pane), a
**left pane** that always shows the *current focus document* — whatever artifact the user is working with,
in viewer or editor mode: a document, a chart dashboard, a live log, a tree, a mind map, a diff, a proof —
and a **right pane** that is a rich chat log with the Metamind kernel. Everything the 42 CLI subcommands
can do is reachable from chat, the command palette, or the document pane; every artifact chat produces can
be focused into the left pane; every selection in the left pane can be sent to chat.

**The three deliberate improvements over the rust_symbolic UI:**

1. **Artifact-centric two panes.** rust_symbolic's side panel shows only the session model (Turtle +
   equations). MetaMind's left pane is a general *Focus Document* workspace: a registry of document kinds
   with viewer/editor modes, live status, versioning, provenance, and export. Chat and documents
   interoperate both ways.
2. **A real component library.** rust_symbolic has ~20 shadcn primitives and ad-hoc cards. This plan
   specifies a 70+-component catalog (primitives + rich custom blocks), a live `/dev/catalog` page, and
   strict tokens so the app reads as one designed system, not a kit of parts.
3. **Graphical richness.** Icon chips, phase rails, sparklines, gauges, budget meters, treemaps, sunburst
   and mind-map views, status color language, chart cards, run timelines — color used semantically and
   economically. Business-like, not toy-like.

**The styling law in five sentences.** Color is *semantic* (ten hues, each with one meaning), never
decorative. Structure is drawn with 1px borders and tinted surfaces, not shadows and not big radii.
**Corner radii are deliberately small**: 2px surfaces, 4px controls, 8px absolute maximum for chat
bubbles, pills only for chips/badges/toggles — `rounded-xl/2xl/3xl` is banned by lint (see
[`02_design_system.md` §5](./02_design_system.md#5-radius-policy--the-anti-roundness-law)). Type is Geist
(Geist Mono for values), with uppercase 10px micro-labels doing most of the "designed" work. Motion is
short (120–240ms) and always optional (`prefers-reduced-motion`).

**Locked decisions** (full table in `00` §5):

| Decision | Choice |
|---|---|
| UI framework | React 19 + Vite + TypeScript (SPA), matching the proven in-house rust_symbolic stack |
| Styling | Tailwind CSS v4 (CSS-first `@theme`), shadcn/ui `base-nova` (Base UI primitives), shadcn blocks as page templates |
| Charts | Recharts (shadcn `chart` wrapper) + custom SVG spark/gauge/radial + `@xyflow/react` for graphs |
| Mind map | Custom SVG radial engine on `d3-hierarchy` + React Flow graph mode |
| Trees | Custom `TreeCore` on `@tanstack/react-virtual` + `@dnd-kit` (full control over editor semantics) |
| Backend | New `crates/mm-serve` (axum) embedding the kernel; CLI remains the headless operator surface |
| Streaming | SSE with `X-Request-Id` + `?from=seq` resume (the rust_symbolic protocol, extended with `focus` and `input_request` frames) |
| Server state | TanStack Query; UI state: Zustand; transcript: pure reducer (rust_symbolic pattern) |

**Naming.** Crate: `mm-serve`. Frontend package: `metamind-console` (`frontend/`). The plan refers to the
product as the *Console* and the UI design language as *Structured Color*.

---

## Conventions used in these documents

- **Exact paths** are given as `crates/mm-serve/...` or `frontend/src/...` relative to the repo root.
- **TypeScript interfaces** are normative; the Rust mirrors are generated with `ts-rs` (see `01` §7).
- **Acceptance criteria** are the *pass gate* in `10` §7 — every milestone must clear it before the next.
- Where this plan departs from the rust_symbolic implementation, the departure is called out in a
  **“Δ vs rust_symbolic”** note so the improvement is traceable.
