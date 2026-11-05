# Magnus

An autonomous digital worker shipped as an appliance. Start every session by reading:

- `PROGRESS.md`: where things stand and how to resume.
- `DECISIONS.md`: every choice made, one line each, with its reason.
- `tests/invariants/`: the eight invariants as tests. Do not edit them without the owner's approval.
- The one open issue labelled `ready`: the single next pull. Claim it before coding ("How work lands"). An issue labelled only `queue` is the backlog and is not pullable.

Rules: record each new decision in DECISIONS.md, keep PROGRESS.md current, commit working states, and revert failed attempts instead of patching them.

## How work lands

Pull requests that sit open while master moves on go stale, and a pile of them wastes CI and loses the work (owner, 2026-10-31). One open pull request at a time blocked everything behind a single unfinished one (owner, 2026-11-04: remove that gate). Several may be open.

### Work starts from an issue

An agent opens a product or fix pull request only for an open issue in this repository labelled `ready`, or for an issue the owner names in the session (#169). `ready` means the single next pull for work that shares the hot files. At most one open issue carries `ready`. The owner may explicitly mark additional issues `ready` only when they do not share those files: `PROGRESS.md`, `DECISIONS.md`, the release and tag paths (`release/`, an appliance or desktop tag, the workflows that publish), and live OAuth. Only the owner adds `ready`, apart from the agent who holds the one `in-progress` claim promoting the next issue as that claim finishes.

A review finding, a failing test, a product idea or any other new finding is an issue first, and it gets `queue` only. `queue` alone is the backlog: it is not pullable. It stays `queue` until that promotion to `ready`.

Before any code, claim that one issue: take `ready` off, add `in-progress`, and leave a short comment that says what is being done. Claiming still removes `ready` and adds `in-progress`, so the next shared-file issue waits to be promoted. One agent owns an issue that is `in-progress`. An issue that already carries `in-progress` and a claim stays with whoever claimed it.

That issue gets one pull request, branched from current `origin/master` and merged into `master`, with `Fixes #N` in the body so the merge closes it. Closing it is Done: take `in-progress` off once it has merged. When the work cannot go on, take `in-progress` off, add `blocked`, and say why on the issue.

One change that touches the shared files is in flight at a time. Those files are the hot files above, and any other file every pull request has to edit. A second such change waits until that one has merged. Work that leaves those files untouched may run beside it when the owner has marked that issue `ready` as well, or has named it in the session.

The issues are what may be opened. The cap of 6 in rule 1 is how many agent pull requests may be open at once: a seventh waits until one of those six merges or closes, even when another issue is `ready`. CI checks the numbered rules (`release/pr-rules.sh`). This one is the instruction every session follows.

Every change follows these, and CI checks the ones marked (`release/pr-rules.sh`):

1. Several pull requests may be open at once. An agent pull request is one whose branch starts with `cursor/`, whose author is a bot, that carries the `agent` label, or whose description says it was generated with Claude. At most 6 of those are open. Before opening one, `git fetch origin` and count the open agent pull requests; if there are already 6, finish or close one first. CI fails only a new one past the cap. The older ones are not failed for being open, and a person's pull request is not capped. Which pull request to open is an issue, under "Work starts from an issue": at most one open issue carries `ready`, unless the owner marks more that do not share the hot files; six is only how many may be open. (CI)
2. Branch from current `origin/master` and merge into `master`; never stack a pull request on another branch. (CI)
3. Do not merge master into a pull request because master moved, even when the rules check notes that the branch is behind: merge it in (`git merge origin/master`, never rebase a pushed branch) only to resolve a conflict. Each merge restarts that pull request's run on the shared runner; merging master into every open branch kept every run restarting and none finishing (owner, 2026-11-04). A pull request lands merged with current master, and master's own run tests the result in full. GitHub's merge queue is not available on this repository (a private repository owned by a person), so that run after the merge is the gate; a red master run is fixed before anything else lands. A development build is not published when it would replace the owner's app with one that lacks work the app already has; the pull request's checks still pass.
4. One concern per pull request, small enough to review in one sitting (CI warns past 800 changed lines outside tests and documents). A review's fixes are their own pull request from master, never added to an older one.
5. Done means: a test for each fix that fails without it, every suite green in CI, DECISIONS.md and PROGRESS.md updated in the same pull request, and the template's sections filled. Open as a draft while iterating, so the desktop and injection jobs do not run; mark it ready when it should be reviewed and merged.
6. The session that opens a pull request drives it to merged or closed: it watches CI, answers each review thread, and merges it itself once every check is green and no review thread waits on an answer (owner, 2026-11-01: "merge each PR yourself once green"), with a merge commit (`gh pr merge --merge`). With auto-merge on, `gh pr merge --auto --merge` lands it once its checks pass. Do not rerun a failed run without reading why it failed: a rerun of a hang or a broken test only holds the runner again; fix the cause in its own pull request from master. Then open the next, if the cap allows.
7. A pull request with no activity for a day gets a comment; at three days it is labelled `stale`; at five days it is closed and its branch is kept. The `hold` label stops the close. Nothing is left open "for later".

A new push cancels that pull request's earlier CI run. A pull request runs only the tests its changed paths reach (`release/ci-select.sh`): documents run none, a test file runs itself, a connector runs its own crate and tests, and the installers and installed-app tests run only when `desktop/` (or anything the selection does not know) changes; master and the merge queue run everything. Required checks stay required. Drafts do not run the desktop or injection jobs; a skipped required check does not pass branch protection, so a draft cannot merge.
