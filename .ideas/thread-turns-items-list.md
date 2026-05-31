# Implement `thread/turns/items/list`

Upstream HEAD has `thread/turns/items/list` as an experimental reserved surface, not a real implementation:

- protocol registration exists
- message dispatch exists
- processor returns `-32601` / `thread/turns/items/list is not supported yet`
- v2 test covers the unsupported response

Local state:

- `ThreadTurnsItemsListParams` and `ThreadTurnsItemsListResponse` already exist
- app-server test helper already has `send_thread_turns_items_list_request`
- protocol registration, dispatch, processor method, and tests are missing locally

Possible first step:

- mechanically port the upstream stub surface and unsupported test

Full implementation sketch:

- load history using the same rules as `thread/turns/list`
- reconstruct turns with the same active-turn merge and status normalization
- find `turn_id`
- paginate that turn's `items`
- return `ThreadTurnsItemsListResponse`

Use an item cursor keyed by `item_id` plus `include_anchor`, mirroring turn pagination without reusing the turn cursor shape.

Tests to add:

- protocol round trip and dispatch
- unsupported stub if porting the upstream placeholder first
- full item list for a turn
- asc/desc pagination with next/backwards cursors
- invalid thread id, missing turn id, invalid cursor
- active running turn behavior matches `thread/turns/list`
- same ephemeral/materialization behavior as `thread/turns/list`
