# `imbl::OrdMap` pop-max via `.iter().last()` is O(n) per pop (`Iterator::last` walks every entry); use `.iter().next_back()` for the O(log n) the B-tree can give

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (three-substrate-specs / ordered-index)`
**Domain tags:** rust, imbl, ordmap, performance, ordered-index, frontier, zset

## Trigger

The native ordered index (`rustyred-thg-core/src/ordered.rs`, the sorted-set backing the crawl frontier) implemented `zpop_max` as:

```rust
let ((score, member), _) = self.by_score.iter().last()?; // by_score: OrdMap<(OrderedScore, Member), ()>
```

`zpop_min` correctly used `.iter().next()` (O(log n) to the leftmost), but `zpop_max` reached for `.iter().last()`. `Iterator::last()` has no special-case for double-ended iterators -- its default impl **consumes the whole iterator via `next()`**, so it walks all n entries. The crawl frontier calls `zpop_max` to get each next URL, so a frontier of n URLs popped one at a time is O(n^2) -- exactly the hot path the transient ordered index exists to make fast.

## Rule

To get the largest key of an `imbl::OrdMap` (or any sorted/B-tree map whose iterator is `DoubleEndedIterator`), use `.iter().next_back()`, never `.iter().last()`. `next_back()` descends to the rightmost element in O(log n); `.last()` is O(n). The same applies to `BTreeMap`. When you see `.iter().last()` on an ordered map in a hot path, treat it as a bug.

## Evidence

- `rustyred-thg-core/src/ordered.rs` `zpop_max`: changed `self.by_score.iter().last()` -> `self.by_score.iter().next_back()`; `zpop_min` already used `.iter().next()`.
- `tests/ordered_index_acceptance.rs::transient_ordered_mode_is_commit_free` pops 50,000 transient members via `zpop_max` and now completes promptly; with `.last()` this was 50k * O(n) work.
