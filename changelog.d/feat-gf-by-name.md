- **`gf` finds an activity by its name**: with no resolver and no stack trace, such as a Go
  error returned rather than panicked, `gf` looks up the activity or workflow's registered
  name, follows the constant that holds it and the registration to the function, and opens
  that, with every other place in the picker.
- **`gf` asks the language server**: the repository's own server (`gopls`,
  `typescript-language-server`, `pyright`, `jdtls`, `csharp-ls`), when it is installed,
  confirms where the function is defined and where the name is used.
- **`[source] roots`, `builtin`, `lsp` and `[source.servers]`**: `roots` lists the
  repositories `gf` searches with no resolver, the one tmprl runs in by default;
  `builtin = false` keeps `gf` to the resolver alone; `lsp = false` starts no language
  server; `[source.servers]` gives a language another server command.
