# #265: get_task/get_schedule and a wider list

## Changed

- `crates/magnus/src/controls.rs`: shared `task_row`/`schedule_row`/
  `latest_run` builders; `get_task` (row plus the cancel-report split of
  what the run did) and `get_schedule` (row with latest run), both under
  `may_control` and `as_policy_allows`; `list_tasks` limit (default 30,
  clamp 200); TOOLS 7 to 9; `listed_taint` reads single rows.
- `crates/magnus/src/tools.rs`: `get_task`/`get_schedule` builtins
  (Read) and specs; `list_tasks` spec gains optional `limit`.
- `crates/magnus/instructions/tools.json`: the two new descriptions, the
  limit sentence, and the true status enum (`suspended`, not `waiting`).
- `crates/magnus/src/boundary.rs`: gets carry their summaries' taint.
- Tests in `tests/scratch/tests/chat_controls.rs`: get one task and
  schedule by id with the control rule (Carol refused, owner sees all);
  default list ends at 30, limit 50 reaches the oldest of 32.

## Verification

- New tests RED (unknown tool) then GREEN; full file 5/6 locally.
- `a_listed_task_summary_taints_the_turn_that_reads_it` fails
  pristine-identical on this box (task spins to budget; noted on #313);
  CI gates that file.
- `magnus` lib 93/93; no new clippy lints on touched lines.
