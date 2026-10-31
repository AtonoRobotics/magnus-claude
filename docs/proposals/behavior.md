# Behavior: substance and character

Status: plan. Source: the owner's spec "Magnus Desktop App — Behavior Spec"
(2026-10-31, https://claude.ai/artifact/RJUmBfvY58fTNwj9EwRZ13). This plan
maps the spec onto what Magnus already has and orders the rest into steps,
each shipped and tested on its own. Where the spec gives a number (60-minute
review, 3 interruptions a day), the number is the spec's.

## What exists already

| Spec item | Where it is today |
| --- | --- |
| Cedar policy, org layer first, lower layers only narrow | `crates/magnus/src/policy.rs` |
| Approvals rendered from the action payload, not model prose | `boundary.rs` (summary and details from the tool input), app "What will happen" |
| Kill switch halts everything | `api.rs` `/v1/kill-switch`; schedules, runs, vault, updates |
| Schedule proposal cards with accept and decline | `chat.rs` `decide_card`, `Ev::CardDeclined` |
| Per-person memory, playbook, library, company knowledge | `state.rs` `MemoryEntry`, `run/memory.rs`, `library.rs`, `knowledge.rs` |
| Every reply logged with model, instructions version, tools hash | `events.rs` `ModelCall.context`, `ModelReply` |
| Five providers on two adapters, one system string each | `config/providers.toml`, `adapter-anthropic`, `adapter-openai` |
| Experiments with pass@k, per-task verdicts | `dev.rs`, `Ev::Verdict`, `eval/` |
| Kokoro read-aloud, one voice per appliance | `speech.rs`, `[read_aloud] voice` |

Missing: identity and character, relationship memory, disagreement records,
message reactions, general suggestion and observation cards with a ledger,
the periodic review, the interruption budget, quiet hours, the Cedar
suggest/interrupt/act permissions, and judge-scored behavior evals.

## Steps

1. **Identity and the character module.**
   - The functional personality (the spec's eight traits) becomes Magnus's
     identity, in both prompts (`instructions/chat.md`, `agent.md`), the same
     for every provider.
   - A character is a versioned package, `characters/<id>/`: `bible.md` (the
     spec's eight sections) and `character.toml` (id, version, voice). The
     default is the spec's appendix, the chief of staff.
   - Each person picks a character in Settings > Appearance (a new section).
     The org policy layer can fix it for everyone (Cedar).
   - The bible goes into the system prompt after the identity. Every model
     call records the character's id and version in the log.
   - Read-aloud uses the character's voice when the appliance has it.
   - Tests: the character's id and version are on every logged call; a
     person's choice holds across a restart and applies only to them; an org
     lock refuses another choice.
2. **Relationship memory and reactions.**
   - The person's tone instructions ("fewer jokes"), what they delegated and
     standing decisions are kept as relationship memory, always in context
     (not searched for), and never shown to anyone else.
   - The model sees its recent messages to the person, so it does not repeat
     itself.
   - Any message can get a "that was great" or "that was annoying" reaction,
     logged.
   - Disagreements are logged: Magnus's position, the person's decision, and
     later the outcome.
3. **Suggestions and observations.**
   - One schema-validated card: title, plan, evidence, permissions, and
     Accept / Not now / Decline with a reason. The validator refuses a card
     whose evidence does not point at something real in the log (a task, a
     message, a connector item).
   - Accept creates the task or schedule. Schedule cards become this card.
   - Every card and its answer go in a ledger the model reads before it
     suggests again.
   - At cold start, and for each newly connected app, Magnus looks at what it
     may see and proposes two or three things it could take over.
4. **The proactive loop.**
   - The kernel wakes a review run for each person every 60 minutes in their
     working hours and once overnight, and on events: a task done or failed,
     an approval left waiting, a schedule coming up, new items in a
     connected app.
   - The review decides: nothing (the usual), queue an observation for the
     next session, interrupt now, propose an automation, or act.
   - Cedar gets three actions per resource: suggest, interrupt, act. Acting
     without a grant becomes an approval request.
   - Interruptions: at most 3 per working day by default (each person can
     change it). Over-budget items wait for the next session. None outside
     working hours. The kill switch stops the loop.
   - Opening the app shows what changed and the queued observations.
5. **Behavior evals.**
   - The spec's seven suites in `eval/behavior/`: substance invariance,
     character consistency, pushback, recession, long-session wear, proactive
     precision, suggestion grounding.
   - A judge model scores what needs judging, and a person spot-checks each
     run.
   - A character or a provider is offered to people only after it passes.

Each step lands as its own pull request, with its tests, in this order:
step 1 is what every later step speaks through, and step 4 needs step 3's
cards.

## Open

- The default character. The spec recommends the chief-of-staff draft in its
  appendix and leaves the choice to the owner. Step 1 ships that draft unless
  the owner replaces it.
- The org-wide character lock needs an org admin. PR #34 (the admin console
  plan) proposes the `admin` role. Until then, the owner sets the lock.
- Cost: the review loop makes about 10 model calls per person per working
  day, plus one per event that wakes it.
