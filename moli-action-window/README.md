# moli-action-window

`moli-action-window` models delayed, compacted browser actions without owning a
timer or depending on a renderer.

The first action opens a one-shot window. Its deadline is fixed at
`first_action_time + 1 second`; later actions join that window without moving
the deadline. Once a batch is taken, the queue becomes idle and no new deadline
exists until another action arrives.

There is no action-count or memory-capacity cutoff. A window is released only
by its deadline or an explicit read/synchronization barrier; admitting more
actions never rejects input or triggers an early batch.

Screenshot, screencast, and other read barriers call `flush` before reading
rendered state. This immediately returns the current batch and cancels its old
deadline. A later action starts a fresh window.

Compaction is deliberately semantic:

- adjacent scrolls in the same scope become one ordered `ScrollRun`;
- only the latest complete logical click in a scope is retained;
- ordered/opaque actions are never discarded;
- order between surviving action kinds and scopes is preserved.

A scroll run keeps every step instead of summing deltas. Applying each step in
order preserves clamping, scroll snap, delta modes, and target changes. The
host applies the whole returned batch and then performs derived work such as
IntersectionObserver delivery, layout, and rendering once.

The crate is synchronous and uses caller-provided `Instant`s. The Page/event
loop remains responsible for arming one timer for `next_deadline`, executing
ready batches, and flushing before capture barriers.

The generic scope should identify the smallest independently ordered input
target (for example a page/document pair). Clicks are replaced only within the
same scope, and scroll runs never cross scope boundaries.

Typical host flow is:

1. Call `push`. If the admission contains a ready batch, execute that older
   batch first.
2. Replace the Page timer with `next_deadline`, or cancel it when the result is
   `None`.
3. On timer wake, call `take_due`; execute the returned actions in order and
   commit derived rendering work once.
4. Before screenshot/screencast, call `flush`, execute the returned batch and
   its one derived-work commit, then capture.

All timestamps supplied to one queue must come from the same monotonic clock
and be admitted in event-loop order.

## Test layers

- Unit tests pin every state transition, deadline edge, compaction rule,
  counter, cancellation, and high-volume behavior with a virtual clock.
- Public API tests compile the crate as an external consumer, including owned
  scopes, non-`Clone` payloads, consuming batches, all mouse buttons, and
  scroll metadata.
- Model tests compare the implementation with an independent flat reference
  compactor for all 9,331 mixed sequences up to five actions, plus a
  deterministic 1,000-action uninterrupted stream.
- End-to-end host tests model Page execution, one observer/render commit per
  batch, screenshot and screencast barriers, stale timer wakes, late input,
  sustained high-volume input, and a real timer armed from `next_deadline`.
