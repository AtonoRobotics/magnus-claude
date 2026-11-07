# #265: get_task/get_schedule and a wider list

Magnus can now look at one task or schedule by id, and past the newest
30 tasks. Two new chat tools, `get_task` and `get_schedule`, under the
same `may_control` rule as every other control; `list_tasks` takes an
optional `limit` (default 30, clamped to 200).

## Shape of the answers

A get returns the same row the list carries, so the two never disagree:
the row builders moved into shared `task_row`/`schedule_row`/`latest_run`
helpers. `get_task` adds what the run did so far, reusing the cancel
report's `stopped_run_report` split (cannot-be-undone first). Both wrap
the row (`{"task": …}`, `{"schedule": …}`) and pass through
`as_policy_allows`, so another person's words stay stripped while data
is private; `listed_taint` reads the single rows too, keeping invariant
4 for gets.

## Incidental fix

The status enum in the tool descriptions said `waiting`, which
`status_str` never returns; the true fifth state is `suspended`. Both
descriptions now list the real vocabulary.
