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

`--format text` prints the same three fields separated by tabs, one result per
line. Whitespace inside a field is collapsed so the record stays one line.
`--format jsonl` is the default.

A fetched page uses the same fields. When the body is cut at 524288 bytes, the
JSON object also carries `"truncated": true`, and stderr reports the cut even
with `-q`. `-q` suppresses the result-count line.

## Config

`.clank/config.toml` is optional. Each binary reads `./.clank/config.toml` in
the working directory when that file exists, and does not look in parent
directories. `--config PATH` names a file. `CLANK_CONFIG` names one when the
flag is absent. A path given that way has to exist. A missing
`./.clank/config.toml` leaves flags and the environment in charge.

Precedence is flags, then the environment, then the file, then built-ins.
`clank` and `clank-jev` read `[clank]` (model, endpoint, timeout, token cap).
`clank-web` reads `[web]` and leaves `[clank]` alone, so a chat `base_url` is
not a search endpoint. `[web]` is not required: a file that only names a Brave
key does not change `clank` or `clank-jev`.

The sample is [`fixtures/clank.config.toml`](../fixtures/clank.config.toml). It
names environment variables. It does not contain a key.

```toml
# Optional. Read from ./.clank/config.toml in the working directory.
# --config PATH or CLANK_CONFIG names a different file. Parents are not searched.
# A missing file leaves flags and the environment in charge.
# Precedence: flags, then environment, then this file, then built-ins.
# The key stays in the environment. api_key_env names the variable.
# base_url is that provider's request URL, not [clank].base_url.
# SearXNG has no built-in URL. A base_url makes a local key optional.

[clank]
base_url = "http://127.0.0.1:8080/v1"
model = "local"
api_key_env = "CLANK_API_KEY"
timeout = 600
max_tokens = 8192
max_rounds = 12

[web]
default_provider = "brave"
limit = 5
format = "jsonl"

[web.brave]
api_key_env = "BRAVE_API_KEY"

[web.tavily]
api_key_env = "TAVILY_API_KEY"

[web.firecrawl]
api_key_env = "FIRECRAWL_API_KEY"

[web.searxng]
api_key_env = "SEARXNG_API_KEY"
# Replace with your instance's JSON search URL.
base_url = "http://127.0.0.1:8888/search"

[web.exa]
api_key_env = "EXA_API_KEY"

[web.perplexity]
api_key_env = "PERPLEXITY_API_KEY"
```

`[web]` keys belong above `[web.brave]`. In TOML, a key after a table is part of
that table.

| key | meaning |
|---|---|
| `[clank].base_url` / `model` | chat endpoint for `clank` and `clank-jev`, under the flags and `CLANK_BASE_URL` / `CLANK_MODEL` |
| `[clank].api_key_env` | variable holding the chat key; `CLANK_API_KEY` and `--api-key` win |
| `[clank].timeout` / `max_tokens` / `max_rounds` | under the flags (and `CLANK_TIMEOUT`) and above 600 / 8192 / 12 |
| `[web].default_provider` | `brave` (the built-in), `tavily`, `firecrawl`, `searxng`, `exa`, or `perplexity`; `--provider` wins |
| `[web].limit` | 1..=20, built-in 5; `--limit` wins |
| `[web].format` | `jsonl` (the built-in) or `text`; `--format` wins |
| `[web.<provider>].api_key_env` | variable holding that provider's key. Defaults are below |
| `[web.<provider>].base_url` | request URL when `--base-url` is absent. Not `[clank].base_url` |

`api_key_env` names the variable that holds the key. The sample and this page do
not put a key in the file. The default provider is Brave even when another key
is set: pass `--provider`, or set `[web].default_provider`, to use a different
one.

`--base-url` and `[web.<provider>].base_url` are the request URL, not an origin
and not the chat endpoint. `--base-url` wins. SearXNG has no built-in URL:
without one of those two, the exit code is 2. Firecrawl, Exa, and Perplexity use
their cloud URL when neither is set, and that cloud URL requires a key. A URL
you set is a local instance, and the key is then optional. When a key is set it
is still sent. Brave and Tavily require a key either way.

There is no built-in chat model and no built-in chat URL. `clank` asks for
`--model` / `CLANK_MODEL` / `[clank].model` and `--base-url` / `CLANK_BASE_URL`
/ `[clank].base_url` when none of those three layers set them. `--list-tools`
prints the four filesystem observers without an endpoint.

## Providers

| provider | request | auth | key |
|---|---|---|---|
| `brave` | `GET https://api.search.brave.com/res/v1/web/search?q=…&count=N` | header `X-Subscription-Token` | `BRAVE_API_KEY`, required |
| `tavily` | `POST https://api.tavily.com/search` with `query`, `max_results`, and `search_depth` set to `basic` | header `Authorization: Bearer`. The key is not in the body | `TAVILY_API_KEY`, required |
| `firecrawl` | `POST https://api.firecrawl.dev/v1/search` with `query` and `limit` | header `Authorization: Bearer` when a key is set | `FIRECRAWL_API_KEY`, required on the cloud URL |
| `searxng` | `GET <request-url>?q=…&format=json` | header `Authorization: Bearer` when a key is set. The key is not in the query | `SEARXNG_API_KEY`, optional. No built-in URL |
| `exa` | `POST https://api.exa.ai/search` with `query`, `numResults`, and `contents.highlights` | header `x-api-key`. The key is not in the body | `EXA_API_KEY`, required on the cloud URL |
| `perplexity` | `POST https://api.perplexity.ai/search` with `query` and `max_results` | header `Authorization: Bearer` | `PERPLEXITY_API_KEY`, required on the cloud URL |

One invocation is one search. Tavily's depth stays `basic`. Firecrawl is not
asked to scrape the hits. Exa is not asked for a deep or streamed search.
Perplexity is the Search API, not a chat completion. A local Firecrawl is its
`/v1/search` URL. A local SearXNG is its `/search` URL, and the instance has to
have JSON format enabled or the body is not JSON (exit 1).

Brave reads `web.results[]` (`title`, `url`, `description`). Tavily and SearXNG
read `results[]` (`title`, `url`, `content`). Firecrawl reads a flat `data[]` or
`data.web[]` (`title`, `url`, `description`). Exa reads `results[]` and takes
the snippet from the first `highlights` string, otherwise `text`. Perplexity
reads `results[]` (`title`, `url`, `snippet`). A result with no `url` is skipped
and counted on stderr. If every result lacks a url, the exit code is 1.

## Fetch

`--fetch URL` is one GET of an `http` or `https` URL. Redirects are followed.
HTML is reduced to a title and text, and `script` and `style` elements are
dropped; any other body is kept as the snippet. There is no second request, no
JavaScript, and no walk of links on the page. The body cap is 524288 bytes.

## Compose

```sh
clank-web "rust sigpipe default disposition" | clank -m "summarize with citations"
printf '%s\n' "$query" | clank-web | clank --no-tools -m "summarize with citations"
clank-web --format text "query" | clank -m "summarize with citations"
clank-web --fetch https://example.com | clank -m "one paragraph: what is this page?"
```

`clank --list-tools` stays the four read-only filesystem observers.
