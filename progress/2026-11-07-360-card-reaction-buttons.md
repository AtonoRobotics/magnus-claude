# #360: reaction buttons on tasks entries and cards (opener pending #359)

## Changed

- `desktop/ui/app.js`: `msgActions` offers great/annoying on `tasks`
  entries and `*_card` kinds. Openers excluded until #359 lands.
- `desktop/tests/live-cards.mjs`: new step pins the buttons on answer
  (control), tasks and schedule-card entries, and their absence on the
  person's own messages.

## Verification

- New browser step RED on pristine UI (no buttons on the tasks entry),
  GREEN with the fix (`live_cards` 1/1 locally, real Chromium).
- `node --check` on both touched JS files.
