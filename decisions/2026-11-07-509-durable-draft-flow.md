# Wait for the durable conversation draft

The real OIDC flow on master run 42455421232 passed sign-in, provider connection, task submission, approval, and evidence, then timed out waiting for `#/chat/new`. UX-07 creates a durable local draft and replaces that transient entry route with `#/draft/{id}`. A test can miss the intermediate URL without any product failure.

Wait for the durable draft URL, its navigation entry, the New conversation heading and empty message input. Retain all existing real password/Keycloak, task/evidence, multi-turn answer, thread count and restart checks. Do not extend timeouts, alter the product route, replace the VM/provider, or repeat the failed CI run without a correction.
