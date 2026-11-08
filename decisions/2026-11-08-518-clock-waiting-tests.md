# Tests that wait for the clock run beside the cap of eight (2026-11-08, #518)

- CI runs 33 tests at once, of which at most eight are in the `working` nextest group; only tests that sleep through a schedule's or a watch's real minute are outside that group. Reason: 19 such tests held about 2,000 of the suite's 4,046 slot-seconds while using no CPU, and every slot was full for the whole run. The #294 cap is about machine load, and these add none.
- The filter names the waiting tests and everything else is in the group, so a new test holds a slot by default.
- `live_cards` and one schedule test (`a_schedule_that_keeps_failing_pauses_until_its_person_resumes_it`) wait too, but also drive a browser, so they stay inside the cap.
- Only the `ci` profile changes: a developer's run sets its own cap.
