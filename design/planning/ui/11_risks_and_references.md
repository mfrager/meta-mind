# 11 — Risks, references, and open questions

> Parent: [`INDEX.md`](./INDEX.md) · Back: [`10_build_and_test.md`](./10_build_and_test.md)

---

## 1. Risks and mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | **RocksDB/SQLite single-writer**: console and CLI contend for the data dir | High | High | Lock file + `locked` error code + read-only CLI rule (`01` §5.1); `doctor` preflight; banner in UI |
| R2 | **Scope creep of the component library** delays the workspace | Medium | High | The library is milestone-gated (M2) and budgeted (`04` §6 richness rules); catalog-first workflow; unregistered ops fall back to generic renderers |
| R3 | **Transcript determinism drift** between live SSE and replay | Medium | High | One reducer for both paths (`08` §2), golden fixtures shared with Rust (`09` §4), replay tests in CI (`10` §5.3) |
| R4 | **Mind-map performance** with 5k+ nodes | Medium | Medium | LOD + culling + worker layouts + "simplify" affordance (`07` §8); perf tests fail CI |
| R5 | **Tree editing violates kernel invariants** (e.g. invariants, production tree) | Medium | High | API-enforced promotion rule (409 `conflict`), read-only `capabilities` per node, typed reason fields, undo/journal on every op (`06` §4) |
| R6 | **Provider gaps** (no multimodal images, no structured decoding) degrade chat inputs | High | Medium | Honest pre-send degradation notes (`08` §5.4), attachment fallback with captions, `unavailable` chips with reasons |
| R7 | **SSE flakiness** (proxy buffering, dropped tails) | Medium | Medium | Heartbeats + backoff + `?from=` resume + detached-with-Resync state (`09` §3.1); fault-injection e2e test |
| R8 | **Bundle bloat** (Monaco, Recharts, xyflow, KaTeX) | Medium | Medium | Route-level lazy loading + worker offloading, bundle budget test (`10` §10.5), Monaco loaded only for `code` docs |
| R9 | **Radius/styling drift** as contributors add screens | High | Low | Token-only styling, lint bans (`02` §5.3), catalog + visual regression, `04` §1 composition rules |
| R10 | **A11y gaps in custom trees/maps** | Medium | Medium | WAI-ARIA patterns + keyboard conformance suite + hidden outline mirrors (`06` §8, `07` §7), axe in CI |
| R11 | **Wall-clock coupling**: live docs freeze during kernel stalls | Medium | Low | `stale` detection + last-frame age in headers (`05` §4), refresh affordances, never fake progress |
| R12 | **Chat orchestrator scope** (planning intent → ops) grows into an agent rewrite | Medium | High | Deterministic-first planner (`01` §5), bounded op vocabulary, `/api/exec` fallback keeps UI complete regardless |

---

## 2. External references (all with code)

| Reference | URL | Used for |
|---|---|---|
| shadcn/ui + blocks | https://ui.shadcn.com · https://ui.shadcn.com/blocks | Templates for shell, cards, dashboards, forms (`04` §3) |
| Tailwind CSS v4 | https://tailwindcss.com | `@theme` tokens, CSS-first config (`02`) |
| Base UI (`@base-ui/react`) | https://base-ui.com | Accessible primitives behind `base-nova` style |
| lucide-react | https://github.com/lucide-icons/lucide | Icon set (`02` §7) |
| Geist fonts | https://vercel.com/font (via `@fontsource-variable/geist`) | UI + mono type (`02` §4) |
| TanStack Query / Virtual | https://tanstack.com | Server state, virtualized rows (`01` §4, `05` §8, `06` §2) |
| Zustand | https://github.com/pmndrs/zustand | UI state slices (`01` §4) |
| dnd-kit | https://github.com/clauderic/dnd-kit | Tree drag/drop with validation (`06` §2) |
| d3-hierarchy | https://github.com/d3/d3-hierarchy | Radial/tidy/partition/treemap layouts (`06` §3.3, `07` §3) |
| React Flow (xyflow) | https://github.com/xyflow/xyflow | Graph engine, minimap, custom nodes (`07` §4) |
| elkjs | https://github.com/kieler/elkjs | Layered DAG layout in a worker (`07` §4) |
| Recharts + shadcn chart | https://github.com/recharts/recharts | Chart documents & chat chart blocks (`05` §5.7) |
| embla-carousel | https://github.com/davidcetinkaya/embla-carousel | Carousels (`04` §4.6) |
| remark / rehype / KaTeX / rehype-sanitize | https://github.com/remarkjs · https://katex.org | Markdown + math + safe HTML (kept from reference) |
| Mermaid | https://github.com/mermaid-js/mermaid | `diagram` blocks (lazy) |
| Monaco (`@monaco-editor/react`) | https://github.com/suren-atoyan/monaco-react | Code/Turtle/diff documents |
| cmdk | https://github.com/pacocoursey/cmdk | Command palette + slash menus |
| sonner | https://github.com/emilkowalski/sonner | Toasts |
| react-hook-form + zod | https://react-hook-form.com · https://github.com/colinhacks/zod | Forms + shared arg schemas |
| axum / tower-http | https://github.com/tokio-rs/axum | `mm-serve` HTTP + SSE |
| ts-rs / schemars | https://github.com/Aleph-Alpha/ts-rs · https://github.com/GREsau/schemars | Wire type generation (`01` §7) |
| **In-repo reference** | `~/Scripts/rust_symbolic/frontend` chat section | Transcript reducer, SSE resume, tool cards, SEM viewer, session panel (`00` §1) |
| **In-repo kernel** | `README.md`, `infodocs/*.md`, `design/planning/*` | Invariants, command surface, plan conventions |

---

## 3. Open questions (with recorded defaults)

| # | Question | Default (until changed) | Where revisited |
|---|---|---|---|
| Q1 | Should `mm-serve` also serve the built SPA in production, or dev-only? | Serve static `frontend/dist` when present (single-process convenience) | M0 |
| Q2 | Provider multimodal policy for image inputs | Attach + caption fallback with pre-send note (`08` §5.4) | M5, when provider matrix is tested |
| Q3 | Persist transcripts client-side (IndexedDB) vs server-only | Server-only; client caches pages for scroll | M5 |
| Q4 | Mermaid in chat blocks — ship or cut? | Ship lazy, behind a feature flag; cut if bundle budget fails | M5/M7 |
| Q5 | Treemap/sunburst for which trees by default | Memory composition + budgets + debt; others opt-in | M4 |
| Q6 | Chat planner heuristics (intent → ops) sophistication | Deterministic keyword/grammar routing first; LLM-proposed plans only as a *proposal* shown for approval | M5–M6 |
| Q7 | Read-only "guest" mode (no token) | Not in v1; token always on (`09` §6) | post-v1 |
| Q8 | Co-editing / multi-window sync of drafts | Single window assumed; second window shows `locked` banner | post-v1 |

---

## 4. Traceability: requirement → document

| Requirement (user brief) | Where specified |
|---|---|
| Review rust_symbolic chat webapp | `00` §1 |
| Similar process for MetaMind, improved & more graphically rich | `00` §1.4, `04` §6, `05`, `07` |
| ShadCN blocks + Tailwind template | `00` §5, `01` §3, `04` §2–3 |
| 2-pane system (excluding left menu) | `03` §1, §3 |
| Left pane = current document status (doc/chart/log…, viewer/editor) | `05` (all), esp. §1, §4, §5 |
| Rich component library: cards, tabs, accordions, carousels, forms | `04` §4.3–4.7 |
| Rich tree editors and viewers | `06` (engine §2, views §3, ten trees §5) |
| High-quality mind-map viewer | `07` (all) |
| Right pane = chat log | `08` (all) |
| Rich chat inputs: button grids, image choosers | `08` §5.2, §5.4 |
| Rich chat outputs of all kinds | `08` §4.4, §2.3 |
| Plan in `design/planning/ui/`, very long & detailed | This directory |
| Colorful, business-like, professional, not over-done; no overused rounded corners | `02` (taxonomy §3, radius law §5), `04` §6 |
