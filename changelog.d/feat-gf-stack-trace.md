- **`gf` works with no resolver on a failure**: with no `[source] command`, or when the
  resolver fails, `gf` follows the failure's stack trace (Go, Java, Node, Python, .NET),
  skips the SDK's and the runtime's frames, and opens the line that threw in the repository
  tmprl runs in, with every other frame of yours in the picker.
