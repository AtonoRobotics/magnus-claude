//! Google Drive, brokered (connectors spec, Brokered API connectors;
//! Google's hosted Drive server refuses Magnus with 403, as Gmail's did).
//! An MCP server over Drive's REST API (`https://www.googleapis.com/drive/v3`,
//! uploads under `/upload/drive/v3`), the `/init` of its own microVM for
//! each person: each call comes with the person's access token, goes to
//! Google through the appliance's egress proxy over TLS checked here, and
//! leaves nothing behind. Its tools are those a person uses in Drive: find
//! and read files, make, copy, rename, move and trash them, share them, and
//! delete one for good. Every tool is classified in the catalog manifest
//! (`catalog/google-drive.toml`); sharing and deleting ask the person
//! first.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use magnus_brokered_google::{enc, opt_num, query, s, Api};
use magnus_brokered_kit::{serve, Failure, Tool};
use serde_json::{json, Value};

/// `drive` alone: the one Drive scope on the project's consent screen, and
/// the only one that reads, changes, shares and deletes any of the
/// person's files (`drive.file` reaches only files Magnus made or was
/// given; `drive.readonly` changes nothing).
const API: Api = Api { host: "www.googleapis.com", base: "/drive/v3", name: "Google Drive", scope: "https://www.googleapis.com/auth/drive" };

const FIELDS: &str = "id,name,mimeType,modifiedTime,size,parents,webViewLink,trashed,shared,description,owners(emailAddress)";
const FOLDER: &str = "application/vnd.google-apps.folder";
/// The most of a file read or downloaded in one call.
const MOST: usize = 10 << 20;

fn file_id(a: &Value) -> Result<String, String> {
    let v = s(a, "file_id");
    if v.is_empty() {
        return Err("file_id is missing".into());
    }
    Ok(enc(&v))
}

/// A file as a person reads it.
fn file(f: &Value) -> Value {
    json!({
        "file_id": f["id"], "name": f["name"], "mime_type": f["mimeType"], "modified": f["modifiedTime"], "size": f["size"].as_str().and_then(|n| n.parse::<u64>().ok()),
        "folder_ids": f["parents"].as_array().cloned().unwrap_or_default(), "link": f["webViewLink"], "trashed": f["trashed"] == true, "shared": f["shared"] == true,
        "description": f["description"].as_str().unwrap_or_default(), "owners": f["owners"].as_array().into_iter().flatten().filter_map(|o| o["emailAddress"].as_str()).collect::<Vec<_>>(),
    })
}

/// Words as Drive's query language quotes them.
fn quoted(v: &str) -> String {
    format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The query of a search: Drive's own syntax as given, the words, the
/// folder, and nothing in the trash unless asked.
fn drive_query(a: &Value) -> String {
    let mut q: Vec<String> = vec![];
    if let Some(raw) = a["query"].as_str().filter(|x| !x.trim().is_empty()) {
        q.push(format!("({raw})"));
    }
    if let Some(w) = a["text"].as_str().filter(|x| !x.trim().is_empty()) {
        q.push(format!("fullText contains {}", quoted(w)));
    }
    if let Some(f) = a["folder_id"].as_str().filter(|x| !x.is_empty()) {
        q.push(format!("{} in parents", quoted(f)));
    }
    if a["include_trashed"] != true {
        q.push("trashed = false".into());
    }
    q.join(" and ")
}

fn files(a: &Value, t: &str, q: String, order: Option<&str>) -> Result<Value, Failure> {
    let pairs = [
        ("q", Some(q)),
        ("orderBy", order.map(String::from)),
        ("pageSize", opt_num(a, "max_results").or(Some("20".into()))),
        ("pageToken", a["page_token"].as_str().map(String::from)),
        ("fields", Some(format!("nextPageToken,files({FIELDS})"))),
        ("supportsAllDrives", Some("true".into())),
        ("includeItemsFromAllDrives", Some("true".into())),
    ];
    let r = API.call("GET", &format!("/files{}", query(&pairs)), t, None)?;
    Ok(json!({ "files": r["files"].as_array().into_iter().flatten().map(file).collect::<Vec<_>>(), "next_page_token": r["nextPageToken"].as_str() }))
}

fn search_files(a: &Value, t: &str) -> Result<Value, Failure> {
    files(a, t, drive_query(a), None)
}

fn list_recent_files(a: &Value, t: &str) -> Result<Value, Failure> {
    files(a, t, "trashed = false".into(), Some("modifiedTime desc"))
}

fn meta(id: &str, t: &str) -> Result<Value, Failure> {
    API.call("GET", &format!("/files/{id}?fields={}&supportsAllDrives=true", enc(FIELDS)), t, None)
}

fn get_file_metadata(a: &Value, t: &str) -> Result<Value, Failure> {
    Ok(file(&meta(&file_id(a)?, t)?))
}

fn get_file_permissions(a: &Value, t: &str) -> Result<Value, Failure> {
    let r = API.call("GET", &format!("/files/{}/permissions?fields={}&supportsAllDrives=true", file_id(a)?, enc("permissions(id,type,role,emailAddress,domain,displayName)")), t, None)?;
    let people: Vec<Value> = r["permissions"].as_array().into_iter().flatten().map(|p| json!({ "permission_id": p["id"], "type": p["type"], "role": p["role"], "email": p["emailAddress"], "domain": p["domain"], "name": p["displayName"] })).collect();
    Ok(json!({ "permissions": people }))
}

/// How a Google file is read as text: a document or slides as plain text,
/// a sheet as CSV (its first sheet).
fn export_as(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        FOLDER => None,
        m if m.starts_with("application/vnd.google-apps.") => Some("text/plain"),
        _ => None,
    }
}

fn textual(mime: &str) -> bool {
    mime.starts_with("text/") || ["application/json", "application/xml", "application/javascript", "application/x-yaml", "application/csv"].contains(&mime) || mime.ends_with("+xml") || mime.ends_with("+json")
}

/// The file's bytes, exported to `to` if given, refused past the most a
/// call carries.
fn content(id: &str, size: Option<u64>, to: Option<&str>, t: &str) -> Result<Vec<u8>, Failure> {
    if size.is_some_and(|n| n as usize > MOST) {
        return Err(format!("The file is larger than {} MB, more than one call carries. Nothing was changed.", MOST >> 20).into());
    }
    let b = match to {
        Some(m) => API.bytes(&format!("/files/{id}/export?mimeType={}", enc(m)), t)?,
        None => API.bytes(&format!("/files/{id}?alt=media&supportsAllDrives=true"), t)?,
    };
    if b.len() > MOST {
        return Err(format!("The file is larger than {} MB, more than one call carries. Nothing was changed.", MOST >> 20).into());
    }
    Ok(b)
}

fn read_file_content(a: &Value, t: &str) -> Result<Value, Failure> {
    let id = file_id(a)?;
    let f = file(&meta(&id, t)?);
    let mime = f["mime_type"].as_str().unwrap_or_default().to_string();
    let to = export_as(&mime);
    if mime == FOLDER {
        return Err("That is a folder: search_files with its folder_id lists what is in it.".to_string().into());
    }
    if to.is_none() && !textual(&mime) {
        return Err(format!("That file is {mime}, not text: download_file_content gives its bytes.").into());
    }
    let text = String::from_utf8_lossy(&content(&id, f["size"].as_u64(), to, t)?).to_string();
    let clipped = text.chars().count() > 100_000;
    Ok(json!({ "file_id": f["file_id"], "name": f["name"], "mime_type": mime, "text": text.chars().take(100_000).collect::<String>(), "truncated": clipped }))
}

fn download_file_content(a: &Value, t: &str) -> Result<Value, Failure> {
    let id = file_id(a)?;
    let f = file(&meta(&id, t)?);
    let mime = f["mime_type"].as_str().unwrap_or_default();
    let to = a["export_mime_type"].as_str().filter(|m| !m.is_empty()).or(mime.starts_with("application/vnd.google-apps.").then_some("application/pdf"));
    let b = content(&id, f["size"].as_u64(), to, t)?;
    Ok(json!({ "file_id": f["file_id"], "name": f["name"], "mime_type": to.unwrap_or(mime), "size": b.len(), "base64": STANDARD.encode(&b) }))
}

/// The bytes a call gives a file: `text`, or `base64`.
fn given(a: &Value) -> Result<Option<Vec<u8>>, String> {
    if let Some(b) = a["base64"].as_str() {
        return STANDARD.decode(b).map(Some).map_err(|_| "base64 is not base64. Nothing was changed.".to_string());
    }
    Ok(a["text"].as_str().map(|t| t.as_bytes().to_vec()))
}

/// A file's metadata and bytes in one upload (`multipart/related`), with a
/// boundary its bytes do not hold.
fn multipart(meta: &Value, mime: &str, data: &[u8]) -> (String, Vec<u8>) {
    let mut boundary = "magnus-part".to_string();
    while data.windows(boundary.len()).any(|w| w == boundary.as_bytes()) {
        boundary.push('x');
    }
    let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

fn create_file(a: &Value, t: &str) -> Result<Value, Failure> {
    let mut m = json!({ "name": s(a, "name") });
    if let Some(f) = a["folder_id"].as_str().filter(|x| !x.is_empty()) {
        m["parents"] = json!([f]);
    }
    if let Some(d) = a["description"].as_str() {
        m["description"] = json!(d);
    }
    if let Some(k) = a["convert_to"].as_str() {
        m["mimeType"] = json!(format!("application/vnd.google-apps.{k}"));
    }
    let q = format!("supportsAllDrives=true&fields={}", enc(FIELDS));
    let made = match given(a)? {
        None => API.call("POST", &format!("/files?{q}"), t, Some(&m))?,
        Some(data) => {
            let (kind, body) = multipart(&m, a["mime_type"].as_str().unwrap_or("text/plain"), &data);
            API.upload("POST", &format!("/upload/drive/v3/files?uploadType=multipart&{q}"), t, &kind, &body)?
        }
    };
    Ok(file(&made))
}

fn create_folder(a: &Value, t: &str) -> Result<Value, Failure> {
    let mut m = json!({ "name": s(a, "name"), "mimeType": FOLDER });
    if let Some(f) = a["folder_id"].as_str().filter(|x| !x.is_empty()) {
        m["parents"] = json!([f]);
    }
    Ok(file(&API.call("POST", &format!("/files?supportsAllDrives=true&fields={}", enc(FIELDS)), t, Some(&m))?))
}

fn copy_file(a: &Value, t: &str) -> Result<Value, Failure> {
    let mut m = json!({});
    if let Some(n) = a["name"].as_str() {
        m["name"] = json!(n);
    }
    if let Some(f) = a["folder_id"].as_str().filter(|x| !x.is_empty()) {
        m["parents"] = json!([f]);
    }
    Ok(file(&API.call("POST", &format!("/files/{}/copy?supportsAllDrives=true&fields={}", file_id(a)?, enc(FIELDS)), t, Some(&m))?))
}

fn update_file(a: &Value, t: &str) -> Result<Value, Failure> {
    let id = file_id(a)?;
    let mut m = json!({});
    for k in ["name", "description"] {
        if let Some(v) = a[k].as_str() {
            m[k] = json!(v);
        }
    }
    let mut q = format!("supportsAllDrives=true&fields={}", enc(FIELDS));
    if let Some(to) = a["move_to_folder_id"].as_str().filter(|x| !x.is_empty()) {
        let from: Vec<String> = meta(&id, t)?["parents"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
        q.push_str(&format!("&addParents={}&removeParents={}", enc(to), enc(&from.join(","))));
    }
    let changed = match given(a)? {
        None => API.call("PATCH", &format!("/files/{id}?{q}"), t, Some(&m))?,
        Some(data) => {
            let (kind, body) = multipart(&m, a["mime_type"].as_str().unwrap_or("text/plain"), &data);
            API.upload("PATCH", &format!("/upload/drive/v3/files/{id}?uploadType=multipart&{q}"), t, &kind, &body)?
        }
    };
    Ok(file(&changed))
}

fn trashed(a: &Value, t: &str, yes: bool) -> Result<Value, Failure> {
    Ok(file(&API.call("PATCH", &format!("/files/{}?supportsAllDrives=true&fields={}", file_id(a)?, enc(FIELDS)), t, Some(&json!({ "trashed": yes })))?))
}

fn trash_file(a: &Value, t: &str) -> Result<Value, Failure> {
    trashed(a, t, true)
}

fn untrash_file(a: &Value, t: &str) -> Result<Value, Failure> {
    trashed(a, t, false)
}

fn delete_file(a: &Value, t: &str) -> Result<Value, Failure> {
    API.call("DELETE", &format!("/files/{}?supportsAllDrives=true", file_id(a)?), t, None)?;
    Ok(json!({ "deleted": true }))
}

/// Who a share is for: a person or group by email, or anyone with the
/// link; never a new owner.
fn grant(a: &Value) -> Result<Value, String> {
    let role = s(a, "role");
    if !["reader", "commenter", "writer"].contains(&role.as_str()) {
        return Err("role is reader, commenter or writer".into());
    }
    if a["anyone_with_link"] == true {
        return Ok(json!({ "type": "anyone", "role": role }));
    }
    let email = s(a, "email");
    if !email.contains('@') {
        return Err("email is the address to share with, or anyone_with_link is true".into());
    }
    Ok(json!({ "type": if a["group"] == true { "group" } else { "user" }, "role": role, "emailAddress": email }))
}

fn share_file(a: &Value, t: &str) -> Result<Value, Failure> {
    let g = grant(a)?;
    let mut q = vec![("supportsAllDrives", Some("true".to_string())), ("fields", Some("id,type,role,emailAddress".to_string()))];
    if g["type"] != "anyone" {
        q.push(("sendNotificationEmail", Some((a["notify"] != false).to_string())));
        q.push(("emailMessage", a["message"].as_str().map(String::from)));
    }
    let p = API.call("POST", &format!("/files/{}/permissions{}", file_id(a)?, query(&q)), t, Some(&g))?;
    Ok(json!({ "permission_id": p["id"], "type": p["type"], "role": p["role"], "email": p["emailAddress"] }))
}

fn unshare_file(a: &Value, t: &str) -> Result<Value, Failure> {
    let p = s(a, "permission_id");
    if p.is_empty() {
        return Err("permission_id is missing (get_file_permissions lists them)".to_string().into());
    }
    API.call("DELETE", &format!("/files/{}/permissions/{}?supportsAllDrives=true", file_id(a)?, enc(&p)), t, None)?;
    Ok(json!({ "removed": true }))
}

/// An object schema: its properties, which are required, and nothing else.
fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn tools() -> Vec<Tool> {
    let st = json!({ "type": "string" });
    let n = json!({ "type": "integer" });
    let yes = json!({ "type": "boolean" });
    let f = json!({ "type": "object", "required": ["file_id", "name", "mime_type"] });
    let many = obj(json!({ "files": { "type": "array", "items": f }, "next_page_token": { "type": ["string", "null"] } }), &["files"]);
    let one = obj(json!({ "file_id": st }), &["file_id"]);
    let body = json!({ "name": st, "description": st, "text": st, "base64": st, "mime_type": st, "folder_id": st, "convert_to": { "type": "string", "enum": ["document", "spreadsheet", "presentation"] } });
    let tool = |name, description, input, output, run| Tool { name, description, input, output, run };
    vec![
        tool("search_files", "Find files: by words in them or their name (text), by Drive's query syntax (query, such as name contains 'plan'), in a folder; none in the trash unless asked.", obj(json!({ "text": st, "query": st, "folder_id": st, "include_trashed": yes, "max_results": n, "page_token": st }), &[]), many.clone(), search_files),
        tool("list_recent_files", "List the files changed most recently.", obj(json!({ "max_results": n, "page_token": st }), &[]), many, list_recent_files),
        tool("get_file_metadata", "Read a file's name, type, size, folder, owners and link.", one.clone(), f.clone(), get_file_metadata),
        tool("get_file_permissions", "List who can open a file and how.", one.clone(), obj(json!({ "permissions": { "type": "array" } }), &["permissions"]), get_file_permissions),
        tool("read_file_content", "Read a file as text: a document or slides as plain text, a sheet as CSV, a text file as it is.", one.clone(), obj(json!({ "file_id": st, "name": st, "mime_type": st, "text": st, "truncated": yes }), &["text"]), read_file_content),
        tool("download_file_content", "Download a file's bytes, base64; a Google file is exported (as PDF unless export_mime_type is given).", obj(json!({ "file_id": st, "export_mime_type": st }), &["file_id"]), obj(json!({ "file_id": st, "name": st, "mime_type": st, "size": n, "base64": st }), &["base64"]), download_file_content),
        tool("create_file", "Make a file in the person's Drive, from text or base64, optionally as a Google document, sheet or slides.", obj(body.clone(), &["name"]), f.clone(), create_file),
        tool("create_folder", "Make a folder.", obj(json!({ "name": st, "folder_id": st }), &["name"]), f.clone(), create_folder),
        tool("copy_file", "Copy a file, with a new name or into a folder if given.", obj(json!({ "file_id": st, "name": st, "folder_id": st }), &["file_id"]), f.clone(), copy_file),
        tool("update_file", "Rename a file, change its description or its contents, or move it to another folder: only what is given. Drive keeps the earlier version.", obj(json!({ "file_id": st, "name": st, "description": st, "text": st, "base64": st, "mime_type": st, "move_to_folder_id": st }), &["file_id"]), f.clone(), update_file),
        tool("trash_file", "Move a file to the trash, from which it can be taken back for 30 days.", one.clone(), f.clone(), trash_file),
        tool("untrash_file", "Take a file back out of the trash.", one.clone(), f, untrash_file),
        tool("delete_file", "Delete a file for good, without the trash. It cannot be taken back.", one, obj(json!({ "deleted": yes }), &["deleted"]), delete_file),
        tool("share_file", "Share a file with someone by email (they are told, unless notify is false) or with anyone who has the link: as reader, commenter or writer.", obj(json!({ "file_id": st, "email": st, "group": yes, "anyone_with_link": yes, "role": { "type": "string", "enum": ["reader", "commenter", "writer"] }, "notify": yes, "message": st }), &["file_id", "role"]), obj(json!({ "permission_id": st, "type": st, "role": st, "email": { "type": ["string", "null"] } }), &["permission_id"]), share_file),
        tool("unshare_file", "Stop sharing a file with someone (a permission_id from get_file_permissions).", obj(json!({ "file_id": st, "permission_id": st }), &["file_id", "permission_id"]), obj(json!({ "removed": yes }), &["removed"]), unshare_file),
    ]
}

fn main() {
    serve("magnus-drive", tools())
}

#[cfg(test)]
#[path = "../../../tests/brokered-drive/drive.rs"]
mod tests;
