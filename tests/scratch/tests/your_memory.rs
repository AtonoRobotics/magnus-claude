//! "Your memory" (org context build spec, In the app): each person sees,
//! edits and forgets what Magnus keeps about them, and no one else's. An
//! edit is in their words; a note gets a new version by them; marked no
//! longer true, a thing stays listed as such; forgotten, it is in their
//! trash. `desktop/tests/your-memory.mjs` does the same in a real browser.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

const KEEP: &str = "Remember how I like to work.";
const PASS: &str = "You keep the person's long-term memory";

async fn kept(api: &Api) -> Vec<Value> {
    let (st, v) = api.get("/v1/memory").await;
    assert!(st.is_success(), "{v}");
    v["kept"].as_array().cloned().unwrap_or_default()
}

fn find<'a>(kept: &'a [Value], text: &str) -> &'a Value {
    kept.iter().find(|k| k["text"].as_str().is_some_and(|t| t.contains(text))).unwrap_or_else(|| panic!("{text:?} is not kept: {kept:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn each_person_sees_edits_and_forgets_what_magnus_keeps_about_them() {
    let m = FakeModel::start(script(|c| async move {
        if c.body["system"].to_string().contains(PASS) || !c.body.to_string().contains(KEEP) {
            return Reply::Text(String::new());
        }
        let steps = [
            contract_unchecked("remember", "kept"),
            tool("memory_write", json!({ "fact": "Fewer jokes, please.", "relationship": true })),
            tool("memory_write", json!({ "fact": "Alice's desk is by the window.", "topic": "office" })),
            tool("memory_write", json!({ "fact": "Alice takes the 8:10 train.", "topic": "travel" })),
            tool("memory_write", json!({ "fact": "Alice is on the parking committee.", "topic": "office" })),
            tool("bundle_write", json!({ "path": "/users/alice/prefs.md", "frontmatter": { "type": "Note", "title": "Preferences", "scope": "user", "owner_user": "alice" }, "body": "Reports as one page." })),
            complete("kept"),
        ];
        steps.get(c.turn()).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let t = alice.submit(KEEP).await;
    let review = alice.wait_item(&t, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    alice.wait_settled(&t).await;

    let hers = kept(&alice).await;
    assert_eq!(find(&hers, "Fewer jokes")["kind"], "relationship");
    assert_eq!(find(&hers, "desk")["kind"], "fact");
    assert_eq!(find(&hers, "one page")["kind"], "note");
    let (jokes, desk, train, note) = (find(&hers, "Fewer jokes")["path"].clone(), find(&hers, "desk")["path"].clone(), find(&hers, "train")["path"].clone(), find(&hers, "one page")["path"].clone());

    // Bob sees none of it, and can change none of it.
    assert!(kept(&bob).await.is_empty(), "bob sees alice's memory");
    for (call, path) in [("PUT /v1/memory", &desk), ("POST /v1/memory/forget", &note), ("POST /v1/memory/no-longer-true", &jokes)] {
        let (method, url) = call.split_once(' ').unwrap();
        let (st, v) = match method {
            "PUT" => bob.put(url, json!({ "path": path, "text": "Planted." })).await,
            _ => bob.post(url, json!({ "path": path })).await,
        };
        assert_eq!(st.as_u16(), 409, "{call} by bob: {v}");
        assert!(v.to_string().contains("another person's memory"), "{v}");
    }

    // Alice edits a fact: it is in her words, and the old one is gone; a
    // note gets a new version.
    let (st, v) = alice.put("/v1/memory", json!({ "path": desk, "text": "Alice's desk is now by the door." })).await;
    assert!(st.is_success(), "{v}");
    let (st, v) = alice.put("/v1/memory", json!({ "path": note, "text": "Reports as two pages." })).await;
    assert!(st.is_success(), "{v}");
    let hers = kept(&alice).await;
    let edited = find(&hers, "by the door");
    assert_eq!(edited["by"], "person", "{edited}");
    assert!(!hers.iter().any(|k| k["text"].as_str().is_some_and(|t| t.contains("by the window"))), "the edited fact's old text is still kept");
    let n = find(&hers, "two pages");
    assert_eq!((n["path"].clone(), n["version"].as_u64()), (note.clone(), Some(2)), "{n}");
    // Marked no longer true: still listed, as such. Forgotten: in her trash.
    assert_eq!(alice.post("/v1/memory/no-longer-true", json!({ "path": jokes })).await.0.as_u16(), 204);
    assert_eq!(find(&kept(&alice).await, "Fewer jokes")["no_longer_true"], true);
    assert_eq!(alice.post("/v1/memory/forget", json!({ "path": train })).await.0.as_u16(), 204);
    assert!(!kept(&alice).await.iter().any(|k| k["path"] == train), "a forgotten fact is still kept");
    let (_, trash) = alice.get("/v1/trash").await;
    assert!(trash.to_string().contains("memory"), "the forgotten fact is not in her trash: {trash}");

    // The same in a real browser.
    assert!(set_password(&app, "alice", PASSWORD));
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/your-memory.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", "alice")
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(shots())
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "Your memory failed in the app:\n{log}");
    let hers = kept(&alice).await;
    assert!(hers.iter().any(|k| k["text"] == "Reports as three pages."), "the note edited in the app: {hers:?}");
    assert!(!hers.iter().any(|k| k["text"].as_str().is_some_and(|t| t.contains("parking committee"))), "the fact forgotten in the app is still kept");
}
