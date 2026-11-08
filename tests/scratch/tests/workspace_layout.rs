//! Reference viewport and keyboard behavior through the real browser bridge.
use magnus_invariants::*;
use magnus_scratch::*;
fn workspace_root() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
#[tokio::test(flavor="multi_thread")]
async fn responsive_workspace_preserves_access_and_preferences() {
 let model=FakeModel::start(script(|_|async{Reply::Text("Recorded.".into())})).await;
 let app=Appliance::start(Spec::default(),&model).await;
 assert!(set_password(&app,"alice",PASSWORD));
 let out=tokio::process::Command::new("node").arg(workspace_root().join("desktop/tests/workspace-layout.mjs"))
 .env("MAGNUS_URL",&app.base).env("MAGNUS_PASSWORD",PASSWORD).env("MAGNUS_CHROMIUM",chromium()).envs(shots()).output().await.unwrap();
 assert!(out.status.success(),"{}{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
 println!("{}",String::from_utf8_lossy(&out.stdout));
}
