# A wire-protocol server must not splice client text into SQL on the strength of a CLAIMED type, nor allocate on an unvalidated wire length

**Kind:** rule
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (PG-WIRE + DOCUMENT-TIER engine tiers)`
**Domain tags:** security, sql-injection, dos, wire-protocol, pg-wire, parameterized-queries, input-validation

## Trigger

Peer review of `rustyred-thg-pg-server`'s parameter rendering found two real defects (both shipped in the sweep `0d574f4`, then fixed-forward in `069c6c7`):

1. **Injection.** `render_param` returned a TEXT-format parameter's bytes VERBATIM when the client-DECLARED OID was INT8/INT4/FLOAT8/NUMERIC, and `substitute_placeholders` then spliced that string into the SQL, which was re-parsed and executed. Both the declared OID (from the Parse message) and the bytes (from Bind) are fully client-controlled, so a client could declare `$1` as INT8 and send the text `0 AND created_ms > 0`, injecting a predicate fragment into the supposedly-safe parameterized path. (Blast radius was bounded -- read-only, single-SELECT, OR/UNION rejected -- so it was predicate-bypass / info-disclosure within native views, not writes or RCE. Still real.)
2. **DoS.** `read_frontend_message` did `let len = read_i32(stream)? as usize` with no range check. A negative i32 length sign-extends to ~18 EB, and `vec![0u8; len - 4]` then panics on capacity overflow (kills the connection thread; with a large-but-valid length it is an unbounded per-connection allocation).

## Rule

In any wire-protocol server: (1) NEVER emit a value into a downstream sink (SQL, a path, a command) on the strength of a CLAIMED type -- parse/validate it AS that type and re-serialize the parsed value; on parse failure fall back to a safe escaped literal. A "numeric" parameter must be proven numeric before it goes into SQL unquoted. (2) NEVER `i32 as usize` (or otherwise widen) a wire length without first range-checking it: `if !(MIN..=MAX).contains(&len) { reject }`. A negative or oversized length must be rejected before any allocation sized by it. Bound every length-prefixed read with a sane max.

## Evidence

- Fix `069c6c7`: `render_param` now does `trimmed.parse::<i64>()` / `parse::<f64>()` for numeric OIDs and re-serializes the parsed value, else `sql_literal_string(...)` (quote+escape). Regression test `numeric_param_text_is_validated_not_spliced_raw` asserts `render_param(b"0 AND created_ms > 0", text-format, INT8)` == `'0 AND created_ms > 0'` (a harmless quoted string, not a spliced predicate).
- Same commit: both `read_startup` and `read_frontend_message` reject lengths outside `4..=MAX_MESSAGE_LEN` (64 MiB) before allocating.
- Found by peer review, NOT by the 17+10 green acceptance suite (no test sent a hostile param) -- see the sibling learning `2026-06-19-green-acceptance-suite-hides-silent-wrong-data`.
