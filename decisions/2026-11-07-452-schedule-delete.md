# An ended schedule can be deleted (2026-11-07)

- An ended schedule's one control is Delete schedule (Delete on its chat card), which removes it with `DELETE /v1/schedules/{id}`. Reason: since #264 an ended schedule stays, marked with why, until its person removes it, but the app showed it no control, so it could never go (owner, 2026-11-07: "Schedules don't have a delete anymore"). A live schedule keeps Stop schedule and Cancel, whose confirmation says runs stop; an ended one's says nothing stops and its runs stay in Tasks.
