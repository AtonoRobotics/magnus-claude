# Magnus

An autonomous digital worker shipped as an appliance. Start every session by reading:

- `PROGRESS.md`: where things stand and how to resume.
- `DECISIONS.md`: every choice made, one line each, with its reason.
- `tests/invariants/`: the eight invariants as tests. Do not edit them without the owner's approval.

Rules: record each new decision in DECISIONS.md, keep PROGRESS.md current, commit working states, and revert failed attempts instead of patching them.

## How work lands

Pull requests that sit open while master moves on go stale, and their work is lost or redone (owner, 2026-10-31). Every change follows these, and CI checks the first three (`release/pr-rules.sh`):

1. One pull request open at a time, merged in the order opened. Before starting, `git fetch origin` and look at the open pull requests; if one is open, finish it (or have its owner finish it) before opening another. Prepare the next on its own branch meanwhile; open it when the one before merges.
2. Branch from current `origin/master` and merge into `master`; never stack a pull request on another branch.
3. Keep the branch holding master: when master moves, merge it in (`git merge origin/master`, never rebase a pushed branch), test, and push.
4. One concern per pull request, small enough to review in one sitting (CI warns past 800 changed lines outside tests and documents). A review's fixes are their own pull request from master, never added to an older one.
5. Done means: a test for each fix that fails without it, every suite green in CI, DECISIONS.md and PROGRESS.md updated in the same pull request, and the template's sections filled.
6. The session that opens a pull request drives it to merged or closed: it watches CI, answers each review thread, and merges master in. Green and waiting only on the owner's merge, it says so once.
7. A pull request is merged or closed within a day of going green. One that cannot be is split or closed, with a comment naming what was kept and where it goes next; nothing is left open "for later". `pr-stale` comments on any with no activity for a day.
