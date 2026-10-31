# Bullseye sync server

`bullseye-sync` keeps the computers that sync in step: every computer sends it what was done there and takes in what was done elsewhere, and its HTTP API takes the same requests as a computer's service. See [Syncing several computers](../README.md#syncing-several-computers) for what syncing does.

## Running it

The image is `ghcr.io/maakbaas/bullseye-sync` (`:latest` is the last release, `:main` the last build of main), for amd64 and arm64. [docker-compose.yml](docker-compose.yml) is an example.

| Variable | Default | What |
|---|---|---|
| `BULLSEYE_TOKEN` | (required) | The token, at least 16 characters. Every request to the API but `/api/health` carries it: `Authorization: Bearer <token>` |
| `BULLSEYE_DATA` | `/data` | Where everything is kept: a volume |
| `BULLSEYE_LISTEN` | `0.0.0.0:8080` | Where it listens |
| `BULLSEYE_PAGE` | `/app/web` | The window's page (`src/`, in the image), served at `/`; a folder without `index.html`: no page |

It speaks plain HTTP: put it behind your own reverse proxy for HTTPS, as the token travels with every request. **The token is full control** over every computer that syncs, within each computer's rules: whoever has it can start sessions everywhere, as you can from any computer.

Its data (`/data`) is the log (`log.jsonl`, the last 20 000 entries), the computers that report to it (`devices.json`) and its own view of what the log comes to (`view/`). A backup is a copy of the folder. A computer that was away for longer than the log keeps joins again from the server's view on its own. If the server loses its data, or is restored from an older backup, each computer notices on its next round (the log is not the one it synced with). The newer side wins: a computer sends everything it has again if the server has nothing newer than what it saw last (emptied, or an older backup), and takes what the server has if another computer has filled it again since (it was away), keeping what it did itself meanwhile. Either way under each computer's own rules, so no lock gets looser, and nothing done since is lost.

To build the image yourself, from the repository's root: `docker build -f server/Dockerfile -t bullseye-sync .`

## How it works

The server is the app's own code (`src-tauri/src`, built with `#[path]`, as `scripts/linux-tests` is): the log is `hub.rs`, the API types are `sync_api.rs`, and a computer's side is `replica.rs` and `syncing.rs`. It numbers the entries in the order they come and decides nothing for a computer: each computer applies them under its own rules. Its view takes what a computer did as done, so it shows what is defined and what runs; requests to its API go through the same rules as at a computer's keyboard (`handler::handle_shared`), and what they do goes into the log as the server's own entries.

## The page

The server shows the app's own window at `/`, for managing every computer from a browser: it asks for the token once (kept for the browser tab only), then works as the window does, under a computer's rules. `src/js/web.js` stands in for Tauri and sends each of the window's commands to `/api/invoke`, answered by `src-tauri/src/web.rs` as the app answers them; what only a computer has (its Store apps, flash drives, files, syncing) is refused there and hidden. The server has no time zone of its own, so the page sends the browser's offset from UTC and times are worded in it.

## The API

| Method and path | Body | Answer |
|---|---|---|
| `GET /` and the page's files | | the window, without the token (it asks for it) |
| `GET /api/health` | | `ok`, without the token |
| `POST /api/request` | a request, as the window and the CLI send them | its answer: the server's view, under a computer's rules |
| `POST /api/invoke` | `{"cmd", "args", "zone"}`: a command of the window by name, with its arguments | its value, or `{"error": ...}` with `400` (`src-tauri/src/web.rs`) |
| `GET /api/snapshot` | | everything defined, the unlock settings, what runs, the log's head, its id (`last`) and when it was done (`newest`) |
| `GET /api/log?after=N&last=ID&wait=S` | | the entries after `N`, waiting up to `S` (at most 25) seconds for one; `410` if `N` is older than the log keeps; `409` if the log has no entry `N`, or another one than `ID` there (wiped, or restored from a backup): the computer then sends everything it has again, or takes the snapshot if that is newer |
| `POST /api/log` | `{"entries": [...], "after": N, "last": ID}` | `{"head": N}`: a computer's entries, each taken once; `409` as for `GET` when `after` and `last` are given and do not match: nothing goes into a log the computer has not recognised |
| `PUT /api/devices/<id>` | `{"name", "confirmed", "held", "running"}` | `204`: a computer saying how it is |
| `GET /api/devices` | | the computers, as they last said |

A refusal is `{"error": "..."}` with `400`, `401` (wrong token), `404`, `405`, `410` or `413` (a body over 32 MB).

`/api/request` is what scripts use. A request is `{"cmd": "<command>", "args": {...}}` (the `proto::Request` of `src-tauri/src/proto.rs`) and its answer `{"kind": "done"}`, `{"kind": "failed", "data": "why"}` or, for `status`, `{"kind": "status", "data": {...}}`, with every session, target and condition and their ids:

```sh
TOKEN=...
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"status"}' https://sync.example.com/api/request
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"start","args":{"id":"<session id>"}}' https://sync.example.com/api/request
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"stop","args":{"id":"<session id>"}}' https://sync.example.com/api/request
```

The command line does the same with `--server`: `bullseye --server https://sync.example.com --token $TOKEN start "Deep work"` (see [CLI.md](../CLI.md)).

## Development

`cargo test http::` in `server/` tests the HTTP side, the app's own client against it and computers syncing over it; the modules it shares with the app are tested in `src-tauri` and `scripts/linux-tests`. CI runs both and builds the image (`.github/workflows/build.yml`).
