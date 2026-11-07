# A review that decides nothing completes quietly (2026-11-07)

- `run/mod.rs` `task_complete`: a review or poll that started no work completes without a contract check, even when it recorded one. Seen by the owner on task `task_604aa80fb2f84121822cc4dd32ec489c`.
- Test: new `a_review_that_decides_nothing_needs_no_review_whatever_its_contract` in `tests/scratch/tests/proactive.rs`; it fails without the change (the review suspends on a `review` item).
