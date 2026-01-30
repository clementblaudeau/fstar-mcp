# F* MCP Server

An HTTP MCP (Model Context Protocol) server that provides a front-end for F*'s `--ide` stdio protocol. This enables AI assistants to interact with F* for typechecking, symbol lookup, and other IDE features via the MCP standard.

## Features

- **HTTP SSE Transport**: Supports multiple concurrent clients
- **Session Management**: One session per file path with automatic replacement
- **Full F* IDE Protocol Support**: Typechecking, symbol lookup, autocomplete, and more

## Installation

```bash
# Build the server
cargo build --release

# Run the server
./target/release/fstar-mcp
```

## Configuration

The server listens on `127.0.0.1:3000` by default. Set the `FSTAR_MCP_ADDR` environment variable to change this:

```bash
FSTAR_MCP_ADDR=0.0.0.0:8080 ./target/release/fstar-mcp
```

## MCP Tools

### `create_fstar`

Create a new F* session for a file and run initial typecheck.

**Parameters:**
- `file_path` (string): Path to the F* file to typecheck
- `config_path` (string): Path to the `.fst.config.json` configuration file

**Returns:**
```json
{
  "session_id": "uuid",
  "status": "ok" | "error",
  "diagnostics": [...],
  "fragments": [...]
}
```

### `typecheck_buffer`

Typecheck code in an existing F* session.

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`
- `code` (string): The F* code to typecheck
- `kind` (string, optional): Typecheck kind - `"full"`, `"lax"`, `"cache"`, `"reload-deps"`, `"verify-to-position"`, `"lax-to-position"`. Default: `"full"`
- `to_line` (number, optional): Line to typecheck to (for position-based kinds)
- `to_column` (number, optional): Column to typecheck to (for position-based kinds)

**Returns:**
```json
{
  "status": "ok" | "error",
  "diagnostics": [...],
  "fragments": [...]
}
```

### `update_buffer`

Add or update a file in F*'s virtual file system (vfs-add).

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`
- `file_path` (string): Path to the file in the virtual file system
- `contents` (string): Contents of the file

**Returns:**
```json
{
  "status": "ok" | "error"
}
```

### `lookup_symbol`

Look up type information, documentation, and definition location for a symbol.

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`
- `file_path` (string): Path to the file containing the symbol
- `line` (number): Line number (1-based)
- `column` (number): Column number (0-based)
- `symbol` (string): The symbol to look up

**Returns:**
```json
{
  "kind": "symbol" | "module" | "not_found",
  "name": "FStar.List.map",
  "type_info": "('a -> 'b) -> list 'a -> list 'b",
  "documentation": "...",
  "defined_at": {
    "file": "...",
    "start_line": 1,
    "start_column": 0,
    "end_line": 1,
    "end_column": 10
  }
}
```

### `autocomplete`

Get autocomplete suggestions for a partial symbol.

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`
- `partial_symbol` (string): Partial symbol to complete

**Returns:**
```json
{
  "completions": [
    {
      "match_length": 3,
      "annotation": "val",
      "candidate": "FStar.List.map"
    }
  ]
}
```

### `restart_solver`

Restart the Z3 SMT solver for a session.

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`

**Returns:**
```json
{
  "status": "ok"
}
```

### `close_session`

Close an F* session and clean up resources.

**Parameters:**
- `session_id` (string): Session ID from `create_fstar`

**Returns:**
```json
{
  "status": "ok"
}
```

## Configuration File Format

The `.fst.config.json` file follows the same format as fstar-vscode-assistant:

```json
{
  "fstar_exe": "fstar.exe",
  "include_dirs": [
    "/path/to/lib",
    "$FSTAR_HOME/ulib"
  ],
  "options": [
    "--cache_dir", ".cache"
  ],
  "cwd": "/path/to/project"
}
```

Environment variables can be used with `$VAR` or `${VAR}` syntax.

## Example Usage

```python
import httpx

# Connect to the MCP server
client = httpx.Client(base_url="http://localhost:3000")

# Create a session
response = client.post("/tools/create_fstar", json={
    "file_path": "/path/to/MyFile.fst",
    "config_path": "/path/to/.fst.config.json"
})
result = response.json()
session_id = result["session_id"]

# Look up a symbol
response = client.post("/tools/lookup_symbol", json={
    "session_id": session_id,
    "file_path": "/path/to/MyFile.fst",
    "line": 10,
    "column": 5,
    "symbol": "map"
})
symbol_info = response.json()

# Close the session when done
client.post("/tools/close_session", json={
    "session_id": session_id
})
```

## Development

```bash
# Run tests
cargo test

# Run with debug logging
RUST_LOG=fstar_mcp=debug cargo run
```

## License

MIT
