# loudible-scheduling

A pure keyed admission policy, ported from the Swift `LoudibleScheduling` target. It decides which
queued items may start. It runs nothing itself and has no dependencies.

It is the bottom of the workspace's layering. `loudible-core` depends on it: the export queue
keeps a `KeyedScheduler<Uuid, String>`.

```sh
cargo test -p loudible-scheduling
```

## Types

| Type | Role |
| --- | --- |
| `Entry<Id, Key>` | A queued item: its `id` and the `key` it serializes on. |
| `KeyedScheduler<Id, Key>` | Pending entries in order, active reservations by id, a concurrency limit and a pause flag. |

The owner drives it:

1. `enqueue(id, key)` adds an entry to the end of the pending list.
2. `reserve_ready()` returns the entries that may start now and marks them active.
3. The owner runs the work, cleans up, then calls `finish(&id)` to release the reservation.

`remove_pending`, `remove_all_pending`, `set_paused` and `is_idle` cover cancellation, pausing and
shutdown.

## Behaviour

* At most one active entry per key. Entries with other keys are not held back by a busy key.
* At most `maximum_concurrent` entries are active. `new(0)` is treated as `new(1)`.
* `reserve_ready` walks pending entries in order; whatever it skips keeps its place.
* `enqueue` returns `false`, and changes nothing, when the id is already pending or active.
* While paused, `reserve_ready` returns nothing. Active reservations stay, and pending entries can
  still be added or removed.
* `remove_pending` and `remove_all_pending` only touch pending entries. Active work is released
  only by `finish`; finishing an unknown id does nothing.
* `is_idle` is true only when nothing is pending and nothing is active.
