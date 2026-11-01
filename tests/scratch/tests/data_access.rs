//! The admin's access to people's data is a policy (owner's ruling 14,
//! 2026-11-01): by default a person's tasks, conversations, files and memory
//! are the organisation's, and the owner reads them; each read is in the log
//! and in the person's inbox. The owner can make people's data private.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn the_admin_reads_a_persons_data_unless_the_policy_makes_it_private() {
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, carol) = (app.api("alice"), app.api("carol"));
    let task = carol.submit("Draft the quarterly letter.").await;
    carol.wait_settled(&task).await;
    assert_eq!(carol.get("/v1/data-access").await.1["private"], json!(false), "people's data is private by default");

    // The owner reads carol's tasks: hers, and the read is told to her.
    let (st, v) = alice.get("/v1/people/carol/data/tasks").await;
    assert_eq!(st.as_u16(), 200, "{v}");
    assert!(v["tasks"].as_array().unwrap().iter().any(|t| t["task_id"] == json!(task)), "{v}");
    for what in ["chats", "files", "memory"] {
        assert_eq!(alice.get(&format!("/v1/people/carol/data/{what}")).await.0.as_u16(), 200, "{what}");
    }
    let told = eventually("carol is told her tasks were read", PATIENCE, || async {
        carol.inbox().await.into_iter().find(|i| i["details"]["reason"] == "data_read" && i["details"]["evidence"]["what"] == "tasks")
    })
    .await;
    assert_eq!(told["details"]["evidence"]["by"], "alice", "{told}");
    let reads = app.db.admin().await.query_one("SELECT count(*) FROM public.events WHERE kind = 'person_data_read' AND body->>'person' = 'carol'", &[]).await.unwrap().get::<_, i64>(0);
    assert_eq!(reads, 4, "each read is in the log");

    // Only the owner reads others' data, and only the owner sets the policy.
    assert_eq!(carol.get("/v1/people/alice/data/tasks").await.0.as_u16(), 403);
    assert_eq!(carol.put("/v1/data-access", json!({ "private": true })).await.0.as_u16(), 403);

    // Private: the owner reads no one's data but their own.
    assert_eq!(alice.put("/v1/data-access", json!({ "private": true })).await.0.as_u16(), 204);
    let (st, v) = alice.get("/v1/people/carol/data/tasks").await;
    assert_eq!(st.as_u16(), 403, "the owner read carol's tasks while people's data is private: {v}");
    assert_eq!(alice.get("/v1/people/alice/data/tasks").await.0.as_u16(), 200);
    assert_eq!(alice.put("/v1/data-access", json!({ "private": false })).await.0.as_u16(), 204);
    assert_eq!(alice.get("/v1/people/carol/data/tasks").await.0.as_u16(), 200);
}
