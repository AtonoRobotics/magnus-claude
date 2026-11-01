# Per-person walls in the database

Status: plan, for the owner's ruling 9 (2026-11-01: "approve per person
data walls"). Each step is its own pull request.

## What the wall can and cannot guard

Today the walls between people are in Magnus's code. The kernel reads the
whole log (`public.events`) and the content store (`public.content`) at
start, holds what it knows in memory, and serves every request from
there, checking who is asking. The tests try to cross those checks.

A wall in Postgres, using row-level security, guards whatever reads the
database directly. It cannot guard the kernel's own answers, because the
kernel has already read everything. So the wall is worth building for
the readers that are not the kernel's memory:

- the admin's reads of a person's data (ruling 14), when the console is
  its own process (`admin.md`);
- exports, backups taken per person, and deletion checks;
- the knowledge index, which already works this way (`knowledge.rs`:
  a reader role whose row policy reads `magnus.principals`);
- any tool or person given SQL access to the appliance.

To make the kernel's own answers depend on the wall as well, it would
have to read each person's content from Postgres per request, not from
memory. That is a rewrite of how Magnus works (DEC 20). It is not
proposed here.

## The wall

- **Who a row belongs to.** `public.content` gains a `person` column,
  filled when the content is written: the person whose task, chat,
  memory or file the event belongs to (the event's run's person, or the
  event's own `person`). Content that belongs to no one person, such as
  organisation policy text, gets none.
- **Two roles.**
  - The kernel's role keeps reading everything, as replay needs.
  - A new `{role}_person` role, with no login, reads `public.content`
    only where `person = current_setting('magnus.person')`, under a
    forced row policy.
- **How a reader uses it.** A component reading for one person opens a
  transaction, sets `magnus.person`, `SET LOCAL ROLE {role}_person`, and
  reads. A crafted query cannot reach another person's rows.
- **The admin.** With people's data the organisation's (ruling 14's
  default), the admin's reads run as the person being read, through the
  same role, and are recorded as today. With the policy set to private,
  the kernel refuses them before they reach the database.

## Steps

1. **Who owns each row.** Add `person` to `public.content`, written with
   every new event. Fill it for existing rows from their events, in
   `magnus migrate`, once. Test: every content row of a person's run
   carries that person.
2. **The role and its policy.** Create `{role}_person` and the forced
   policy in `migrate`. Test: under the role, with `magnus.person`
   set to carol, no query returns alice's content, including a
   hand-written `SELECT` and a join through `public.events`.
3. **The admin's reads through it.** `GET /v1/people/{id}/data/...`
   reads the person's content through the role. Test: a bug that asks
   for the wrong person (forced in the test) returns nothing, not the
   other person's data.
4. **Exports and per-person backups through it**, as they are built.

## Not in this plan

- Encrypting each person's content under their own key (the
  authentication design's per-user keys) is separate. The wall limits
  who may read rows; keys limit what a copy of the disk reveals.
- Walls on `public.events` itself: the log holds no content since
  deletion step 1, only records (who, when, what kind), which the
  kernel needs whole.
