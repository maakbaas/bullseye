# Bullseye sync server

The sync server keeps the computers that sync in step: each sends it what was done there and takes in what was done elsewhere. It decides nothing for a computer: each applies what comes in under its own rules.

## Running it

The image is `ghcr.io/maakbaas/bullseye` (`:latest` is the last release, `:main` the last build of main), for amd64 and arm64. [docker-compose.yml](docker-compose.yml) runs it: put your own token in.

| Variable | Default | What |
|---|---|---|
| `BULLSEYE_TOKEN` | (required) | The token. Every request but `/api/health` carries it: `Authorization: Bearer <token>` |
| `BULLSEYE_DATA` | `/data` | Where everything is kept: a volume |
| `BULLSEYE_LISTEN` | `0.0.0.0:8080` | Where it listens |
| `BULLSEYE_PAGE` | `/app/web` | The window's page, served at `/`; a folder without `index.html`: no page |

It speaks plain HTTP: put it behind your own reverse proxy for HTTPS. **The token is full control** over every computer that syncs, within each computer's rules.

A backup is a copy of `/data`. If the server loses its data or is restored from an older backup, the computers notice and fill it again, under their own rules, so nothing gets looser and nothing done since is lost.

To build the image yourself, from the repository's root: `docker build -f server/Dockerfile -t bullseye .`

## The page

The server shows the app's window at its address, to manage every computer from a browser. It asks for the token once and keeps it in the browser until **Sign out** in Settings. What only a computer has (its Store apps, flash drives, files and sync settings) stays in the app.

## The API

| Method and path | Body | Answer |
|---|---|---|
| `GET /` and the page's files | | the window, without the token (it asks for it) |
| `GET /api/health` | | `ok`, without the token |
| `POST /api/request` | a request, as the window and the CLI send them | its answer: the server's view, under a computer's rules |
| `POST /api/invoke` | `{"cmd", "args", "zone"}`: a command of the window by name, with its arguments | its value, or `{"error": ...}` with `400` |
| `GET /api/snapshot` | | everything defined, the unlock settings, what runs, the log's head, its id (`last`) and when it was done (`newest`) |
| `GET /api/log?after=N&last=ID&wait=S` | | the entries after `N`, waiting up to `S` (at most 25) seconds for one; `410` if `N` is older than the log keeps, `409` if the log is not the one the computer synced with |
| `POST /api/log` | `{"entries": [...], "after": N, "last": ID}` | `{"head": N}`: a computer's entries, each taken once; `409` as for `GET` |
| `PUT /api/devices/<id>` | `{"name", "confirmed", "held", "running"}` | `204`: a computer saying how it is |
| `GET /api/devices` | | the computers, as they last said |

A refusal is `{"error": "..."}` with `400`, `401` (wrong token), `404`, `405`, `410` or `413` (a body over 32 MB).

`/api/request` is what scripts use. A request is `{"cmd": "<command>", "args": {...}}` and its answer `{"kind": "done"}`, `{"kind": "failed", "data": "why"}` or, for `status`, every session, target and condition with their ids:

```sh
TOKEN=...
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"status"}' https://sync.example.com/api/request
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"start","args":{"id":"<session id>"}}' https://sync.example.com/api/request
curl -s -H "Authorization: Bearer $TOKEN" -d '{"cmd":"stop","args":{"id":"<session id>"}}' https://sync.example.com/api/request
```

The command line does the same with `--server`: `bullseye --server https://sync.example.com --token $TOKEN start "Deep work"`.

## Development

`cargo test http::` in `server/` tests the HTTP side and computers syncing over it; the modules it shares with the app are tested in `src-tauri` and `scripts/linux-tests`.
