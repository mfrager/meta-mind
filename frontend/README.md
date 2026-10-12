# Metamind web console

Local UI for the kernel. `mm-serve` binds loopback and the Vite app talks to it through `/api`. Nothing in this UI invents a run, a stage timing, a budget figure, or a provider reply. When a route has no data, it shows the kernel's own reason.

## Run it

From the repository root, in two terminals:

```bash
cargo run -p mm-serve -- --data-dir /tmp/mm-console-data
cd frontend && npm ci && npm run dev
```

`npm run dev` listens on `0.0.0.0:8900`. Open `http://127.0.0.1:8900` on this machine, or the machine's address on port 8900 from another host. `mm-serve` stays on `127.0.0.1:8899`; the dev server proxies `/api` to it. A fresh data directory already has the four seeded budget rows (`meta_analysis`, `improvement`, `evolution`, `metacognitive`).

## Pass gate

From the repository root, on a machine with Rust, Node 22, and Chrome (CI installs Playwright's Chromium instead):

```bash
cargo build --workspace && cargo test --workspace
cargo test -p mm-serve && cargo test -p mm-serve --test parity
cargo test -p mm-serve export_types -- --ignored && git diff --exit-code frontend/src/lib/wire
cd frontend
npm ci && npm run typecheck && npm run lint && npm run test && npm run build
npm run test:replay
npm run test:e2e
cd ..
./scripts/ui_flows.sh --offline
cd frontend && npm run lint:radius
```

`npm run build` writes `frontend/dist/bundle-report.json` and fails when the entry chunk is over 250 KiB gzip. `npm run test:replay` replays `chat_run_*.jsonl` to a stable outline. `npm run test:e2e` starts `mm-serve` and Vite and drives Chrome. `./scripts/ui_flows.sh --offline` boots a fresh data directory and checks Flows A–D on the API. It does not call a language model. Stop anything already bound to port 8899 or 8900 before the end-to-end step.

`git diff --exit-code frontend/src/lib/wire` is clean only after the generated wire files are committed. Re-run `export_types` before that check; do not hand-edit `frontend/src/lib/wire/`.
