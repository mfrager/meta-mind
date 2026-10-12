# Web console

Operator notes for the local UI (`mm-serve` + `frontend`). How to start it, and the checks behind Milestone 7, are in [`frontend/README.md`](../frontend/README.md).

The console quotes the kernel. A healthy boot still reports graph and language-model services as unavailable:

- graph: the RDF store is opened on demand
- language model: `no language-model provider is configured`
- runs: `no run is recorded`
- stage timings: `No stage timings are recorded for this run.`
- provenance: `No events are recorded.`
- change sets: `No change sets are recorded.`
- a direct module load returns 409 `use the change-set flow`

Creating a change set is the write that is allowed. The UI does not fill in stage durations, verdicts, or proof steps.

## Performance

Measured by `npm run test` and `npm run build` (local, Node 22):

| Check | Budget | Result |
| --- | --- | --- |
| Last chat frame applied to a 1,000-row transcript | ≤ 16 ms | under the budget (`src/lib/perf.test.ts`) |
| Severity filter on 1,000,000 log rows | ≤ 100 ms | indexed count, under the budget |
| Expand a 20,000-node tree | ≤ 50 ms | under the budget |
| 5,000-node map layout at zoom 0.4, with depth culling | one layout step ≤ 50 ms | under the budget |
| Entry JavaScript | ≤ 250 KiB gzip | 134.8 KiB gzip |

Route chunks (workspace, catalog, settings, charts, mind map) are separate from the entry. A frame rate on a GPU was not sampled; the map check times the culled layout, which is the work a pan or zoom has to finish.

## Accessibility

Automated, and what was not run:

- Hue tint pairs from the design system (text 700 on 50 and 100, text 300 on 950) are at least 3:1. That is the floor for 10px bold micro-labels.
- Body text, muted text, and button text are at least 4.5:1 in both themes. Light `--primary` is `oklch(0.52 0.155 258)` so white button text clears 4.5:1.
- Keyboard: Ctrl/⌘K opens the palette, Escape closes it, other shortcuts stay quiet in a text field, and Escape inside a dialog is left to the dialog.
- `prefers-reduced-motion: reduce` sets the motion tokens to 0ms and disables the pulse.
- `prefers-contrast: more` sets the border to `--border-strong`.
- Print CSS hides the sidebar, top bar, status bar, and chat pane so the open document prints.
- Catalog axe runs in unit tests. The color-contrast axe rule is off there; contrast is the unit test above, because happy-dom does not compute the oklch tokens the way a browser does.
- VoiceOver and NVDA were not run on this machine. Before calling a screen-reader pass done, walk: palette (Ctrl/⌘K, type, Enter, Escape), composer (Ctrl/⌘Enter sends, Enter is a newline), a tree (arrow keys), a dialog (Escape returns focus), and the status bar kernel state.

## Themes

The theme control in the top bar switches light and dark. The choice is stored in the browser. Reduced motion and higher contrast follow the operating system.
