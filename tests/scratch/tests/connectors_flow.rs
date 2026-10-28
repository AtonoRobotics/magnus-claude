//! Mail, calendar and files, demonstrated through the app's own pages in a
//! real browser (`desktop/tests/connectors.mjs`): the owner adds an app from
//! a preset and removes it; the person signs in to mail with a code and to
//! the calendar through their browser; the owner classifies each app's tools
//! on its review card; a files task and a calendar task run; and an inbox
//! task's reply waits for approval (the Rule of Two), with the message
//! signed as Magnus for the person, before it is sent.
//!
//! The mail and calendar apps and their sign-in provider are fakes here, as
//! in every test; the files app is a stdio server on the appliance.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::{mcp, *};
use magnus_scratch::*;
use magnus_invariants::model::Script;
use serde_json::{json, Value};

const REPLY: &str = "Thanks, received. I'll send the deck by Friday.";

fn calendar() -> mcp::Builder {
    mcp::FakeMcp::builder()
        .world("events", json!([{ "title": "Board meeting", "when": "2026-10-29T15:00", "organizer": "cfo@corp.example" }]))
        .tool("list_events", "List calendar events.", json!({ "type": "object", "properties": { "day": { "type": "string" } } }), |w, _, _| mcp::ok(json!({ "events": w["events"] })))
        .tool(
            "create_event",
            "Create an event and invite people.",
            json!({ "type": "object", "properties": { "title": { "type": "string" }, "when": { "type": "string" }, "attendees": { "type": "array", "items": { "type": "string" } }, "description": { "type": "string" } } }),
            |w, a, _| {
                w["events"].as_array_mut().unwrap().push(a.clone());
                mcp::ok(json!({ "status": "created" }))
            },
        )
}

/// What the model does for each of the person's tasks, by the task's words.
fn model(folder: String) -> Script {
    script(move |c| {
        let folder = folder.clone();
        async move {
            let first = c.messages().first().map(|m| m.to_string()).unwrap_or_default();
            let check = |tool: &str, input: Value, pointer: &str, contains: &str| {
                json!({ "goal": "answer from the app", "assumptions": [], "irreversible_actions": [],
                    "criteria": [{ "id": "c1", "description": "the app has it", "check": { "tool": tool, "input": input, "expect": [{ "pointer": pointer, "contains": contains }] } }] })
            };
            if first.contains("reply to the CFO") {
                match c.turn() {
                    0 => tool("task_contract", check("mail__list_sent", json!({}), "/messages", "Re: Q3 numbers")),
                    1 => tool("mail__list_inbox", json!({})),
                    2 => tool("mail__send", json!({ "to": "cfo@corp.example", "subject": "Re: Q3 numbers", "body": REPLY })),
                    _ => complete("Replied to the CFO."),
                }
            } else if first.contains("plan") {
                let file = format!("{folder}/plan.txt");
                match c.turn() {
                    0 => tool("task_contract", check("files__read_file", json!({ "path": "plan.txt" }), "", "Friday")),
                    1 => tool("files__read_file", json!({ "path": "plan.txt" })),
                    _ => complete(&format!("The plan ({file}) says: ship on Friday.")),
                }
            } else if first.contains("calendar") {
                match c.turn() {
                    0 => tool("task_contract", check("calendar__list_events", json!({ "day": "2026-10-29" }), "/events", "Board meeting")),
                    1 => tool("calendar__list_events", json!({ "day": "2026-10-29" })),
                    _ => complete("Tomorrow: the board meeting at 15:00."),
                }
            } else {
                fail("not a task this demo knows")
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn mail_calendar_and_files_in_the_app_with_an_approved_signed_reply() {
    let oauth = FakeOAuth::start().await;
    let mail = mcp::mail().start().await;
    let cal = calendar().start().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plan.txt"), "Ship on Friday.").unwrap();
    let folder = dir.path().to_string_lossy().to_string();
    let m = FakeModel::start(model(folder.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    assert!(set_password(&app, PERSON, PASSWORD));
    let alice = app.api(PERSON);
    let server = workspace_root().join("tests/scratch/fixtures/stdio_files.py");
    for c in [
        json!({ "name": "mail", "title": "Mail", "url": mail.url, "audience": "public", "label": "private+untrusted", "auth": oauth.auth("device") }),
        json!({ "name": "calendar", "title": "Calendar", "url": cal.url, "audience": "public", "label": "private+untrusted", "auth": oauth.auth("loopback") }),
        json!({ "name": "files", "title": "Files", "transport": "mcp_stdio", "command": ["python3", "-I", server, folder], "audience": "private", "label": "private" }),
    ] {
        let (st, v) = alice.put(&format!("/v1/connectors/{}", c["name"].as_str().unwrap()), c.clone()).await;
        assert!(st.is_success(), "{v}");
    }

    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/connectors.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", PERSON)
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_OAUTH", &oauth.url)
        .env("MAGNUS_REPLY", REPLY)
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(std::env::var("MAGNUS_SHOTS").ok().map(|s| ("MAGNUS_SHOTS", s)))
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the flow failed:\n{log}");
    println!("{log}");

    // What the person saw is what happened.
    let sent = mail.world()["sent"].clone();
    assert_eq!(sent.as_array().map(Vec::len), Some(1), "{sent}");
    assert_eq!(sent[0]["body"], format!("{REPLY}\n\n--\nSent by Magnus, a digital worker, on behalf of {PERSON}."), "{sent}");
    let tasks = alice.tasks().await;
    // The first task only had the apps list their tools: until the owner
    // classified them it could use none, and it stopped there.
    let list: Vec<&Value> = tasks["tasks"].as_array().unwrap().iter().filter(|t| t["text"] != "Look at my apps.").collect();
    assert_eq!(list.len(), 3, "{tasks}");
    for t in list {
        assert_eq!(t["status"], "completed", "{t}");
    }
    let approvals: Vec<Value> = alice.inbox().await.into_iter().filter(|i| i["kind"] == "approval").collect();
    assert_eq!(approvals.len(), 1, "{approvals:?}");
    assert_eq!(approvals[0]["details"]["rule"], "kernel:rule_of_two", "{}", approvals[0]);
    assert_eq!(approvals[0]["resolution"]["by"], format!("person:{PERSON}"));
    let dump = app.db.dump().await;
    for t in oauth.issued() {
        assert!(!dump.contains(&t), "a token reached the log");
    }
}
