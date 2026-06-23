# Deploying the stock `valkey/valkey:8` image on Railway is unreachable on the private network until you override the start command with `--bind 0.0.0.0 ::` and `--protected-mode no`

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (three-substrate-specs / valkey-deploy)`
**Domain tags:** railway, valkey, redis, private-networking, deploy, ipv6, start-command

## Trigger

Deploying a Valkey cache as a new Railway service from `valkey/valkey:8` (handoff: Valkey, not Redis 8, for the BSD-3 license). The stock image runs `valkey-server` with the wrong defaults for Railway service-to-service traffic:
- it binds `127.0.0.1` (loopback), so it is **invisible on Railway's private network** (which is IPv6, `<service>.railway.internal`);
- `protected-mode` is on, so a non-loopback client with no password is **refused**;
- RDB snapshots + AOF are on, which the spec wants OFF (the cache is recomputable, never a source of truth).

There is no env var for any of this; Valkey is configured by command args or a config file, and you can't bake a config file into the public image. The fix is a Railway **start-command override** carrying every flag.

## Rule

For a stock Valkey/Redis image on Railway as a private cache, set the deploy start command to:

```
valkey-server --bind 0.0.0.0 :: --protected-mode no --maxmemory <N>gb --maxmemory-policy allkeys-lru --save '' --appendonly no
```

`--bind 0.0.0.0 ::` makes it listen on IPv6 (the private DNS path); `--protected-mode no` is safe ONLY because there is no public domain (private-network-only); `--save ''` + `--appendonly no` disable persistence. Set it via a JSON config patch, NOT the dot-path, to avoid shell-quoting the empty `--save ''`:

```
railway environment edit --json <<'JSON'
{"services":{"<service-id>":{"deploy":{"startCommand":"valkey-server --bind 0.0.0.0 :: --protected-mode no --maxmemory 8gb --maxmemory-policy allkeys-lru --save '' --appendonly no"}}}}
JSON
```

Confirm "private only" by checking the service `networking` is empty (no `serviceDomains`). For a pure LRU cache, size `maxmemory` to the hot set, not big-by-default: LRU makes under-sizing graceful (a miss recomputes), and `maxmemory` ≈ the steady-state RAM bill.

## Evidence

- `theorem-valkey` (Theorem project, prod) logs after the patch: `Valkey version=8.1.8 ... Configuration loaded ... Ready to accept connections tcp`, `Running mode=standalone, port=6379`.
- `railway environment config --json` showed `networking: {}` (no public domain) after creation, i.e. private-only by default.
- The start command was set with the `railway environment edit --json` heredoc above; the dot-path form would have mangled `--save ''`.
