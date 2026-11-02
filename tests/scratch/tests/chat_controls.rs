//! Tasks and schedules under control in conversation (behavior spec, Tasks
//! and schedules in conversation): Magnus lists, cancels, pauses, resumes,
//! edits and cancels them for the person, through the same operations as
//! the app's buttons. A person controls their own and nobody else's; the
//! owner controls anyone's. A cancelled task says what it had already done.

use magnus_invariants::model::Script;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHAT: &str = "talking with the person you work for in a chat";

/// Chat turns call `calls[said]` (a tool, then text); a task asks a
/// question and so waits.
fn model(calls: Arc<Mutex<Vec<(String, String, Value)>>>) -> Script {
    script(move |c| {
        let calls = calls.clone();
        async move {
            if !c.body["system"].to_string().contains(CHAT) {
                return tool("ask_principal", json!({ "question": "Which quarter?" }));
            }
            let first = c.messages().first().cloned().unwrap_or_default();
            let said = first["content"].as_array().and_then(|b| b.last()).and_then(|b| b["text"].as_str()).unwrap_or_default().to_string();
            let found = calls.lock().unwrap().iter().find(|(s, _, _)| *s == said).cloned();
            match (found, c.turn()) {
                (Some((_, t, input)), 0) => tool(&t, input),
                _ => Reply::Text(c.last_results().first().map(|r| r.text.clone()).unwrap_or_else(|| "Done.".into())),
            }
        }
    })
}

/// Say `text` in a new conversation, answered by `tool(input)`; what the
/// tool answered, as the model saw it.
async fn ask(api: &Api, calls: &Arc<Mutex<Vec<(String, String, Value)>>>, text: &str, tool: &str, input: Value) -> String {
    calls.lock().unwrap().push((text.into(), tool.into(), input));
    let (st, v) = api.post("/v1/chat", json!({ "text": text })).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let (thread, turn) = (v["thread"].as_str().unwrap().to_string(), v["task_id"].as_str().unwrap().to_string());
    for _ in 0..300 {
        let (_, t) = api.get(&format!("/v1/chat/{thread}")).await;
        if let Some(a) = t["entries"].as_array().into_iter().flatten().find(|e| e["kind"] == "answer" && e["task_id"] == turn.as_str()) {
            return a["text"].as_str().unwrap_or_default().to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no answer to {text:?}");
}

async fn schedule(api: &Api, id: &str) -> Value {
    let (_, s) = api.get("/v1/schedules").await;
    s["schedules"].as_array().unwrap().iter().find(|x| x["id"] == id).cloned().unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn magnus_cancels_a_task_in_conversation_and_says_what_it_had_done() {
    let calls = Arc::new(Mutex::new(vec![]));
    let m = FakeModel::start(model(calls.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let task = bob.submit("Draft the quarterly numbers.").await;
    bob.wait_item(&task, "question").await;

    // Bob's tasks are his to see; Carol cannot cancel his.
    let listed = ask(&bob, &calls, "What am I working on?", "list_tasks", json!({})).await;
    assert!(listed.contains(&task), "bob's task is not in his list: {listed}");
    let carol = app.api("carol");
    let refused = ask(&carol, &calls, "Cancel that task.", "cancel_task", json!({ "id": task })).await;
    assert!(refused.contains("no such task"), "carol was not refused: {refused}");
    let (_, t) = bob.get(&format!("/v1/tasks/{task}")).await;
    assert_ne!(t["status"], "cancelled", "someone else cancelled bob's task");

    // Bob cancels it in conversation: it stops, and he is told what it had done.
    let said = ask(&bob, &calls, "Cancel the quarterly numbers.", "cancel_task", json!({ "id": task })).await;
    assert!(said.contains("\"cancelled\"") && said.contains("cannot_be_undone"), "{said}");
    let (_, t) = bob.get(&format!("/v1/tasks/{task}")).await;
    assert_eq!(t["status"], "cancelled", "{t}");
    // Once ended, it cannot be cancelled again.
    let again = ask(&bob, &calls, "Cancel it again.", "cancel_task", json!({ "id": task })).await;
    assert!(again.contains("already ended"), "{again}");

    // The owner controls anyone's: Alice cancels a task of Bob's.
    let other = bob.submit("Draft the yearly numbers.").await;
    bob.wait_item(&other, "question").await;
    let said = ask(&alice, &calls, "Cancel Bob's yearly numbers.", "cancel_task", json!({ "id": other })).await;
    assert!(said.contains("\"cancelled\""), "{said}");
    let (_, t) = bob.get(&format!("/v1/tasks/{other}")).await;
    assert_eq!(t["status"], "cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn magnus_pauses_resumes_edits_and_cancels_a_schedule_in_conversation() {
    let calls = Arc::new(Mutex::new(vec![]));
    let m = FakeModel::start(model(calls.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (bob, carol) = (app.api("bob"), app.api("carol"));
    let (st, v) = bob.post("/v1/schedules", json!({ "text": "Water the plants.", "daily_at": "03:00" })).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let id = v["id"].as_str().unwrap().to_string();

    let listed = ask(&bob, &calls, "What runs on a schedule?", "list_schedules", json!({})).await;
    assert!(listed.contains(&id) && listed.contains("03:00"), "{listed}");
    // Not Carol's to touch.
    let refused = ask(&carol, &calls, "Pause the plants.", "pause_schedule", json!({ "id": id })).await;
    assert!(refused.contains("no such schedule"), "{refused}");
    assert!(schedule(&bob, &id).await["paused"].is_null(), "carol paused bob's schedule");

    ask(&bob, &calls, "Pause the plants.", "pause_schedule", json!({ "id": id })).await;
    assert_eq!(schedule(&bob, &id).await["paused"], "Paused by you");
    ask(&bob, &calls, "Resume the plants.", "resume_schedule", json!({ "id": id })).await;
    assert!(schedule(&bob, &id).await["paused"].is_null());

    // Edited: when it runs and when it ends; what it does stays.
    let said = ask(&bob, &calls, "Make the plants 06:15, and only five more times.", "edit_schedule", json!({ "id": id, "daily_at": "06:15", "max_runs": 5 })).await;
    assert!(said.contains("06:15"), "{said}");
    let s = schedule(&bob, &id).await;
    assert_eq!((s["daily_at"].as_str(), s["max_runs"].as_u64()), (Some("06:15"), Some(5)), "{s}");
    let bad = ask(&bob, &calls, "Make it 25:00.", "edit_schedule", json!({ "id": id, "daily_at": "25:00" })).await;
    assert!(!bad.contains("\"daily_at\":\"25:00\""), "an impossible time was taken: {bad}");
    assert_eq!(schedule(&bob, &id).await["daily_at"], "06:15");

    // The app's buttons are the same operations.
    assert_eq!(bob.post(&format!("/v1/schedules/{id}/pause"), json!({})).await.0.as_u16(), 204);
    assert_eq!(bob.post(&format!("/v1/schedules/{id}/pause"), json!({})).await.0.as_u16(), 409, "paused twice");
    assert_eq!(carol.post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 404, "carol resumed bob's schedule");
    assert_eq!(bob.post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 204);

    // Cancelled: gone.
    let said = ask(&bob, &calls, "Stop the plants for good.", "cancel_schedule", json!({ "id": id })).await;
    assert!(said.contains("\"cancelled\""), "{said}");
    for _ in 0..50 {
        if schedule(&bob, &id).await.is_null() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the cancelled schedule is still there: {}", schedule(&bob, &id).await);
}
