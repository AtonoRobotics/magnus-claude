# Reaction buttons on tasks entries and cards (2026-11-07)

- The app's entry toolbar offers great/annoying on `tasks` entries and every `*_card` kind, as the server takes them (#360, cards/tasks scope). Reason: #351/#354 opened the API but left the app without the buttons, so nobody could react to a card or a started-tasks entry. Kinds match by suffix, so future card kinds are covered; openers stay out until #359 is decided.
