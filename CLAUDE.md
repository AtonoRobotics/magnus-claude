# Magnus

An autonomous digital worker shipped as an appliance. Start every session by reading:

- `PROGRESS.md`: where things stand and how to resume.
- `DECISIONS.md`: every choice made, one line each, with its reason.
- `tests/invariants/`: the eight invariants as tests. Do not edit them without the owner's approval.

Rules: record each new decision in DECISIONS.md, keep PROGRESS.md current, commit working states, and revert failed attempts instead of patching them.

## How work lands

Pull requests that sit open while master moves on go stale, and a pile of them wastes CI and loses the work (owner, 2026-10-31). One open pull request at a time blocked everything behind a single unfinished one (owner, 2026-11-04: remove that gate). Several may be open. Every change follows these, and CI checks the ones marked (`release/pr-rules.sh`):

1. Several pull requests may be open at once. An agent pull request is one whose branch starts with `cursor/`, whose author is a bot, that carries the `agent` label, or whose description says it was generated with Claude. At most 6 of those are open. Before opening one, `git fetch origin` and count the open agent pull requests; if there are already 6, finish or close one first. CI fails only a new one past the cap. The older ones are not failed for being open, and a person's pull request is not capped. (CI)
2. Branch from current `origin/master` and merge into `master`; never stack a pull request on another branch. (CI)
3. Do not merge master into every open pull request each time master moves: that retests each one. The merge queue, once an admin turns it on, updates and tests a pull request as it lands, so each merge holds the ones before it. Until then, when the rules check warns that the branch is behind master, merge master in (`git merge origin/master`, never rebase a pushed branch) before merging the pull request. A development build is not published when it would replace the owner's app with one that lacks work the app already has; the pull request's checks still pass.
4. One concern per pull request, small enough to review in one sitting (CI warns past 800 changed lines outside tests and documents). A review's fixes are their own pull request from master, never added to an older one.
5. Done means: a test for each fix that fails without it, every suite green in CI, DECISIONS.md and PROGRESS.md updated in the same pull request, and the template's sections filled. Open as a draft while iterating, so the desktop and injection jobs do not run; mark it ready when it should be reviewed and merged.
6. The session that opens a pull request drives it to merged or closed: it watches CI, answers each review thread, and merges it itself once every check is green and no review thread waits on an answer (owner, 2026-11-01: "merge each PR yourself once green"), with a merge commit (`gh pr merge --merge`). When auto-merge and the merge queue are on, `gh pr merge --auto --merge` enters the queue and the queue lands it. Then open the next, if the cap allows.
7. A pull request with no activity for a day gets a comment; at three days it is labelled `stale`; at five days it is closed and its branch is kept. The `hold` label stops the close. Nothing is left open "for later".

A new push cancels that pull request's earlier CI run. Required checks stay required. Drafts do not run the desktop or injection jobs; a skipped required check does not pass branch protection, so a draft cannot merge.
