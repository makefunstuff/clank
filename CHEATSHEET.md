# clank cheatsheet

stdout is data, stderr is diagnostics, the exit code is the verdict.

## Route

`clank-jev` is the typed gate.

```sh
printf '%s\n' "$task" | clank-jev --ask 'What kind of task is this?' --choice code,prose,math --min-prob 0.7 | clank -m "answer this"
```

## Fetch

`clank-web` writes one search to stdout.

```sh
clank-web "rust sigpipe default disposition" | clank -m "summarize with citations"
```

## Generate

`clank` is the model stage.

```sh
rg "userData" src/ | clank -m "what does this do?"
```

## Config

`./.clank/config.toml` is read from the working directory only. `--config PATH`
or `CLANK_CONFIG` names a different file.
