# RedCoreGraphStore is Send + Sync; the embedded crate's Rc<RefCell> wrapper is a single-thread choice, not a necessity

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (CommonPlace consumer loop)`
**Domain tags:** rust, send-sync, redcore, axum, durable-store, threading

## Trigger

Building `apps/commonplace-api` over the in-memory store, CC deferred the durable `RedCoreGraphStore` + `DiskObjectStore` backing of the HTTP server as a "named follow-up," citing uncertainty over whether `RedCoreGraphStore` is `Send + Sync` (axum's shared `State` requires `Clone + Send + Sync + 'static`, i.e. `Arc<Mutex<Commonplace<RedCoreGraphStore, DiskObjectStore>>>` needs `RedCoreGraphStore: Send`). The uncertainty came from `apps/rustyred-embedded`, which wraps the store in `SharedStore(Rc<RefCell<S>>)` -- which LOOKS like the store needs single-thread cells. It does not. Inspecting the struct (`graph_store.rs`): `RedCoreGraphStore { store: InMemoryGraphStore, data_dir: Option<PathBuf>, _directory_lock: Option<RedCoreDirectoryLock>, options, last_txn_id: u64, ...counters, transient_ordered_indexes: HashMap<String, OrderedIndex>, hook_emitter: Option<HookEmitter>, hook_emit_depth: u32, hook_tenant: String }` -- ALL `Send + Sync`, NO `Rc`/`RefCell`/`Cell`. The embedded crate chose `Rc<RefCell>` because it is single-threaded in-process (cheaper than `Arc<Mutex>`), not because the store is `!Send`.

## Rule

`RedCoreGraphStore` (and `InMemoryGraphStore`) are `Send + Sync` -- they can back a multi-threaded server via `Arc<Mutex<..>>` as axum `State` directly. Do NOT infer `!Send` from `apps/rustyred-embedded`'s `Rc<RefCell>` `SharedStore`: that is a single-thread optimization, not a constraint. Before deferring server-side durable backing over a Send/Sync fear, read the struct's fields (grep for `Rc<`/`RefCell`/`Cell<`); absence of interior-mutability-without-sync means it is thread-safe behind a `Mutex`. The real cost of durable backing over the GraphQL schema is the async-graphql generics-over-store refactor (or fixing the `ApiStore` alias), NOT thread-safety.

## Evidence

- `pub struct RedCoreGraphStore { ... }` in `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` (~line 2387): fields are `InMemoryGraphStore`, `Option<PathBuf>`, `Option<RedCoreDirectoryLock>`, `RedCoreOptions`, `u64` counters, `bool`, `Option<SystemTime>`, `HashMap<String, OrderedIndex>`, `Option<HookEmitter>`, `u32`, `String`. `grep -nE "Rc<|RefCell|Cell<"` over the RedCore region returns nothing.
- `RedCoreGraphStore` exposes both `::memory()` and `::open(dir, RedCoreOptions)`, and (per Codex's F2) impls `EmbeddingGraphStore` -- so it is a drop-in for the `commonplace-api`/`commonplace-mcp` store type once the schema is generic over the store (or its alias is repointed).
- Contrast: `apps/rustyred-embedded` deliberately uses `SharedStore(Rc<RefCell<S>>)` for the in-process embedded mode (no async runtime, single thread).
