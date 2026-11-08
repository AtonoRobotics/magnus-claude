//! Reference viewport and keyboard behavior through the real browser bridge.
use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::json;
fn workspace_root() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
#[tokio::test(flavor="multi_thread")]
async fn responsive_workspace_preserves_access_and_preferences() {
 let model=FakeModel::start(script(|c|async move{
  if c.body["system"].to_string().contains("in a chat"){return Reply::Text("Recorded.".into());}
  match c.turn(){0=>contract_unchecked("Navigation review","Useful result"),_=>tool("task_complete",json!({"summary":"Navigation review completed","deliverable":[{"type":"text","text":"Actual reviewed navigation result"}]}))}
 })).await;
 let app=Appliance::start(Spec::default(),&model).await;
 let alice=app.api("alice");let task=alice.submit("Review navigation fixture").await;let review=alice.wait_item(&task,"review").await;
 let bob=app.api("bob");let (status,chat)=bob.post("/v1/chat",json!({"text":"Shared activity update"})).await;assert!(status.is_success(),"{chat}");let thread=chat["thread"].as_str().unwrap();let (_,entries)=bob.get(&format!("/v1/chat/{thread}")).await;let entry=entries["entries"].as_array().unwrap().iter().find(|e|e["kind"]=="message").unwrap()["id"].as_str().unwrap();let (status,shared)=bob.post(&format!("/v1/chat/{thread}/entries/{entry}/share"),json!({"person":"alice"})).await;assert!(status.is_success(),"{shared}");
 assert!(set_password(&app,"alice",PASSWORD));
 let out=tokio::process::Command::new("node").arg(workspace_root().join("desktop/tests/workspace-layout.mjs"))
 .env("MAGNUS_REVIEW_TASK",&task).env("MAGNUS_REVIEW_ITEM",review["id"].as_str().unwrap()).env("MAGNUS_URL",&app.base).env("MAGNUS_PASSWORD",PASSWORD).env("MAGNUS_CHROMIUM",chromium()).envs(shots()).output().await.unwrap();
 assert!(out.status.success(),"{}{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
 println!("{}",String::from_utf8_lossy(&out.stdout));
}
