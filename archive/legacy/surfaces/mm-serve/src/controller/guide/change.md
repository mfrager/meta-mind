# Change sets and promotion

Production is written only by promotion. The path is change set, then sandbox, then the gate, then promote or reject.

The controller calls `changeset.show` to list change sets. An empty list says "No change sets are recorded." `budget.show` reads the four seeded rows: meta_analysis, improvement, evolution, and metacognitive.

A reject requires a reason. Promote and reject are not wired on this build. The CLI equivalents are `mm-cli changeset new`, `mm-cli sandbox run`, `mm-cli promote`, and `mm-cli budget show`.
