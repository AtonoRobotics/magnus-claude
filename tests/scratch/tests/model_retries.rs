//! A model call is paid for, failed or not: before each one, a retry too,
//! the kernel reads the run as it is now, and stops without asking when the
//! dollars are spent or the task has been stopped while it waited (#188).

use magnus_invariants::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A failed call the provider charged for takes the run past its dollar
/// limit: the run stops on its budget, and the provider is not asked again.
#[tokio::test(flavor = "multi_thread")]
async fn a_charged_failure_past_the_dollar_limit_asks_the_model_no_more() {
    // A million input tokens at the test model's $2 a million: $2, past $1.
    let model = FakeModel::start(script(|_| async { Reply::Charged(1_000_000) })).await;
    let app = Appliance::start(Spec { max_usd: 1.0, ..Spec::default() }, &model).await;
    let alice = app.api("alice");
    let id = alice.submit("Say hello.").await;
    let t = alice.wait_settled(&id).await;
    let asked = model.requests().len();
    assert_eq!(asked, 1, "the model was asked {asked} times after a charged failure spent the task's dollars: {t}");
    let stop = alice.inbox().await.into_iter().find(|i| i["task_id"] == id.as_str()).unwrap_or_default();
    assert_eq!(stop["details"]["reason"], "budget", "the run did not stop on its budget: {stop}");
}

/// A task cancelled while the kernel waits to retry an unavailable provider
/// is not retried: the wait ends with nothing more asked.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_cancelled_during_the_retry_wait_is_not_retried() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let model = FakeModel::start(script(move |_| {
        let calls = seen.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Reply::Status(529)
        }
    }))
    .await;
    let app = Appliance::start(Spec::default().model_retries(2), &model).await;
    let alice = app.api("alice");
    let id = alice.submit("Say hello.").await;
    eventually("the first request fails", PATIENCE, || {
        let n = calls.load(Ordering::SeqCst);
        async move { (n >= 1).then_some(()) }
    })
    .await;
    // The first wait is a second; cancel inside it.
    let (st, v) = alice.post(&format!("/v1/tasks/{id}/cancel"), json!({})).await;
    assert!(st.is_success(), "cancel: {st} {v}");
    alice.wait_status(&id, &["cancelled"]).await;
    // Past the longest the first two waits could take.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let asked = model.requests().len();
    assert_eq!(asked, 1, "a cancelled task's model call was retried ({asked} requests)");
}
