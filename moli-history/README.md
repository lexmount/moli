# moli-history

Native per-Window history storage. This crate has no dependency on V8 or JavaScript objects.

- `WindowHistory` owns the entry list, current index, and state cache revision. Push and replace operations preserve forward-entry and scroll-restoration semantics.
- `HistoryEntry` stores URL and identity metadata, scroll position and restoration mode, and immutable serialized History/Navigation state.
- Entry references can outlive their membership in a history, so a retained `NavigationHistoryEntry` remains readable after replacement or pruning.

`moli-session-history` coordinates the traversable's joint history across documents and frames. `moli-history` stores each Window's native view of those entries. `moli-page-types` transports serialized snapshots across document or VM replacement.

The renderer provides the structured-clone codec and JS bindings. World-specific `History` wrappers share the same native `WindowHistory`; each caches its own deserialized `history.state`. Weak V8 wrapper registrations lease native records without keeping JS contexts alive. Serialized values contain immutable bytes and trusted native attachments such as Blob data, never realm-local JS handles.
