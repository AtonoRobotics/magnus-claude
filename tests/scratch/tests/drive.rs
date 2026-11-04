//! Google Drive, brokered (connectors spec, Brokered API connectors;
//! Google's hosted Drive server refuses Magnus with 403). The catalog's
//! Drive manifest names the image built from this checkout
//! (`brokered/drive`), which runs in each person's own microVM and reaches
//! only `www.googleapis.com`, through the appliance's egress proxy. A local
//! HTTPS host stands in for Drive's API, with a certificate for its name
//! from the test's own CA, and FakeOAuthApp for Google's sign-in (the real
//! ones need a Google account: `drive_contract.rs` uses the owner's test
//! account).

use magnus_invariants::*;
use magnus_scratch::stand_in::*;
use serde_json::{json, Value};

const HOST: &str = "www.googleapis.com";
const BASE: &str = "/drive/v3";
const WRITES: [&str; 4] = ["create_file", "share_file", "unshare_file", "delete_file"];

/// What Drive answers, for the requests the tests make: one document, the
/// plan, whose text says Q3's revenue, shared in two pages of permissions;
/// a file made, a share, an unshare and a delete each come back done.
fn drive(r: &Request, _: &[Request]) -> (u16, String, String) {
    let plan = json!({ "id": "f1", "name": "Q3 plan", "mimeType": "application/vnd.google-apps.document", "parents": ["root"], "owners": [{ "emailAddress": "alice@example.com" }] });
    let path = r.path.split('?').next().unwrap_or("");
    match (r.method.as_str(), path) {
        ("GET", "/drive/v3/files") => ok(json!({ "files": [plan] })),
        ("GET", "/drive/v3/files/f1") => ok(plan),
        ("GET", "/drive/v3/files/f1/export") => (200, String::new(), "Q3 revenue was 41.7M.".into()),
        ("POST", "/upload/drive/v3/files") => ok(json!({ "id": "f2", "name": "Q3 summary", "mimeType": "text/plain", "parents": ["root"] })),
        ("POST", "/drive/v3/files/f1/permissions") => ok(json!({ "id": "p1", "type": "user", "role": "reader", "emailAddress": "bob@example.com" })),
        ("DELETE", "/drive/v3/files/f1") => (204, String::new(), String::new()),
        // Who can open the plan, in two pages: Alice, then Bob.
        ("GET", "/drive/v3/files/f1/permissions") if r.path.contains("pageToken=next") => ok(json!({ "permissions": [{ "id": "p2", "type": "user", "role": "reader", "emailAddress": "bob@example.com" }] })),
        ("GET", "/drive/v3/files/f1/permissions") => ok(json!({ "permissions": [{ "id": "p1", "type": "user", "role": "owner", "emailAddress": "alice@example.com" }], "nextPageToken": "next" })),
        ("DELETE", "/drive/v3/files/f1/permissions/p2") => (204, String::new(), String::new()),
        _ => not_found(),
    }
}

/// Sharing a file shows it to someone else, which cannot be called back,
/// so it always asks first: here Magnus has read nothing yet (nothing else
/// makes it ask), and nothing reaches Drive until Alice approves it.
#[tokio::test(flavor = "multi_thread")]
async fn drive_shares_a_file_only_once_the_person_approves_it_reaching_only_google() {
    let api = StandIn::start(HOST, drive).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("let Bob read the Q3 plan", "Bob can read it"),
            1 => tool("drive__share_file", json!({ "file_id": "f1", "email": "bob@example.com", "role": "reader", "message": "The plan." })),
            _ => complete("Bob can read the plan"),
        }
    }))
    .await;
    let (app, provider, _image) = signed_in(&m, &api, "google-drive", "magnus-drive", "drive").await;
    let alice = app.api("alice");

    let task = alice.submit("Let Bob read the Q3 plan.").await;
    let (ask, tool) = asked(&alice, &task, &WRITES).await;
    assert_eq!(tool, "share_file");
    assert!(api.requests().is_empty(), "Drive was called before Alice approved: {:?}", api.calls(BASE));
    approve(&alice, &ask).await;

    let review = alice.wait_item(&task, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    assert_eq!(alice.wait_settled(&task).await["status"], "completed");
    assert_eq!(api.calls(BASE), [("POST".to_string(), "/files/f1/permissions".to_string())]);
    let sent = &api.requests()[0];
    assert!(sent.path.contains("sendNotificationEmail=true") && sent.path.contains("emailMessage=The%20plan."), "{sent:?}");
    assert_eq!(serde_json::from_str::<Value>(&sent.body).unwrap(), json!({ "type": "user", "role": "reader", "emailAddress": "bob@example.com" }));
    only_the_api(&app, &api, &provider, BASE);
}

/// Having read a document (private, and maybe written by others), Magnus
/// asks before it writes anything to Drive (the Rule of Two), though a
/// file made can be trashed; the file goes up with its name and its text
/// in one upload. (After that read, any further call to Drive asks too.)
#[tokio::test(flavor = "multi_thread")]
async fn having_read_a_document_drive_asks_before_making_a_file_and_uploads_it_whole() {
    let api = StandIn::start(HOST, drive).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("write a one-line summary of the Q3 plan into Drive", "the summary is in Drive"),
            1 => tool("drive__read_file_content", json!({ "file_id": "f1" })),
            2 => tool("drive__create_file", json!({ "name": "Q3 summary", "text": "Q3: 41.7M." })),
            _ => complete("the summary is in Drive"),
        }
    }))
    .await;
    let (app, provider, _image) = signed_in(&m, &api, "google-drive", "magnus-drive", "drive").await;
    let alice = app.api("alice");
    let get = |p: &str| ("GET".to_string(), p.to_string());

    let task = alice.submit("Write a one-line summary of the Q3 plan into Drive.").await;
    let (ask, tool) = asked(&alice, &task, &WRITES).await;
    assert_eq!((tool.as_str(), api.calls(BASE)), ("create_file", vec![get("/files/f1"), get("/files/f1/export")]));
    assert!(told(&m, "Write a one-line summary of the Q3 plan").contains("Q3 revenue was 41.7M."), "the document's text did not reach Magnus");
    approve(&alice, &ask).await;

    let review = alice.wait_item(&task, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    assert_eq!(alice.wait_settled(&task).await["status"], "completed");
    let up = api.requests().last().cloned().unwrap();
    assert_eq!((up.method.as_str(), up.path.split('?').next().unwrap()), ("POST", "/upload/drive/v3/files"));
    assert!(up.path.contains("uploadType=multipart"), "{up:?}");
    assert_eq!(up.body, "--magnus-part\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"Q3 summary\"}\r\n--magnus-part\r\nContent-Type: text/plain\r\n\r\nQ3: 41.7M.\r\n--magnus-part--\r\n");
    only_the_api(&app, &api, &provider, "/");
    assert!(api.requests().iter().all(|r| r.path.starts_with(BASE) || r.path.starts_with("/upload/drive/v3/")), "{:?}", api.calls(BASE));
}

/// Who can open a file comes back whole when Drive answers in pages: the
/// tool follows them, so a share only a later page lists can be removed.
#[tokio::test(flavor = "multi_thread")]
async fn drive_lists_everyone_a_file_is_shared_with_across_pages() {
    let api = StandIn::start(HOST, drive).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("stop sharing the Q3 plan with Bob", "Bob can no longer open it"),
            1 => tool("drive__get_file_permissions", json!({ "file_id": "f1" })),
            2 => tool("drive__unshare_file", json!({ "file_id": "f1", "permission_id": "p2" })),
            _ => complete("Bob can no longer open it"),
        }
    }))
    .await;
    let (app, provider, _image) = signed_in(&m, &api, "google-drive", "magnus-drive", "drive").await;
    let alice = app.api("alice");

    let task = alice.submit("Stop sharing the Q3 plan with Bob.").await;
    let (ask, tool) = asked(&alice, &task, &WRITES).await;
    assert_eq!(tool, "unshare_file");
    let sent = api.requests();
    let pages: Vec<&str> = sent.iter().map(|r| r.path.as_str()).filter(|p| p.starts_with("/drive/v3/files/f1/permissions")).collect();
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert!(pages[1].contains("pageToken=next"), "{pages:?}");
    let said = told(&m, "Stop sharing the Q3 plan with Bob.");
    assert!(said.contains("alice@example.com") && said.contains("bob@example.com"), "both pages' permissions did not reach Magnus: {said}");
    approve(&alice, &ask).await;

    let review = alice.wait_item(&task, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    assert_eq!(alice.wait_settled(&task).await["status"], "completed");
    assert_eq!(api.calls(BASE).last().unwrap(), &("DELETE".to_string(), "/files/f1/permissions/p2".to_string()));
    only_the_api(&app, &api, &provider, BASE);
}

/// Google's refusals reach the boundary as what they mean: a sign-in
/// without Drive's access asks the person for it with the app's card, and
/// a delete with no clear answer is an unknown outcome, never a failure
/// that could be tried again as if the file were still there.
#[tokio::test(flavor = "multi_thread")]
async fn drive_s_refusals_reach_the_boundary_as_what_they_mean() {
    let api = StandIn::start(HOST, drive).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("delete the old Q3 plan for good", "it is gone"),
            1 => tool("drive__search_files", json!({ "text": "Q3 plan" })),
            2 => tool("drive__delete_file", json!({ "file_id": "f1" })),
            _ => complete("done"),
        }
    }))
    .await;
    let (app, provider, _image) = signed_in(&m, &api, "google-drive", "magnus-drive", "drive").await;
    let alice = app.api("alice");
    let scope = "https://www.googleapis.com/auth/drive";
    api.then(
        403,
        &format!("WWW-Authenticate: Bearer realm=\"https://accounts.google.com/\", error=\"insufficient_scope\", scope=\"{scope}\"\r\n"),
        json!({ "error": { "code": 403, "message": "Request had insufficient authentication scopes.", "errors": [{ "reason": "insufficientPermissions" }] } }),
    );
    let task = alice.submit("Delete the old Q3 plan for good.").await;
    let (ask, tool) = asked(&alice, &task, &WRITES).await;
    assert!(told(&m, "Delete the old Q3 plan for good.").contains("needs more access from the person for search_files"), "the refusal was not a request for access");
    assert_eq!(tool, "delete_file");
    // The delete: Drive breaks off with 503, so the file may be gone.
    api.then(503, "", json!({ "error": { "code": 503, "message": "The service is currently unavailable." } }));
    approve(&alice, &ask).await;
    eventually("the delete's unknown outcome", std::time::Duration::from_secs(60), || async { app.logs().contains("delete_file: outcome unknown: Google Drive answered HTTP 503 and whether it did this is unknown.").then_some(()) }).await;
    only_the_api(&app, &api, &provider, BASE);
}
