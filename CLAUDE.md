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
3. Do not merge master into a pull request because master moved, even when the rules check notes that the branch is behind: merge it in (`git merge origin/master`, never rebase a pushed branch) only to resolve a conflict. Each merge restarts that pull request's run on the shared runner; merging master into every open branch kept every run restarting and none finishing (owner, 2026-11-04). A pull request lands merged with current master, and master's own run tests the result in full. GitHub's merge queue is not available on this repository (a private repository owned by a person), so that run after the merge is the gate; a red master run is fixed before anything else lands. A development build is not published when it would replace the owner's app with one that lacks work the app already has; the pull request's checks still pass.
4. One concern per pull request, small enough to review in one sitting (CI warns past 800 changed lines outside tests and documents). A review's fixes are their own pull request from master, never added to an older one.
5. Done means: a test for each fix that fails without it, every suite green in CI, DECISIONS.md and PROGRESS.md updated in the same pull request, and the template's sections filled. Open as a draft while iterating, so the desktop and injection jobs do not run; mark it ready when it should be reviewed and merged.
6. The session that opens a pull request drives it to merged or closed: it watches CI, answers each review thread, and merges it itself once every check is green and no review thread waits on an answer (owner, 2026-11-01: "merge each PR yourself once green"), with a merge commit (`gh pr merge --merge`). With auto-merge on, `gh pr merge --auto --merge` lands it once its checks pass. Do not rerun a failed run without reading why it failed: a rerun of a hang or a broken test only holds the runner again; fix the cause in its own pull request from master. Then open the next, if the cap allows.
7. A pull request with no activity for a day gets a comment; at three days it is labelled `stale`; at five days it is closed and its branch is kept. The `hold` label stops the close. Nothing is left open "for later".

A new push cancels that pull request's earlier CI run. A pull request runs only the tests its changed paths reach (`release/ci-select.sh`): documents run none, a test file runs itself, a connector runs its own crate and tests, and the installers and installed-app tests run only when `desktop/` (or anything the selection does not know) changes; master and the merge queue run everything. Required checks stay required. Drafts do not run the desktop or injection jobs; a skipped required check does not pass branch protection, so a draft cannot merge.
