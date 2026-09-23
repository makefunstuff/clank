# `clank-web` — one search, or one page

`clank` observes the local filesystem. `clank-web` is the stage that talks to a
search API, or fetches one URL, and writes the results where a pipe can read
them:

```sh
clank-web "query" | clank -m "summarize with citations"
```

stdout is the results. stderr is diagnostics. The key is on neither stream, and
it is never an argument.

## Exit codes

| code | meaning |
|---|---|
| `0` | the search or fetch completed and the results were written, including an empty result set |
| `1` | the request did not complete: network, an HTTP error, a missing key, or a body that is not the provider's JSON |
| `2` | usage: bad flags, an empty query, an unknown provider, a config file that does not parse |

An empty result set is a completed search. `clank-jev`'s exit `3` is not used.

## Output

The default is one JSON object per line:

```json
{"title":"…","url":"https://…","snippet":"…"}
```

`--text` prints the same three fields separated by tabs, one result per line.
Whitespace inside a field is collapsed so the record stays one line.

A fetched page uses the same fields. When the body is cut at `--max-bytes`, the
JSON object also carries `"truncated": true`, and stderr reports the cut even
with `-q`. `-q` suppresses the result-count line.

## Config

`clank-web` reads `.clank/config.toml` from the working directory when that file
exists. `--config PATH` reads that path instead, and does not also read the
working-directory file. `clank` does not open either one. The example is
[`.clank/config.example.toml`](../.clank/config.example.toml):

```toml
# .clank/config.toml — read by clank-web, from the working directory.
# clank does not open this file. A key is an environment variable named here.

provider = "brave"
max_results = 5

[brave]
api_key_env = "BRAVE_API_KEY"

[tavily]
api_key_env = "TAVILY_API_KEY"
```

| key | meaning |
|---|---|
| `provider` | `brave` or `tavily` |
| `max_results` | 1..=20, default 5; `--max-results` overrides it |
| `brave.api_key_env` | environment variable holding the Brave subscription token; default `BRAVE_API_KEY` |
| `tavily.api_key_env` | environment variable holding the Tavily key; default `TAVILY_API_KEY` |

A field named `api_key` is a usage error: the file names the variable, and the
environment holds the value. With no `provider` and no `--provider`, the one key
that is set selects its provider. Both keys set, and no provider chosen, is exit
2.

## Providers

| provider | request | auth |
|---|---|---|
| `brave` | `GET https://api.search.brave.com/res/v1/web/search?q=…&count=N` | header `X-Subscription-Token` |
| `tavily` | `POST https://api.tavily.com/search` with `query`, `max_results`, and `search_depth` set to `basic` | header `Authorization: Bearer` |

`--base-url` replaces that endpoint, for a proxy or a stub. Tavily's depth is
pinned to `basic` so one invocation is one basic search.

Brave reads `web.results[]` (`title`, `url`, `description`). Tavily reads
`results[]` (`title`, `url`, `content`). A result with no `url` is skipped and
counted on stderr. If every result lacks a url, the exit code is 1.

## Fetch

`--fetch URL` is one GET of an `http` or `https` URL. Redirects are followed.
HTML is reduced to a title and text, and `script` and `style` elements are
dropped; any other body is kept as the snippet. There is no second request, no
JavaScript, and no walk of links on the page. The default body cap is 524288
bytes. `--max-bytes` sets it, up to 8388608.

## Compose

```sh
clank-web "rust sigpipe default disposition" | clank -m "summarize with citations"
printf '%s\n' "$query" | clank-web | clank --no-tools -m "summarize with citations"
clank-web --text "query" | clank -m "summarize with citations"
clank-web --fetch https://example.com | clank -m "one paragraph: what is this page?"
```

`clank --list-tools` stays the four read-only filesystem observers.
