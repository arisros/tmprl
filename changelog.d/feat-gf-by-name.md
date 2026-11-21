- **`gf` finds an activity by its name**: with no resolver and no stack trace, such as a Go
  error returned rather than panicked, `gf` looks up the activity or workflow's registered
  name, follows the constant that holds it and the registration to the function, and opens
  that, with every other place in the picker.
- **`[source] roots` and `[source] builtin`**: `roots` lists the repositories `gf` searches
  with no resolver, the one tmprl runs in by default; `builtin = false` keeps `gf` to the
  resolver alone.
