# Starting a session

The console is a conversation with the being. `mm-serve` holds the data directory, including the graph, for as long as it runs.

mm-serve holds the graph while it runs. A CLI pointed at the same data directory fails on the RocksDB lock. Use another --data-dir, or stop mm-serve first.

The controller calls `doctor.run` to read health. The CLI equivalent for a script is `mm-cli doctor`.

The CLI still exists for scripts. This chat is the interactive surface.
