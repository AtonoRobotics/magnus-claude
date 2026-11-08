# Wait for the durable conversation draft

The real OIDC flow on master run 42455421232 passed sign-in, provider connection, task submission, approval, and evidence, then timed out waiting for `#/chat/new`. UX-07 creates a durable local draft and replaces that transient entry route with `#/draft/{id}`. A test can miss the intermediate URL without any product failure.

Wait for the durable draft URL, its navigation entry, the New conversation heading and empty message input. Retain all existing real password/Keycloak, task/evidence, multi-turn answer, thread count and restart checks. Do not extend timeouts, alter the product route, replace the VM/provider, or repeat the failed CI run without a correction.

The Activity first-opened fixture (#511) also raced its real prerequisite. Appliance::start launches the kernel before installing the test provider key. A proactive review could start in that gap and end with “background work ended rather than wait on its person (auth)”, with no model requests or observation events. Start with reviews off, let the existing harness install credentials, then use its existing reconfigure/restart helper to enable real reviews. Preserve every Activity visibility, retry, deduplication and decision-state assertion. No sleeps, synthetic inbox rows, authorization changes or production edits are introduced.
