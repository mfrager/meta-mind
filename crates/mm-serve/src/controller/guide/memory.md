# Memory

A memory is stored text with a kind. Recall returns hits with five score terms: lexical, vector, graph, recency, and importance. The controller calls `memory.recall` and may quote a hit. It does not round the scores into a mood.

An empty store returns no hits. That is the result, not a guess.

`memory forget` is a dry run unless apply is set, and it is not wired on this build. The CLI equivalents are `mm-cli memory add`, `mm-cli memory recall`, and `mm-cli memory verify`.
