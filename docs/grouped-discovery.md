# Grouped discovery mode

Toolport has three tool-discovery modes. Identified clients use Auto unless you
choose a mode for the client in Clients or turn off Lazy discovery in Settings.
Auto selects Full or Lazy from the client's capabilities. Installs upgraded from
1.x on its Lazy default keep Lazy. `TOOLPORT_DISCOVERY` selects the process default
for a standalone gateway; shared daemon sessions use their client policy. See
[client discovery](client-discovery.md).

| Mode      | `tools/list` advertises                                                                                                                                                                                                  | Best for                                                                                 |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| `lazy`    | The core meta-tools (`toolport_status`, `toolport_search_tools`, `toolport_call_tool`, `toolport_fetch_result`), plus the Code Mode tool when it is enabled (off by default, so the default exposes the four core tools) | Capable models: minimal, constant context regardless of server count                     |
| `grouped` | The core meta-tools **plus** a per-server `help_<server>` browse tool                                                                                                                                                    | Weaker / local models: an _enumerable_ server choice instead of inventing a search query |
| `full`    | `toolport_status`, `toolport_fetch_result`, and the scoped namespaced catalog (`server__tool`)                                                                                                                           | Debugging, or small setups where full schemas are affordable                             |

These are the core definitions. Code Mode, confirmation,
and negotiated MCP Apps settings can add or change definitions for a particular
client. The exact measurement uses that client's resulting tool array.

Measured with `tiktoken o200k_base`, the four core tools measure about 500 tokens
including the gateway's instructions, and enabling Code Mode adds `toolport_run_script`
for a surface of about 870. The floor is flat regardless of server count, so lazy discovery costs
more than a flat client until the catalog passes roughly 10 to 25 tools.

## Why grouped exists

`lazy` mode is ideal for a strong model: it exposes a single
`toolport_search_tools` and the model invents a query to find what it needs. A
weaker or local model (e.g. a 7B) often struggles to invent a good query from a
blank slate.

Grouped mode keeps context small but replaces the blank-slate search with an
**enumerable** choice: the model sees one `help_<server>` tool per connected
server (`help_github`, `help_stripe`, ...). It picks a server by name, calls
`help_<server>` to list that server's tools (optionally filtered by a `query`),
then runs the chosen tool with `toolport_call_tool` using the exact name the
listing returned.

Context cost is roughly `4 + (number of servers)` tool definitions plus any Code
Mode tools, so grouped
mode is the sweet spot for a **handful of tool-heavy servers** (e.g. Stripe's
587 tools collapse to one `help_stripe`). It is not worth it for many tiny
servers, where the per-server tools approach the full catalog.

## How it works (and why it's safe)

- `help_<server>` is a thin rewrite: internally it runs `toolport_search_tools`
  scoped to that server, reusing the exact ranking, truncation, and schema
  handling. `help_<server>(query: "refund")` on Stripe returns
  `stripe__create_refund`, ready to call.
- Tool execution still goes through `toolport_call_tool`, so the audited call
  path is unchanged: content-defense screening, human-in-the-loop approval, the
  destructive-tool confirm gate, per-client scope enforcement, and result
  shaping all apply exactly as they do in lazy and full mode. Grouped mode adds
  **no new execution surface.**
- The `help_<server>` tools are scoped to the client's allowed servers, so a
  registered HTTP client never sees a browse tool for a server outside its
  scope.

## Catalog exposure measurements

The headline says **net tokens saved**. The expanded measurement panel shows
an exact equation: full catalog baseline tokens minus all catalog response tokens
minus search response tokens. Counts use the bundled `cl100k_base` tokenizer on
compact JSON tool arrays and search text. This local comparison does not measure
provider billing, prompt caching, or a client's native tool search.

Each stdio connection or stateful HTTP session receives one baseline: the full-mode
tool array for the same allowed servers and policies at its first `tools/list`.
Every list response is subtracted, including concurrent repeats, catalog changes,
and a switch to full discovery. None earns another baseline. Search text is also
subtracted, including its guidance. The result can be negative for small catalogs
or repeated responses. Sessionless HTTP is excluded because it has no identifiable
client session. Sessions whose initial full catalog exceeds a known client tool
limit are also excluded. A cold first catalog stays a conservative baseline even if more
tools become available later.

Tokenization runs on the existing background telemetry writer, with no runtime
vocabulary download. Only the first baseline queues full catalog text; subsequent
responses queue their exposed array. Raw text is removed before persistence.
The bounded queue and telemetry failure notices still apply.

New rows have `v: 4`, a local random session ID and an accounting method in
`savings-v4.jsonl`. Rotation preserves baseline, response and search totals in a
separate aggregate. The headline and all measured breakdown values come from
these same records. Pre-2.0 `savings.jsonl`, bytes/4 `savings-v2.jsonl` and tokenized
`savings-v3.jsonl` remain readable without rewriting. They lack verifiable session
accounting, so their prior totals and record counts appear as excluded history.
Clearing Activity removes all four files; an existing session does not earn
another baseline afterward. Team server attribution remains catalog-only, not
net savings. Ordinary tool results are task output and are outside this catalog
comparison.

Prometheus exposes the headline as the signed `toolport_tokens_saved` gauge,
with `tokenizer="cl100k_base"` and `method="net_of_discovery"` labels. Its
component token counters are separate. The existing `toolport_tokens_saved_total`
and `toolport_tool_definition_tokens_estimated_avoided_total` retain historical
estimates; neither includes new tokenizer measurements.

## Enabling it

Per client, set the env in that client's MCP server config:

```
TOOLPORT_DISCOVERY=grouped
```

## Roadmap

Grouped is the stateless, universally-compatible foundation. A future opt-in
enhancement ("dynamic drill-in") could, for clients that reliably honor
`notifications/tools/list_changed`, swap in a server's real flat tools on
activation so weak models call them with top-level arguments. That is gated on
verified client support because a client that caches `tools/list` for the
session would break it; grouped mode works everywhere today.

## Tokenizer cost

On the shared Linux x64 devbox, matching release builds on the supervisor base
grew from 30,285,720 to 32,199,592 bytes (+1,913,872 bytes, 6.3%). A release probe
initialized the bundled cl100k_base vocabulary in 38.2 ms on the telemetry worker;
a preserved 150,558-byte catalog capture took 6.1 ms to count. This is the trimmed
audit capture, not a reconstruction of the audit's full 166,913-byte response.
All 15 preserved compact catalog captures matched Python tiktoken with the same
bundled vocabulary offline.

Five paired 200-iteration gateway runs had median tools/list latency 0.284 to
0.271 ms, search 0.344 to 0.374 ms, and routed calls 0.243 to 0.241 ms. All paired
warm p95 values remained below 0.5 ms. Median handshake time was 52.3 to 52.4 ms;
cold catalog-ready time was 347 to 311 ms. Shared-machine load makes startup
differences noisy, not evidence of a tokenizer speedup. Both variants prime cold
discovery with tools/list, as the supervisor requires.

The 10,000-row debug audit benchmark's cached aggregation median was 0.460 to
0.527 ms (uncached: 52.1 to 55.5 ms; record enqueue: 5.472 to 5.482 microseconds).
The binary growth is acceptable for offline measurements, with vocabulary
initialization and counting deferred to the telemetry worker.
