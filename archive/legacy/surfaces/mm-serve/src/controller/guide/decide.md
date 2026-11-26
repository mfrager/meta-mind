# Decisions

`decide` has three cores: rules, local, and hosted. `admitted: false` means the conformal rule abstained. It is not a yes and it is not a no.

Hosted calls the language-model provider and is a separate ledger row from this chat. Rules is the default. Unavailable cores are reported with their reason. They are not counted as passes.

The CLI equivalent is `mm-cli decide`. The controller tool `decide.ask` is not wired on this build.
