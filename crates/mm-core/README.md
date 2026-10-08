# `mm-core` — the kernel core

Everything every other Metamind crate needs, and nothing that depends on a store,
a runtime, or a model: identity, time, configuration, hashing, the IRI grammar, and
the store traits. Nothing here is probabilistic, no value is a float, and no
timestamp is anything but UTC.

`#![forbid(unsafe_code)]` is set workspace-wide (`[workspace.lints.rust]`), so it
applies to this crate and every crate that opts in with `[lints] workspace = true`.

## Modules

| Module | Contract |
|---|---|
| `id` | `UlidFactory` — monotonic ULIDs within a process *and* across restarts, by persisting a high-water mark. `parse_ulid` accepts the stored form. `ULID_LEN` is 26. |
| `time` | `Timestamp { seconds, nanos }` — a UTC instant. `to_rfc3339` always renders 9 fractional digits; `from_rfc3339` accepts only UTC (`Z` / `+00:00`) and rejects a local offset rather than guessing. |
| `error` | `MmError` (`Store`, `Graph`, `Event`, `Config`, `Codec`, `Internal`) with a stable `code()` for logging. |
| `config` | `Config` — loaded from `config/metamind.toml` (or `$MM_CONFIG`), then overridden by `MM_*` environment variables. Every relative path is resolved against the repository root, not the working directory. |
| `hash` | `content_hash` — sha256 hex of bytes. `hash_fields` — a sha256 over several fields joined by a separator that cannot occur inside them, so `["ab","c"]` and `["a","bc"]` differ. |
| `iri` | The single IRI funnel. Constants `MM`, `MMC`, `CODE`, `DATA`, `GRAPH`, `DESIGN`, `VENDOR`; constructors `data`, `mm`, `mmc`, `graph`, `module`, `vendor`, `design`; parsers `data_ulid`, `is_named_graph`. `NAMED_GRAPHS` lists the seven runtime graphs. |
| `store` | The three async traits — `Tabular`, `Graph`, `EventSink` — plus `AuditWriter`, and the shared wire types `Param`/`Params`, `ShaclReport`/`ShaclViolation`, `EventKind`, `NewEvent`, `AuditRecord`. |

## Invariants

- **Identity is lowercase Crockford.** `ulid`'s `Display` emits uppercase; every
  identifier is stored and compared through `ulid_string`, so an IRI and a primary
  key agree byte-for-byte. Instance IRIs are `https://metamind.dev/data/{ulid}`.
- **One abstraction for storage.** Callers depend on the traits, never on SQLite or
  Oxigraph directly; the concrete stores live in `mm-store-sqlite` and
  `mm-store-graph`.
- **`EventKind`'s wire name is the log code.** `EventKind::KernelBoot.as_str()` is
  `kernel.boot`, the same string as `mm_log::codes::KERNEL_BOOT`;
  `mm-log`'s tests assert that equality so the two can't drift.

## Where the rest of the kernel lives

`mm-log` (records, event codes, sinks, redaction) · `mm-store-sqlite` (bitemporal
tables, chained append-only audit) · `mm-store-graph` (Oxigraph behind a
single-writer actor, canonical hashing, SHACL) · `mm-eventlog` (append → apply →
commit, deterministic replay) · `mm-cli` (the operator surface).
