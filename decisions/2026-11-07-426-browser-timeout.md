# Navigation budgets tolerate a loaded runner (#426)

The private-address browser test failed three times in a day with
`page.goto: Timeout 45000ms exceeded` on unrelated PRs, passing on
master around them: a loaded CI runner starves chromium past 45s
even on local pages. The sidecar's navigation budget goes 45s to
90s, and the Rust request wrapper 90s to 120s so it cannot fire
before a slow-but-progressing navigation plus its overhead.
Pure tolerance, no behavior change: real stalls just fail slower.
Timeout constants, so no failing-test-first cycle per the durable
test skill; the gate is the previously-flaking suite passing in CI.
