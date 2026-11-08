# Resource reconciliation and final acceptance integration

Implemented per-resource last-success/stale feedback, latest-response and identity guards, unchanged-cache failure behavior, deduplicated live announcements, preserved composed work on a failed refresh, and one existing sign-in flow on session failure. Corrected work-panel schedule wording against controls::cancel_schedule. Verification uses an expandable summary, leaves failures visibly open, and omits invented zero usage metadata.

Real rebuilt-appliance `cargo test -p magnus-scratch --test workspace_status` passes: actual stored projects/conversations survive partial/all GET failure; healthy resources update; a delayed older response cannot replace newer data; recovery clears stale state; cached composed file remains readable; session expiry shows sign-in once. `chat_controls::cancelling_a_schedule_stops_its_run_in_flight` passed (61 seconds), confirming actual endpoint effects. Full layout/continuity integration and AT-01–AT-20 host matrix remain in progress; no acceptance claim yet.

Optional work feedback now opens through a labeled Feedback disclosure rather than placing a rating request in the reading flow. Existing verdict/note APIs and values remain unchanged; the outcome host flow opens the disclosure before recording and restoring its verdict. Final combined browser/native regression still required.

Extended the real appliance/browser regression with a stored completed task: its actual deliverable renders first; successful verification is collapsed; rating controls remain hidden until Feedback opens; selecting Right persists through the unchanged task verdict API. Latest forced embedded-asset rebuild and `workspace_status` pass, alongside resource-failure, cached-work and session-expiry cases.
