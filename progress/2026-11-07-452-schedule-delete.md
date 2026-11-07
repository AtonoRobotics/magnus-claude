# Ended schedules get Delete (2026-11-07)

- `desktop/ui/app.js`: `scheduleButtons` gives an ended schedule Delete schedule (`delete-schedule`, with its own confirmation) in Schedules and the schedule itself; `scheduleLine` gives its chat card Delete, confirmed as a delete.
- Test: `a_schedule_that_keeps_failing_pauses_until_its_person_resumes_it` adds a one-run schedule, waits for it to end, and `desktop/tests/schedules.mjs` (new `MAGNUS_ENDED`) checks it shows Delete schedule and no Stop schedule, and that Delete removes it. Fails without the change (no such button), passes with it.
