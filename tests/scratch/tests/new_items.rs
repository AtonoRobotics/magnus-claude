//! A new item in a connected app wakes a review for its person
//! (#369): watched reads diff their answers against what was last
//! seen, and new ids wake naming the count. The first sighting seeds
//! the baseline silently.

use magnus_invariants::model::Script;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REVIEW: &str = "reviewing one person's state";

fn done(summary: &str) -> Reply {
    tool("task_complete", json!({ "summary": summary, "deliverable": [{ "type": "text", "text": summary }] }))
}

async fn drain_reviews(bob: &Api) {
    eventually("all reviews settled", Duration::from_secs(60), || {
        let me = bob.clone();
        async move {
            let (_, v) = me.get("/v1/tasks").await;
            let reviews: Vec<_> = v["tasks"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t["review"] == true)
                .collect();
            (!reviews.is_empty() && reviews.iter().all(|t| t["status"] == "completed" || t["status"] == "failed")).then_some(())
        }
    })
    .await;
}

fn seen(prompts: &Arc<Mutex<Vec<String>>>, needle: &str) -> bool {
    prompts.lock().unwrap().iter().any(|p| p.contains(needle))
}

/// The first read seeds silently; after two messages arrive, the next
/// read wakes a review naming them.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_item_in_a_connected_app_wakes_a_review() {
    let mail = mcp::mail().start().await;
    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_prompts = prompts.clone();
    let m = FakeModel::start(script(move |c| {
        let seen_prompts = seen_prompts.clone();
        async move {
            if c.body["system"].to_string().contains(REVIEW) {
                seen_prompts.lock().unwrap().push(c.text());
                return done("Reviewed quietly.");
            }
            if c.turn() == 0 {
                return tool("mail__list_inbox", json!({}));
            }
            // Failing ends the task without a contract; the read it
            // made is what matters. The failure's own wake loses to
            // the item wake, which fires during the read above.
            return tool("task_fail", json!({ "reason": "checked the inbox" }));
        }
    }))
    .await;
    let app = Appliance::start(
        Spec::default()
            .proactive(json!({ "on": true }))
            .app("mail", &mail, "public", "private", &[("list_inbox", "read")])
            .watch("mail", "list_inbox", "/messages", "subject"),
        &m,
    )
    .await;
    let bob = app.api("bob");
    drain_reviews(&bob).await;
    // The seed read: one message in the world, nothing to wake on.
    let first = bob.submit("Check the inbox.").await;
    assert_eq!(bob.wait_settled(&first).await["status"], "failed");
    drain_reviews(&bob).await;
    assert!(!seen(&prompts, "new in mail"), "the seeding read woke a review");
    // Two messages arrive; the next read wakes naming the count.
    let (from, body) = ("boss@corp.example", "Read this now.");
    mail.set_world("inbox", json!([{ "from": "cfo@corp.example", "subject": "Q3 numbers", "body": "Confidential: revenue 41.7M" }, { "from": from, "subject": "First", "body": body }, { "from": from, "subject": "Second", "body": body }]));
    let second = bob.submit("Check the inbox again.").await;
    assert_eq!(bob.wait_settled(&second).await["status"], "failed");
    eventually("a review naming the new mail", Duration::from_secs(60), || {
        let prompts = prompts.clone();
        async move { seen(&prompts, "2 new in mail").then_some(()) }
    })
    .await;
}
