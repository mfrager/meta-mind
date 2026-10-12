# Controller chat — design plan (index)

> **What this is.** The build-ready design for the **controller**: one conversational front for the
> Metamind kernel. The operator talks to the being. The being calls tools. The rest of the console
> shows either the being's live state or the one document those tools have put in focus.
>
> **Status:** design only. Nothing in `crates/` or `frontend/` changes until a phase in
> [`05_phases.md`](./05_phases.md) is started.
>
> **Reading order:** `00` → `01` → `02` → `03` → `04` → `05`. If you only read two: `03` (how a turn
> is wrapped into the kernel) and `05` (what to build, in order).

---

## Documents

| # | File | What it specifies |
|---|---|---|
| 00 | [`00_context.md`](./00_context.md) | What the console does today, why a classifier is not a controller, objectives, non-goals, locked decisions |
| 01 | [`01_being_and_voice.md`](./01_being_and_voice.md) | How the controller adopts the live being: baseline personality, chat projection, session introduction, the guide it uses to explain the system |
| 02 | [`02_tool_surface.md`](./02_tool_surface.md) | Every kernel capability as a tool: reads, writes, focus, guide, sandbox tools, and the gates a write must pass |
| 03 | [`03_middleware.md`](./03_middleware.md) | The turn pipeline inside `mm-serve`: session, persona, model, admission, execution, focus, honesty, transcript |
| 04 | [`04_ui.md`](./04_ui.md) | The simplified console: chat is the only way to act; the other pane is state or one focused document |
| 05 | [`05_phases.md`](./05_phases.md) | Five implementation phases, each with files, behaviour, tests, and a pass gate |

The UI plan in [`../ui/INDEX.md`](../ui/INDEX.md) stays the law for tokens, radius, streaming frames, and
the focus-document model. This plan changes *what the default screen is* and *who is allowed to act*.
It does not replace `02_design_system.md`.

---

## The product in one page

The operator opens `http://127.0.0.1:8900` and is already in a conversation with the being. The first
message is an introduction built from the live identity, the kernel health line, and one sentence on
how to talk. Later messages are ordinary language. The controller answers in the being's voice —
friendly, short, direct — and does the work by calling tools.

A tool is not a shell command. It is a typed call into the same kernel the CLI opens: stores, the
being guard, the decision cores, the firewall, the change-set gate, the sandbox tools. The model
proposes a call. Middleware admits it, runs it, and puts the result on the transcript. If the call
produced a document, the other pane shows that document. If it did not, the other pane keeps showing
the being's current state: health, identity, goals, budgets, the last decision.

Production still changes only by promotion. The controller cannot skip that. A missing provider, a
closed store, or a refused invariant comes back as the kernel's own reason, in the being's voice, and
the tool card shows the reason verbatim.

`mm-cli` remains the headless operator. While `mm-serve` holds `data/graph`, the CLI uses another
`--data-dir` or waits. The console does not grow a second executor beside the controller: the command
palette, slash tokens, and the old first-word classifier stop being ways to run the kernel.
