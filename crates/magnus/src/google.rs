//! Google's connectors call the generally available REST APIs. The hosted
//! Workspace MCP servers (`gmailmcp`, `drivemcp`, `calendarmcp`, `chatmcp`,
//! `docsmcp`, `sheetsmcp` `.googleapis.com`) are a Developer Preview and are
//! not called.
//!
//! Gmail v1, Drive v3, Calendar v3, Docs v1, Sheets v4, Chat v1. People v1
//! is used only to name the account when the sign-in answer has no email.

use crate::apps::{AppTool, CallError, CallOutput};
use crate::rest::explain;
use serde_json::{json, Value};

pub const ISSUER: &str = "https://accounts.google.com";
pub const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE: &str = "https://oauth2.googleapis.com/revoke";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Product {
    Gmail,
    Drive,
    Calendar,
    Docs,
    Sheets,
    Chat,
}

impl Product {
    fn name(self) -> &'static str {
        match self {
            Product::Gmail => "Gmail",
            Product::Drive => "Drive",
            Product::Calendar => "Calendar",
            Product::Docs => "Docs",
            Product::Sheets => "Sheets",
            Product::Chat => "Chat",
        }
    }
}

/// The connector's address is one of Google's ordinary APIs (or a test
/// double on this machine). A preview MCP host is not.
pub fn handles(url: &str) -> bool {
    product(url).is_some()
}

pub fn oauth_metadata() -> Value {
    json!({
        "authorization_endpoint": AUTH,
        "token_endpoint": TOKEN,
        "revocation_endpoint": REVOKE,
        "issuer": ISSUER,
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true,
    })
}

fn local_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn product(url: &str) -> Option<Product> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str().unwrap_or("");
    if host.contains("mcp") {
        return None;
    }
    let path = u.path();
    if local_host(host) {
        return match path {
            p if p.starts_with("/gmail/") => Some(Product::Gmail),
            p if p.starts_with("/drive/") => Some(Product::Drive),
            p if p.starts_with("/calendar/") => Some(Product::Calendar),
            p if p.starts_with("/docs/") => Some(Product::Docs),
            p if p.starts_with("/sheets/") => Some(Product::Sheets),
            p if p.starts_with("/chat/") => Some(Product::Chat),
            _ => None,
        };
    }
    match host {
        "gmail.googleapis.com" => Some(Product::Gmail),
        "docs.googleapis.com" => Some(Product::Docs),
        "sheets.googleapis.com" => Some(Product::Sheets),
        "chat.googleapis.com" => Some(Product::Chat),
        "www.googleapis.com" if path.starts_with("/drive/") => Some(Product::Drive),
        "www.googleapis.com" if path.starts_with("/calendar/") => Some(Product::Calendar),
        _ => None,
    }
}

pub fn list_tools(url: &str) -> Option<Vec<AppTool>> {
    Some(match product(url)? {
        Product::Gmail => gmail_tools(),
        Product::Drive => drive_tools(),
        Product::Calendar => calendar_tools(),
        Product::Docs => docs_tools(),
        Product::Sheets => sheets_tools(),
        Product::Chat => chat_tools(),
    })
}

fn tool(name: &str, description: &str, read_only: bool, required: &[&str], fields: &[(&str, &str, &str)]) -> AppTool {
    let mut properties = serde_json::Map::new();
    for (n, typ, desc) in fields {
        let mut p = json!({ "type": typ, "description": desc });
        if *typ == "array" {
            p["items"] = json!({ "type": "string" });
        }
        properties.insert((*n).to_string(), p);
    }
    AppTool {
        name: name.into(),
        description: description.into(),
        input_schema: json!({ "type": "object", "properties": properties, "required": required }),
        annotations: json!({ "readOnlyHint": read_only, "destructiveHint": false }),
    }
}

fn gmail_tools() -> Vec<AppTool> {
    vec![
        tool("search_threads", "List or search threads in the person's mailbox. `query` is Gmail search syntax (from:, subject:, newer_than:); leave it empty to list recent threads.", true, &[], &[("query", "string", "Gmail search query"), ("pageSize", "integer", "How many threads, at most 50"), ("pageToken", "string", "Page token"), ("includeTrash", "boolean", "Include the trash"), ("view", "string", "Accepted and ignored: the list returns ids and snippets")]),
        tool("get_thread", "Read one thread.", true, &["threadId"], &[("threadId", "string", "Thread id"), ("messageFormat", "string", "FULL_CONTENT, PLAIN_TEXT, METADATA_ONLY, MINIMAL or RAW")]),
        tool("get_message", "Read one message.", true, &["messageId"], &[("messageId", "string", "Message id"), ("messageFormat", "string", "FULL_CONTENT, PLAIN_TEXT, METADATA_ONLY, MINIMAL or RAW")]),
        tool("list_drafts", "List drafts.", true, &[], &[("query", "string", "Gmail search query"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token")]),
        tool("get_draft", "Read one draft.", true, &["draftId"], &[("draftId", "string", "Draft id"), ("messageFormat", "string", "FULL_CONTENT, PLAIN_TEXT, METADATA_ONLY, MINIMAL or RAW")]),
        tool("list_labels", "List mailbox labels.", true, &[], &[]),
        tool("create_draft", "Create a draft. Does not send it. `to`, `cc` and `bcc` are arrays of addresses. `attachments` are objects with content (base64), filename and mimeType.", false, &[], &[("to", "array", "To"), ("cc", "array", "Cc"), ("bcc", "array", "Bcc"), ("subject", "string", "Subject"), ("body", "string", "Plain text"), ("htmlBody", "string", "HTML body"), ("replyToMessageId", "string", "Message this replies to"), ("includeSignature", "boolean", "Accepted; the mailbox signature is not appended")]),
        tool("send_draft", "Send a draft that already exists. Sending cannot be undone.", false, &["draftId"], &[("draftId", "string", "Draft id")]),
        tool("create_label", "Create a label.", false, &["displayName"], &[("displayName", "string", "Label name"), ("labelListVisibility", "string", "labelShow, labelShowIfUnread or labelHide"), ("messageListVisibility", "string", "show or hide")]),
        tool("label_message", "Add labels to a message.", false, &["messageId", "labelIds"], &[("messageId", "string", "Message id"), ("labelIds", "array", "Label ids")]),
        tool("unlabel_message", "Remove labels from a message.", false, &["messageId", "labelIds"], &[("messageId", "string", "Message id"), ("labelIds", "array", "Label ids")]),
        tool("label_thread", "Add labels to a thread.", false, &["threadId", "labelIds"], &[("threadId", "string", "Thread id"), ("labelIds", "array", "Label ids")]),
        tool("unlabel_thread", "Remove labels from a thread.", false, &["threadId", "labelIds"], &[("threadId", "string", "Thread id"), ("labelIds", "array", "Label ids")]),
        tool("update_message_labels", "Add and remove labels on a message.", false, &["messageId"], &[("messageId", "string", "Message id"), ("addLabelIds", "array", "Labels to add"), ("removeLabelIds", "array", "Labels to remove")]),
        tool("trash_message", "Move a message to the trash.", false, &["messageId"], &[("messageId", "string", "Message id")]),
        tool("untrash_message", "Take a message out of the trash.", false, &["messageId"], &[("messageId", "string", "Message id")]),
        tool("trash_thread", "Move a thread to the trash.", false, &["threadId"], &[("threadId", "string", "Thread id")]),
        tool("untrash_thread", "Take a thread out of the trash.", false, &["threadId"], &[("threadId", "string", "Thread id")]),
        tool("mark_message_spam", "Mark a message as spam.", false, &["messageId"], &[("messageId", "string", "Message id")]),
        tool("unmark_message_spam", "Unmark a message as spam.", false, &["messageId"], &[("messageId", "string", "Message id")]),
        tool("mark_thread_spam", "Mark a thread as spam.", false, &["threadId"], &[("threadId", "string", "Thread id")]),
        tool("unmark_thread_spam", "Unmark a thread as spam.", false, &["threadId"], &[("threadId", "string", "Thread id")]),
        tool("apply_sensitive_message_label", "Move a message to the trash or mark it spam. `labelOption` is TRASH or SPAM.", false, &["messageId", "labelOption"], &[("messageId", "string", "Message id"), ("labelOption", "string", "TRASH or SPAM")]),
        tool("apply_sensitive_thread_label", "Move a thread to the trash or mark it spam. `labelOption` is TRASH or SPAM.", false, &["threadId", "labelOption"], &[("threadId", "string", "Thread id"), ("labelOption", "string", "TRASH or SPAM")]),
    ]
}

fn drive_tools() -> Vec<AppTool> {
    vec![
        tool("search_files", "Search Drive. `query` is Drive search syntax (name contains 'report').", true, &[], &[("query", "string", "Drive query"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token")]),
        tool("list_recent_files", "List recently changed files.", true, &[], &[("pageSize", "integer", "How many"), ("pageToken", "string", "Page token"), ("orderBy", "string", "Sort, default modifiedTime desc")]),
        tool("get_file_metadata", "Read a file's name, type and link.", true, &["fileId"], &[("fileId", "string", "File id")]),
        tool("get_file_permissions", "List who can access a file.", true, &["fileId"], &[("fileId", "string", "File id")]),
        tool("read_file_content", "Read a file as text. Documents, sheets and slides are exported.", true, &["fileId"], &[("fileId", "string", "File id")]),
        tool("download_file_content", "Download a file as base64. `exportMimeType` chooses the export for a Google file.", true, &["fileId"], &[("fileId", "string", "File id"), ("exportMimeType", "string", "Export type, such as text/plain")]),
        tool("create_file", "Create a file. `textContent` or `base64Content`, with `contentMimeType` and `title`.", false, &["title"], &[("title", "string", "Name"), ("textContent", "string", "Text body"), ("base64Content", "string", "Base64 body"), ("contentMimeType", "string", "Media type"), ("parentId", "string", "Folder id")]),
        tool("copy_file", "Copy a file.", false, &["fileId"], &[("fileId", "string", "File id"), ("title", "string", "New name"), ("parentId", "string", "Folder id")]),
    ]
}

fn calendar_tools() -> Vec<AppTool> {
    vec![
        tool("list_calendars", "List calendars.", true, &[], &[]),
        tool("list_events", "List events on a calendar between two times.", true, &[], &[("calendarId", "string", "Calendar id, default primary"), ("startTime", "string", "Start, RFC3339"), ("endTime", "string", "End, RFC3339"), ("fullText", "string", "Words to match"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token"), ("timeZone", "string", "IANA time zone"), ("orderBy", "string", "startTime or updated")]),
        tool("search_events", "Search events by words.", true, &["query"], &[("query", "string", "Words"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token")]),
        tool("get_event", "Read one event.", true, &["eventId"], &[("eventId", "string", "Event id"), ("calendarId", "string", "Calendar id")]),
        tool("suggest_time", "Find a free slot for the attendees between two times.", true, &["attendeeEmails", "startTime", "endTime"], &[("attendeeEmails", "array", "Email addresses"), ("startTime", "string", "Window start, RFC3339"), ("endTime", "string", "Window end, RFC3339"), ("durationMinutes", "integer", "Length of the slot, default 30"), ("timeZone", "string", "IANA time zone")]),
        tool("create_event", "Add an event. Invited people are told. This cannot be undone.", false, &["summary", "startTime", "endTime"], &[("summary", "string", "Title"), ("startTime", "string", "Start"), ("endTime", "string", "End"), ("calendarId", "string", "Calendar id"), ("description", "string", "Description"), ("location", "string", "Place"), ("attendeeEmails", "array", "People to invite"), ("timeZone", "string", "IANA time zone"), ("allDay", "boolean", "All day"), ("notificationLevel", "string", "NONE, EXTERNAL_ONLY or ALL")]),
        tool("update_event", "Change an event. People already invited may be told.", false, &["eventId"], &[("eventId", "string", "Event id"), ("calendarId", "string", "Calendar id"), ("summary", "string", "New title"), ("startTime", "string", "New start"), ("endTime", "string", "New end"), ("description", "string", "New description"), ("location", "string", "New place"), ("timeZone", "string", "IANA time zone"), ("notificationLevel", "string", "NONE, EXTERNAL_ONLY or ALL")]),
        tool("delete_event", "Cancel an event. People invited may be told.", false, &["eventId"], &[("eventId", "string", "Event id"), ("calendarId", "string", "Calendar id"), ("notificationLevel", "string", "NONE, EXTERNAL_ONLY or ALL")]),
        tool("respond_to_event", "Answer an invitation: accepted, declined or tentative.", false, &["eventId", "responseStatus"], &[("eventId", "string", "Event id"), ("responseStatus", "string", "accepted, declined or tentative"), ("calendarId", "string", "Calendar id"), ("responseComment", "string", "Note"), ("notificationLevel", "string", "NONE, EXTERNAL_ONLY or ALL")]),
    ]
}

fn docs_tools() -> Vec<AppTool> {
    vec![tool("read_doc", "Read a Google document as text.", true, &["documentId"], &[("documentId", "string", "Document id, the same id Drive uses")])]
}

fn sheets_tools() -> Vec<AppTool> {
    vec![
        tool("get_spreadsheet", "Read a spreadsheet's title and sheet names.", true, &["spreadsheetId"], &[("spreadsheetId", "string", "Spreadsheet id")]),
        tool("get_values", "Read the cells in a range, in A1 notation (Sheet1!A1:C10).", true, &["spreadsheetId", "range"], &[("spreadsheetId", "string", "Spreadsheet id"), ("range", "string", "A1 range")]),
    ]
}

fn chat_tools() -> Vec<AppTool> {
    vec![
        tool("list_messages", "List messages in a Chat space.", true, &["conversationId"], &[("conversationId", "string", "Space id, such as spaces/AAA"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token"), ("startTime", "string", "Only messages after this RFC3339 time"), ("endTime", "string", "Only messages before this RFC3339 time")]),
        tool("search_conversations", "List Chat spaces, optionally by name.", true, &[], &[("spaceNameQuery", "string", "Words in the space name"), ("pageSize", "integer", "How many"), ("pageToken", "string", "Page token")]),
        AppTool {
            name: "search_messages".into(),
            description: "List messages in one space. searchParameters.conversationId is the space; searchParameters.query keeps messages whose text contains it.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["searchParameters"],
                "properties": { "searchParameters": { "type": "object", "properties": {
                    "conversationId": { "type": "string", "description": "Space id, such as spaces/AAA" },
                    "query": { "type": "string", "description": "Words to keep" }
                }, "required": ["conversationId"] } }
            }),
            annotations: json!({ "readOnlyHint": true, "destructiveHint": false }),
        },
        tool("send_message", "Send a message to a space. Sending cannot be undone.", false, &["conversationId", "messageText"], &[("conversationId", "string", "Space id"), ("messageText", "string", "The message"), ("threadId", "string", "Thread to reply in")]),
    ]
}

struct Api<'a> {
    http: &'a reqwest::Client,
    base: String,
    bearer: String,
    what: &'static str,
}

impl Api<'_> {
    async fn go(&self, method: reqwest::Method, path: &str, body: Option<&Value>, wrote: bool) -> Result<Resp, CallError> {
        let url = format!("{}{}", self.base.trim_end_matches('/'), path);
        let mut req = self.http.request(method, &url).bearer_auth(&self.bearer);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_builder() {
                CallError::NotSent(format!("could not reach {}: {e}", self.what))
            } else if wrote {
                CallError::Unknown(format!("the request to {} did not finish: {e}", self.what))
            } else {
                CallError::NotSent(format!("could not reach {}: {e}", self.what))
            }
        })?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| CallError::Unknown(format!("response lost: {e}")))?;
        if bytes.len() > 4_000_000 {
            return Err(CallError::NotSent(format!("{} returned more than this path will carry. Nothing was changed.", self.what)));
        }
        Ok(Resp { status, bytes: bytes.to_vec(), what: self.what, wrote })
    }
}

struct Resp {
    status: reqwest::StatusCode,
    bytes: Vec<u8>,
    what: &'static str,
    wrote: bool,
}

impl Resp {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    fn json(&self) -> Result<Value, CallError> {
        if !self.status.is_success() {
            return Err(explain(self.status, &self.text(), self.what, self.wrote));
        }
        if self.bytes.is_empty() {
            return Ok(json!({}));
        }
        Ok(serde_json::from_slice(&self.bytes).unwrap_or_else(|_| json!({ "text": self.text() })))
    }
    fn raw(&self) -> Result<String, CallError> {
        if !self.status.is_success() {
            return Err(explain(self.status, &self.text(), self.what, self.wrote));
        }
        Ok(self.text())
    }
}

fn out(v: Value) -> CallOutput {
    let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
    CallOutput { text, structured: Some(v), is_error: false }
}

fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, CallError> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| CallError::NotSent(format!("{key} is required. Nothing was changed.")))
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    match &v[key] {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    }
}

fn num(v: &Value, key: &str, default: u64, max: u64) -> u64 {
    let n = v[key].as_u64().or_else(|| v[key].as_i64().map(|n| n.max(0) as u64)).unwrap_or(default);
    n.clamp(1, max)
}

fn q(pairs: &[(&str, String)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let mut u = url::Url::parse("http://local/").unwrap();
    {
        let mut s = u.query_pairs_mut();
        for (k, v) in pairs {
            if !v.is_empty() {
                s.append_pair(k, v);
            }
        }
    }
    u.query().map(|s| format!("?{s}")).unwrap_or_default()
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub async fn call(http: &reqwest::Client, url: &str, bearer: Option<&str>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    let Some(product) = product(url) else {
        return Err(CallError::NotSent("not a Google API".into()));
    };
    let Some(bearer) = bearer.filter(|s| !s.is_empty()) else {
        return Err(CallError::Reauth(format!("{} has no working sign-in. Sign in again. Nothing was changed.", product.name())));
    };
    let api = Api { http, base: url.trim_end_matches('/').to_string(), bearer: bearer.to_string(), what: product.name() };
    if url.contains("mcp.googleapis.com") {
        return Err(CallError::NotSent(format!("{} would have called a preview server. Nothing was changed.", product.name())));
    }
    match product {
        Product::Gmail => gmail(&api, tool, args).await,
        Product::Drive => drive(&api, tool, args).await,
        Product::Calendar => calendar(&api, tool, args).await,
        Product::Docs => docs(&api, tool, args).await,
        Product::Sheets => sheets(&api, tool, args).await,
        Product::Chat => chat(&api, tool, args).await,
    }
}

async fn gmail(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    match tool {
        "search_threads" => {
            let mut pairs = vec![
                ("q", args["query"].as_str().unwrap_or("").to_string()),
                ("maxResults", num(args, "pageSize", 20, 50).to_string()),
                ("pageToken", args["pageToken"].as_str().unwrap_or("").to_string()),
            ];
            if args["includeTrash"] == true {
                pairs.push(("includeSpamTrash", "true".into()));
            }
            let v = api.go(reqwest::Method::GET, &format!("/users/me/threads{}", q(&pairs)), None, false).await?.json()?;
            Ok(out(v))
        }
        "get_thread" => message_view(api, "threads", need(args, "threadId")?, args["messageFormat"].as_str().unwrap_or("FULL_CONTENT")).await,
        "get_message" => message_view(api, "messages", need(args, "messageId")?, args["messageFormat"].as_str().unwrap_or("FULL_CONTENT")).await,
        "get_draft" => {
            let id = need(args, "draftId")?;
            let v = api.go(reqwest::Method::GET, &format!("/users/me/drafts/{id}?format=full"), None, false).await?.json()?;
            Ok(out(present_message(v.get("message").unwrap_or(&v), args["messageFormat"].as_str().unwrap_or("FULL_CONTENT"))))
        }
        "list_drafts" => {
            let pairs = [
                ("q", args["query"].as_str().unwrap_or("").to_string()),
                ("maxResults", num(args, "pageSize", 20, 50).to_string()),
                ("pageToken", args["pageToken"].as_str().unwrap_or("").to_string()),
            ];
            Ok(out(api.go(reqwest::Method::GET, &format!("/users/me/drafts{}", q(&pairs)), None, false).await?.json()?))
        }
        "list_labels" => Ok(out(api.go(reqwest::Method::GET, "/users/me/labels", None, false).await?.json()?)),
        "create_draft" => {
            let (raw, thread) = draft_raw(api, args).await?;
            let mut message = json!({ "raw": raw });
            if let Some(t) = thread {
                message["threadId"] = json!(t);
            }
            let v = api.go(reqwest::Method::POST, "/users/me/drafts", Some(&json!({ "message": message })), true).await?.json()?;
            let mut v = v;
            if args["includeSignature"] == true {
                v["signature"] = json!("the mailbox signature is not added on this path");
            }
            Ok(out(v))
        }
        "send_draft" => {
            let v = api.go(reqwest::Method::POST, "/users/me/drafts/send", Some(&json!({ "id": need(args, "draftId")? })), true).await?.json()?;
            Ok(out(v))
        }
        "create_label" => {
            let body = json!({
                "name": need(args, "displayName")?,
                "labelListVisibility": args["labelListVisibility"].as_str().unwrap_or("labelShow"),
                "messageListVisibility": args["messageListVisibility"].as_str().unwrap_or("show"),
            });
            Ok(out(api.go(reqwest::Method::POST, "/users/me/labels", Some(&body), true).await?.json()?))
        }
        "label_message" => modify(api, "messages", need(args, "messageId")?, &strings(args, "labelIds"), &[]).await,
        "unlabel_message" => modify(api, "messages", need(args, "messageId")?, &[], &strings(args, "labelIds")).await,
        "label_thread" => modify(api, "threads", need(args, "threadId")?, &strings(args, "labelIds"), &[]).await,
        "unlabel_thread" => modify(api, "threads", need(args, "threadId")?, &[], &strings(args, "labelIds")).await,
        "update_message_labels" => modify(api, "messages", need(args, "messageId")?, &strings(args, "addLabelIds"), &strings(args, "removeLabelIds")).await,
        "trash_message" => trash(api, "messages", need(args, "messageId")?, true).await,
        "untrash_message" => trash(api, "messages", need(args, "messageId")?, false).await,
        "trash_thread" => trash(api, "threads", need(args, "threadId")?, true).await,
        "untrash_thread" => trash(api, "threads", need(args, "threadId")?, false).await,
        "mark_message_spam" => modify(api, "messages", need(args, "messageId")?, &["SPAM".into()], &["INBOX".into()]).await,
        "unmark_message_spam" => modify(api, "messages", need(args, "messageId")?, &["INBOX".into()], &["SPAM".into()]).await,
        "mark_thread_spam" => modify(api, "threads", need(args, "threadId")?, &["SPAM".into()], &["INBOX".into()]).await,
        "unmark_thread_spam" => modify(api, "threads", need(args, "threadId")?, &["INBOX".into()], &["SPAM".into()]).await,
        "apply_sensitive_message_label" => sensitive(api, "messages", need(args, "messageId")?, need(args, "labelOption")?).await,
        "apply_sensitive_thread_label" => sensitive(api, "threads", need(args, "threadId")?, need(args, "labelOption")?).await,
        other => Err(CallError::NotSent(format!("Gmail has no tool {other}"))),
    }
}

async fn modify(api: &Api<'_>, kind: &str, id: &str, add: &[String], remove: &[String]) -> Result<CallOutput, CallError> {
    let body = json!({ "addLabelIds": add, "removeLabelIds": remove });
    Ok(out(api.go(reqwest::Method::POST, &format!("/users/me/{kind}/{id}/modify"), Some(&body), true).await?.json()?))
}

async fn trash(api: &Api<'_>, kind: &str, id: &str, to_trash: bool) -> Result<CallOutput, CallError> {
    let op = if to_trash { "trash" } else { "untrash" };
    Ok(out(api.go(reqwest::Method::POST, &format!("/users/me/{kind}/{id}/{op}"), Some(&json!({})), true).await?.json()?))
}

async fn sensitive(api: &Api<'_>, kind: &str, id: &str, option: &str) -> Result<CallOutput, CallError> {
    match option {
        "TRASH" => trash(api, kind, id, true).await,
        "SPAM" => modify(api, kind, id, &["SPAM".into()], &["INBOX".into()]).await,
        _ => Err(CallError::NotSent("labelOption must be TRASH or SPAM. Nothing was changed.".into())),
    }
}

async fn message_view(api: &Api<'_>, kind: &str, id: &str, format: &str) -> Result<CallOutput, CallError> {
    let v = api.go(reqwest::Method::GET, &format!("/users/me/{kind}/{id}?format=full"), None, false).await?.json()?;
    if kind == "threads" {
        let messages = v["messages"].as_array().map(|ms| ms.iter().map(|m| present_message(m, format)).collect::<Vec<_>>()).unwrap_or_default();
        return Ok(out(json!({ "id": v["id"], "messages": messages })));
    }
    Ok(out(present_message(&v, format)))
}

fn present_message(msg: &Value, format: &str) -> Value {
    let header = |name: &str| {
        msg["payload"]["headers"].as_array().and_then(|hs| hs.iter().find(|h| h["name"].as_str().unwrap_or("").eq_ignore_ascii_case(name))).and_then(|h| h["value"].as_str()).unwrap_or("").to_string()
    };
    let (mut plain, mut html) = (String::new(), String::new());
    let mut attachments = vec![];
    walk_part(&msg["payload"], &mut plain, &mut html, &mut attachments);
    if plain.is_empty() && !html.is_empty() {
        plain = html2text::from_read(html.as_bytes(), 90).unwrap_or_default();
    }
    let id = msg["id"].as_str().unwrap_or("");
    let mut v = json!({
        "id": id,
        "threadId": msg["threadId"],
        "snippet": msg["snippet"],
        "subject": header("Subject"),
        "sender": header("From"),
        "to": header("To"),
        "cc": header("Cc"),
        "date": header("Date"),
        "labelIds": msg["labelIds"],
        "viewUrl": format!("https://mail.google.com/mail/u/0/#all/{id}"),
    });
    match format {
        "MINIMAL" | "METADATA_ONLY" => {}
        "RAW" => {
            v["raw"] = msg["raw"].clone();
        }
        "PLAIN_TEXT" => {
            v["plaintext"] = json!(plain);
            v["attachments"] = json!(attachments);
        }
        _ => {
            v["plaintext"] = json!(plain);
            v["html"] = json!(html);
            v["attachments"] = json!(attachments);
        }
    }
    v
}

fn walk_part(part: &Value, plain: &mut String, html: &mut String, attachments: &mut Vec<Value>) {
    let mime = part["mimeType"].as_str().unwrap_or("");
    if let Some(data) = part["body"]["data"].as_str() {
        let text = decode_b64(data);
        if mime == "text/plain" {
            plain.push_str(&text);
        } else if mime == "text/html" {
            html.push_str(&text);
        }
    }
    if let Some(id) = part["body"]["attachmentId"].as_str() {
        attachments.push(json!({ "id": id, "filename": part["filename"], "mimeType": mime }));
    }
    if let Some(parts) = part["parts"].as_array() {
        for p in parts {
            walk_part(p, plain, html, attachments);
        }
    }
}

fn decode_b64(s: &str) -> String {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(s)).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn header_safe(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

async fn draft_raw(api: &Api<'_>, args: &Value) -> Result<(String, Option<String>), CallError> {
    let mut extra = String::new();
    let mut thread = None;
    if let Some(reply) = args["replyToMessageId"].as_str().filter(|s| !s.is_empty()) {
        if let Ok(msg) = api.go(reqwest::Method::GET, &format!("/users/me/messages/{reply}?format=metadata&metadataHeaders=Message-ID&metadataHeaders=Subject&metadataHeaders=References"), None, false).await {
            if let Ok(v) = msg.json() {
                thread = v["threadId"].as_str().map(String::from);
                let mid = v["payload"]["headers"].as_array().and_then(|hs| hs.iter().find(|h| h["name"].as_str().unwrap_or("").eq_ignore_ascii_case("Message-ID"))).and_then(|h| h["value"].as_str()).unwrap_or("");
                if !mid.is_empty() {
                    extra.push_str(&format!("In-Reply-To: {}\r\nReferences: {}\r\n", header_safe(mid), header_safe(mid)));
                }
            }
        }
    }
    let to = strings(args, "to").join(", ");
    let cc = strings(args, "cc").join(", ");
    let bcc = strings(args, "bcc").join(", ");
    let subject = header_safe(args["subject"].as_str().unwrap_or(""));
    let body = args["body"].as_str().unwrap_or("");
    let html = args["htmlBody"].as_str().unwrap_or("");
    let mut headers = format!("To: {to}\r\n");
    if !cc.is_empty() {
        headers.push_str(&format!("Cc: {cc}\r\n"));
    }
    if !bcc.is_empty() {
        headers.push_str(&format!("Bcc: {bcc}\r\n"));
    }
    headers.push_str(&format!("Subject: {subject}\r\n{extra}MIME-Version: 1.0\r\n"));
    let raw = if !html.is_empty() {
        let boundary = "magnus-draft";
        format!("{headers}Content-Type: multipart/alternative; boundary={boundary}\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--{boundary}--\r\n")
    } else {
        format!("{headers}Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n")
    };
    Ok((b64url(raw.as_bytes()), thread))
}

async fn drive(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    let fields = "files(id,name,mimeType,modifiedTime,webViewLink),nextPageToken";
    match tool {
        "search_files" => {
            let pairs = [("q", args["query"].as_str().unwrap_or("").to_string()), ("pageSize", num(args, "pageSize", 20, 100).to_string()), ("pageToken", args["pageToken"].as_str().unwrap_or("").to_string()), ("fields", fields.into())];
            Ok(out(api.go(reqwest::Method::GET, &format!("/files{}", q(&pairs)), None, false).await?.json()?))
        }
        "list_recent_files" => {
            let order = args["orderBy"].as_str().unwrap_or("modifiedTime desc");
            let pairs = [("orderBy", order.to_string()), ("pageSize", num(args, "pageSize", 20, 100).to_string()), ("pageToken", args["pageToken"].as_str().unwrap_or("").to_string()), ("fields", fields.into())];
            Ok(out(api.go(reqwest::Method::GET, &format!("/files{}", q(&pairs)), None, false).await?.json()?))
        }
        "get_file_metadata" => {
            let id = need(args, "fileId")?;
            Ok(out(api.go(reqwest::Method::GET, &format!("/files/{id}?fields=id,name,mimeType,modifiedTime,size,parents,webViewLink"), None, false).await?.json()?))
        }
        "get_file_permissions" => {
            let id = need(args, "fileId")?;
            Ok(out(api.go(reqwest::Method::GET, &format!("/files/{id}/permissions"), None, false).await?.json()?))
        }
        "read_file_content" => {
            let id = need(args, "fileId")?;
            let meta = api.go(reqwest::Method::GET, &format!("/files/{id}?fields=id,name,mimeType"), None, false).await?.json()?;
            let mime = meta["mimeType"].as_str().unwrap_or("");
            let export = export_type(mime);
            let text = if let Some(mt) = export {
                api.go(reqwest::Method::GET, &format!("/files/{id}/export?mimeType={}", enc(mt)), None, false).await?.raw()?
            } else {
                api.go(reqwest::Method::GET, &format!("/files/{id}?alt=media"), None, false).await?.raw()?
            };
            Ok(out(json!({ "id": id, "name": meta["name"], "mimeType": mime, "text": clip_text(&text) })))
        }
        "download_file_content" => {
            let id = need(args, "fileId")?;
            let export = args["exportMimeType"].as_str().filter(|s| !s.is_empty());
            let path = match export {
                Some(mt) => format!("/files/{id}/export?mimeType={}", enc(mt)),
                None => format!("/files/{id}?alt=media"),
            };
            let resp = api.go(reqwest::Method::GET, &path, None, false).await?;
            if !resp.status.is_success() {
                return Err(explain(resp.status, &resp.text(), api.what, false));
            }
            Ok(out(json!({ "id": id, "base64": b64url(&resp.bytes) })))
        }
        "create_file" => {
            let title = need(args, "title")?;
            let parent = args["parentId"].as_str().filter(|s| !s.is_empty());
            let mut meta = json!({ "name": title });
            if let Some(p) = parent {
                meta["parents"] = json!([p]);
            }
            if let Some(mt) = args["contentMimeType"].as_str() {
                meta["mimeType"] = json!(mt);
            }
            let bytes = if let Some(b) = args["base64Content"].as_str().filter(|s| !s.is_empty()) {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(b).map_err(|_| CallError::NotSent("base64Content is not base64. Nothing was changed.".into()))?
            } else if let Some(t) = args["textContent"].as_str() {
                t.as_bytes().to_vec()
            } else {
                return Ok(out(api.go(reqwest::Method::POST, "/files", Some(&meta), true).await?.json()?));
            };
            let mime = args["contentMimeType"].as_str().unwrap_or("text/plain");
            let boundary = "magnus-upload";
            let head = format!("--{boundary}\r\nContent-Type: application/json; charset=utf-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n", meta);
            let mut body = head.into_bytes();
            body.extend_from_slice(&bytes);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let origin = url::Url::parse(&api.base).ok().map(|u| u.origin().ascii_serialization()).unwrap_or_else(|| "https://www.googleapis.com".into());
            let url = format!("{origin}/upload/drive/v3/files?uploadType=multipart");
            let resp = api
                .http
                .post(&url)
                .bearer_auth(&api.bearer)
                .header("content-type", format!("multipart/related; boundary={boundary}"))
                .body(body)
                .send()
                .await
                .map_err(|e| CallError::Unknown(format!("the upload to Drive did not finish: {e}")))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(explain(status, &text, api.what, true));
            }
            Ok(out(serde_json::from_str(&text).unwrap_or(json!({ "text": text }))))
        }
        "copy_file" => {
            let id = need(args, "fileId")?;
            let mut body = json!({});
            if let Some(t) = args["title"].as_str() {
                body["name"] = json!(t);
            }
            if let Some(p) = args["parentId"].as_str().filter(|s| !s.is_empty()) {
                body["parents"] = json!([p]);
            }
            Ok(out(api.go(reqwest::Method::POST, &format!("/files/{id}/copy"), Some(&body), true).await?.json()?))
        }
        other => Err(CallError::NotSent(format!("Drive has no tool {other}"))),
    }
}

fn export_type(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        m if m.starts_with("application/vnd.google-apps.") => Some("text/plain"),
        _ => None,
    }
}

fn clip_text(s: &str) -> String {
    if s.chars().count() <= 100_000 {
        return s.to_string();
    }
    format!("{}\n\n(truncated)", s.chars().take(100_000).collect::<String>())
}

fn cal_id(args: &Value) -> String {
    args["calendarId"].as_str().filter(|s| !s.is_empty()).unwrap_or("primary").to_string()
}

fn send_updates(args: &Value) -> &'static str {
    match args["notificationLevel"].as_str().unwrap_or("ALL") {
        "NONE" => "none",
        "EXTERNAL_ONLY" => "externalOnly",
        _ => "all",
    }
}

fn when(args: &Value, key: &str) -> Value {
    let t = args[key].as_str().unwrap_or("");
    let zone = args["timeZone"].as_str().unwrap_or("");
    if args["allDay"] == true {
        json!({ "date": t.chars().take(10).collect::<String>() })
    } else if zone.is_empty() {
        json!({ "dateTime": t })
    } else {
        json!({ "dateTime": t, "timeZone": zone })
    }
}

async fn calendar(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    match tool {
        "list_calendars" => Ok(out(api.go(reqwest::Method::GET, "/users/me/calendarList", None, false).await?.json()?)),
        "list_events" => {
            let id = enc(&cal_id(args));
            let mut pairs = vec![
                ("timeMin", rfc3339(args["startTime"].as_str().unwrap_or(""))),
                ("timeMax", rfc3339(args["endTime"].as_str().unwrap_or(""))),
                ("q", args["fullText"].as_str().unwrap_or("").into()),
                ("maxResults", num(args, "pageSize", 20, 250).to_string()),
                ("pageToken", args["pageToken"].as_str().unwrap_or("").into()),
                ("singleEvents", "true".into()),
                ("orderBy", if args["orderBy"].as_str() == Some("updated") { "updated".into() } else { "startTime".into() }),
            ];
            if let Some(z) = args["timeZone"].as_str() {
                pairs.push(("timeZone", z.into()));
            }
            Ok(out(api.go(reqwest::Method::GET, &format!("/calendars/{id}/events{}", q(&pairs)), None, false).await?.json()?))
        }
        "search_events" => {
            let pairs = [("q", need(args, "query")?.to_string()), ("maxResults", num(args, "pageSize", 20, 50).to_string()), ("pageToken", args["pageToken"].as_str().unwrap_or("").into()), ("singleEvents", "true".into())];
            Ok(out(api.go(reqwest::Method::GET, &format!("/calendars/primary/events{}", q(&pairs)), None, false).await?.json()?))
        }
        "get_event" => {
            let id = enc(&cal_id(args));
            let ev = enc(need(args, "eventId")?);
            Ok(out(api.go(reqwest::Method::GET, &format!("/calendars/{id}/events/{ev}"), None, false).await?.json()?))
        }
        "suggest_time" => suggest(api, args).await,
        "create_event" => {
            let id = enc(&cal_id(args));
            let body = event_body(args, true)?;
            let updates = send_updates(args);
            Ok(out(api.go(reqwest::Method::POST, &format!("/calendars/{id}/events?sendUpdates={updates}"), Some(&body), true).await?.json()?))
        }
        "update_event" => {
            let id = enc(&cal_id(args));
            let ev = enc(need(args, "eventId")?);
            let body = event_body(args, false)?;
            let updates = send_updates(args);
            Ok(out(api.go(reqwest::Method::PATCH, &format!("/calendars/{id}/events/{ev}?sendUpdates={updates}"), Some(&body), true).await?.json()?))
        }
        "delete_event" => {
            let id = enc(&cal_id(args));
            let ev = enc(need(args, "eventId")?);
            let updates = send_updates(args);
            let _ = api.go(reqwest::Method::DELETE, &format!("/calendars/{id}/events/{ev}?sendUpdates={updates}"), None, true).await?.json()?;
            Ok(out(json!({ "deleted": ev })))
        }
        "respond_to_event" => respond(api, args).await,
        other => Err(CallError::NotSent(format!("Calendar has no tool {other}"))),
    }
}

fn rfc3339(s: &str) -> String {
    if s.is_empty() || s.contains('Z') || s.contains('+') || s.matches('-').count() > 2 {
        return s.to_string();
    }
    format!("{s}Z")
}

fn event_body(args: &Value, creating: bool) -> Result<Value, CallError> {
    let mut body = json!({});
    if creating {
        body["summary"] = json!(need(args, "summary")?);
        body["start"] = when(args, "startTime");
        body["end"] = when(args, "endTime");
    } else {
        if let Some(s) = args["summary"].as_str() {
            body["summary"] = json!(s);
        }
        if args["startTime"].as_str().is_some() {
            body["start"] = when(args, "startTime");
        }
        if args["endTime"].as_str().is_some() {
            body["end"] = when(args, "endTime");
        }
    }
    if let Some(d) = args["description"].as_str() {
        body["description"] = json!(d);
    }
    if let Some(l) = args["location"].as_str() {
        body["location"] = json!(l);
    }
    let people = strings(args, "attendeeEmails");
    if !people.is_empty() {
        body["attendees"] = json!(people.into_iter().map(|email| json!({ "email": email })).collect::<Vec<_>>());
    }
    Ok(body)
}

async fn suggest(api: &Api<'_>, args: &Value) -> Result<CallOutput, CallError> {
    let emails = strings(args, "attendeeEmails");
    if emails.is_empty() {
        return Err(CallError::NotSent("attendeeEmails is required. Nothing was changed.".into()));
    }
    let start = rfc3339(need(args, "startTime")?);
    let end = rfc3339(need(args, "endTime")?);
    let minutes = num(args, "durationMinutes", 30, 24 * 60);
    let body = json!({ "timeMin": start, "timeMax": end, "items": emails.iter().map(|e| json!({ "id": e })).collect::<Vec<_>>() });
    let v = api.go(reqwest::Method::POST, "/freeBusy", Some(&body), false).await?.json()?;
    let mut busy: Vec<(i64, i64)> = vec![];
    if let Some(cals) = v["calendars"].as_object() {
        for cal in cals.values() {
            if let Some(slots) = cal["busy"].as_array() {
                for s in slots {
                    if let (Some(a), Some(b)) = (parse_time(s["start"].as_str().unwrap_or("")), parse_time(s["end"].as_str().unwrap_or(""))) {
                        busy.push((a, b));
                    }
                }
            }
        }
    }
    busy.sort_by_key(|s| s.0);
    let (Some(win_s), Some(win_e)) = (parse_time(&start), parse_time(&end)) else {
        return Ok(out(json!({ "freeBusy": v, "suggestions": [] })));
    };
    let need = minutes as i64 * 60;
    let mut cursor = win_s;
    let mut suggestions = vec![];
    for (a, b) in busy {
        if a - cursor >= need {
            suggestions.push(json!({ "start": chrono::DateTime::from_timestamp(cursor, 0), "end": chrono::DateTime::from_timestamp(cursor + need, 0) }));
        }
        if b > cursor {
            cursor = b;
        }
        if suggestions.len() >= 3 {
            break;
        }
    }
    if suggestions.len() < 3 && win_e - cursor >= need {
        suggestions.push(json!({ "start": chrono::DateTime::from_timestamp(cursor, 0), "end": chrono::DateTime::from_timestamp(cursor + need, 0) }));
    }
    Ok(out(json!({ "suggestions": suggestions })))
}

fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp()).or_else(|| chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S").ok().map(|n| n.and_utc().timestamp()))
}

async fn respond(api: &Api<'_>, args: &Value) -> Result<CallOutput, CallError> {
    let id = enc(&cal_id(args));
    let ev = enc(need(args, "eventId")?);
    let raw_status = need(args, "responseStatus")?.to_ascii_lowercase();
    let status = match raw_status.as_str() {
        "accepted" | "accept" | "yes" => "accepted",
        "declined" | "decline" | "no" => "declined",
        "tentative" | "maybe" => "tentative",
        other => return Err(CallError::NotSent(format!("responseStatus {other} is not accepted, declined or tentative. Nothing was changed."))),
    };
    let current = api.go(reqwest::Method::GET, &format!("/calendars/{id}/events/{ev}"), None, false).await?.json()?;
    let email = current["attendees"].as_array().and_then(|a| a.iter().find(|x| x["self"] == true)).and_then(|x| x["email"].as_str()).unwrap_or("").to_string();
    if email.is_empty() {
        return Err(CallError::NotSent("this calendar is not an attendee of that event, so there is no answer to give. Nothing was changed.".into()));
    }
    let mut attendee = json!({ "email": email, "responseStatus": status });
    if let Some(c) = args["responseComment"].as_str() {
        attendee["comment"] = json!(c);
    }
    let updates = send_updates(args);
    Ok(out(api.go(reqwest::Method::PATCH, &format!("/calendars/{id}/events/{ev}?sendUpdates={updates}"), Some(&json!({ "attendees": [attendee] })), true).await?.json()?))
}

async fn docs(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    if tool != "read_doc" {
        return Err(CallError::NotSent(format!("Docs has no tool {tool}")));
    }
    let id = need(args, "documentId")?;
    let v = api.go(reqwest::Method::GET, &format!("/documents/{}", enc(id)), None, false).await?.json()?;
    let mut text = String::new();
    if let Some(content) = v["body"]["content"].as_array() {
        doc_text(content, &mut text);
    }
    Ok(out(json!({ "documentId": v["documentId"], "title": v["title"], "text": clip_text(&text) })))
}

fn doc_text(nodes: &[Value], out: &mut String) {
    for n in nodes {
        if let Some(t) = n["textRun"]["content"].as_str() {
            out.push_str(t);
        }
        if let Some(els) = n["paragraph"]["elements"].as_array() {
            doc_text(els, out);
        }
        if let Some(rows) = n["table"]["tableRows"].as_array() {
            for row in rows {
                if let Some(cells) = row["tableCells"].as_array() {
                    for cell in cells {
                        if let Some(content) = cell["content"].as_array() {
                            doc_text(content, out);
                        }
                    }
                }
            }
        }
    }
}

async fn sheets(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    let id = enc(need(args, "spreadsheetId")?);
    match tool {
        "get_spreadsheet" => {
            let v = api.go(reqwest::Method::GET, &format!("/spreadsheets/{id}?fields=spreadsheetId,properties.title,sheets.properties"), None, false).await?.json()?;
            let names: Vec<String> = v["sheets"].as_array().into_iter().flatten().filter_map(|s| s["properties"]["title"].as_str().map(String::from)).collect();
            Ok(out(json!({ "spreadsheetId": v["spreadsheetId"], "title": v["properties"]["title"], "sheets": names })))
        }
        "get_values" => {
            let range = enc(need(args, "range")?);
            Ok(out(api.go(reqwest::Method::GET, &format!("/spreadsheets/{id}/values/{range}"), None, false).await?.json()?))
        }
        other => Err(CallError::NotSent(format!("Sheets has no tool {other}"))),
    }
}

fn space_of(id: &str) -> String {
    if id.starts_with("spaces/") { id.to_string() } else { format!("spaces/{id}") }
}

async fn chat(api: &Api<'_>, tool: &str, args: &Value) -> Result<CallOutput, CallError> {
    match tool {
        "list_messages" => {
            let space = space_of(need(args, "conversationId")?);
            let mut filter = vec![];
            if let Some(s) = args["startTime"].as_str().filter(|s| !s.is_empty()) {
                filter.push(format!("createTime > \"{}\"", rfc3339(s)));
            }
            if let Some(s) = args["endTime"].as_str().filter(|s| !s.is_empty()) {
                filter.push(format!("createTime < \"{}\"", rfc3339(s)));
            }
            let pairs = [("pageSize", num(args, "pageSize", 20, 100).to_string()), ("pageToken", args["pageToken"].as_str().unwrap_or("").into()), ("filter", filter.join(" AND "))];
            Ok(out(api.go(reqwest::Method::GET, &format!("/{space}/messages{}", q(&pairs)), None, false).await?.json()?))
        }
        "search_conversations" => {
            let pairs = [("pageSize", num(args, "pageSize", 20, 100).to_string()), ("pageToken", args["pageToken"].as_str().unwrap_or("").into())];
            let v = api.go(reqwest::Method::GET, &format!("/spaces{}", q(&pairs)), None, false).await?.json()?;
            let needle = args["spaceNameQuery"].as_str().unwrap_or("").to_ascii_lowercase();
            if needle.is_empty() {
                return Ok(out(v));
            }
            let spaces: Vec<Value> = v["spaces"].as_array().into_iter().flatten().filter(|s| s["displayName"].as_str().unwrap_or("").to_ascii_lowercase().contains(&needle)).cloned().collect();
            Ok(out(json!({ "spaces": spaces })))
        }
        "search_messages" => {
            let params = &args["searchParameters"];
            let space = params["conversationId"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| CallError::NotSent("searchParameters.conversationId is required: Chat's API lists messages inside one space. Nothing was changed.".into()))?;
            let query = params["query"].as_str().unwrap_or("").to_ascii_lowercase();
            let v = api.go(reqwest::Method::GET, &format!("/{}/messages?pageSize=50", space_of(space)), None, false).await?.json()?;
            if query.is_empty() {
                return Ok(out(v));
            }
            let messages: Vec<Value> = v["messages"].as_array().into_iter().flatten().filter(|m| m["text"].as_str().unwrap_or("").to_ascii_lowercase().contains(&query) || m.to_string().to_ascii_lowercase().contains(&query)).cloned().collect();
            Ok(out(json!({ "messages": messages })))
        }
        "send_message" => {
            let space = space_of(need(args, "conversationId")?);
            let mut body = json!({ "text": need(args, "messageText")? });
            let mut path = format!("/{space}/messages");
            if let Some(t) = args["threadId"].as_str().filter(|s| !s.is_empty()) {
                body["thread"] = json!({ "name": t });
                path.push_str("?messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD");
            }
            Ok(out(api.go(reqwest::Method::POST, &path, Some(&body), true).await?.json()?))
        }
        other => Err(CallError::NotSent(format!("Chat has no tool {other}"))),
    }
}

/// The signed-in account, from People API, when the token answer named none.
pub async fn account_from_people(http: &reqwest::Client, access: &str) -> Option<String> {
    let v: Value = http.get("https://people.googleapis.com/v1/people/me").query(&[("personFields", "emailAddresses,names")]).bearer_auth(access).send().await.ok()?.json().await.ok()?;
    v["emailAddresses"].as_array().and_then(|a| a.iter().find_map(|e| e["value"].as_str().map(String::from))).or_else(|| v["names"].as_array().and_then(|a| a.iter().find_map(|n| n["displayName"].as_str().map(String::from))))
}

/// Token endpoints that are ordinary OAuth, not an MCP resource server:
/// the `resource` parameter is not sent.
pub fn omit_resource(token_endpoint: &str) -> bool {
    const HOSTS: &[&str] = &[
        "googleapis.com",
        "accounts.google.com",
        "login.microsoftonline.com",
        "github.com",
        "slack.com",
        "vercel.com",
        "sentry.io",
        "asana.com",
        "gitlab.com",
        "huggingface.co",
        "supabase.com",
        "cloudflare.com",
        "myshopify.com",
    ];
    HOSTS.iter().any(|h| token_endpoint.contains(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, http::StatusCode, response::IntoResponse, Router};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Rec(Arc<Mutex<Vec<(String, String, String, String)>>>);

    async fn mock() -> (String, Rec) {
        let rec = Rec(Arc::new(Mutex::new(vec![])));
        let app = Router::new().fallback(handler).with_state(rec.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), rec)
    }

    async fn handler(State(rec): State<Rec>, req: axum::extract::Request) -> axum::response::Response {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = axum::body::to_bytes(req.into_body(), 2_000_000).await.unwrap_or(Bytes::new());
        let body = String::from_utf8_lossy(&bytes).to_string();
        rec.0.lock().unwrap().push((method.clone(), path.clone(), query.clone(), body.clone()));
        if auth == "Bearer fail-401" {
            return (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": { "code": 401, "message": "Invalid Credentials", "status": "UNAUTHENTICATED" } }))).into_response();
        }
        if auth == "Bearer fail-config" {
            return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": { "code": 403, "message": "Gmail API has not been used in project 49379713533 before or it is disabled.", "errors": [{ "reason": "accessNotConfigured" }] } }))).into_response();
        }
        if auth == "Bearer fail-scope" {
            return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": { "code": 403, "message": "Request had insufficient authentication scopes.", "status": "PERMISSION_DENIED", "details": [{ "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT" }] } }))).into_response();
        }
        if auth == "Bearer fail-preview" {
            return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": { "code": 403, "message": "Access to this tool requires that your Google Cloud project (49379713533) is enrolled in the Google Workspace Developer Preview Program." } }))).into_response();
        }
        let json = |v: Value| (StatusCode::OK, axum::Json(v)).into_response();
        if path.contains("/upload/") {
            return json(json!({ "id": "new-file", "name": "notes.txt" }));
        }
        if path.ends_with("/export") || query.contains("alt=media") {
            return (StatusCode::OK, "exported document text").into_response();
        }
        if path.contains("/threads") && method == "GET" && !path.contains("/threads/") {
            return json(json!({ "threads": [{ "id": "t1", "snippet": "hello from ada" }], "nextPageToken": "p2" }));
        }
        if path.contains("/messages/") && method == "GET" {
            return json(json!({
                "id": "m1", "threadId": "t1", "snippet": "hello",
                "payload": { "mimeType": "text/plain", "headers": [{ "name": "Subject", "value": "Hi" }, { "name": "From", "value": "Ada <ada@example.com>" }], "body": { "data": b64url(b"Hello there") } },
                "labelIds": ["INBOX"]
            }));
        }
        if path.ends_with("/drafts/send") {
            return json(json!({ "id": "m-sent", "labelIds": ["SENT"] }));
        }
        if path.ends_with("/drafts") && method == "POST" {
            return json(json!({ "id": "d1", "message": { "id": "m-draft" } }));
        }
        if path.contains("/files") && method == "GET" && query.contains("q=") {
            return json(json!({ "files": [{ "id": "f1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" }] }));
        }
        if path.contains("/files/") && query.contains("fields=id,name,mimeType") && !query.contains("modifiedTime") {
            return json(json!({ "id": "f1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" }));
        }
        if path.contains("/calendars/") && path.contains("/events") && method == "GET" {
            return json(json!({ "items": [{ "id": "e1", "summary": "Standup", "start": { "dateTime": "2026-11-03T15:00:00Z" } }] }));
        }
        if path.contains("/documents/") {
            return json(json!({ "documentId": "doc1", "title": "Notes", "body": { "content": [{ "paragraph": { "elements": [{ "textRun": { "content": "A paragraph.\n" } }] } }] } }));
        }
        if path.contains("/values/") {
            return json(json!({ "range": "Sheet1!A1:B2", "values": [["a", "b"], ["1", "2"]] }));
        }
        json(json!({ "ok": true }))
    }

    fn saw(rec: &Rec, part: &str) -> bool {
        rec.0.lock().unwrap().iter().any(|(_, path, _, _)| path.contains(part))
    }

    #[tokio::test]
    async fn gmail_lists_reads_and_sends_a_draft_against_the_rest_api() {
        let (base, rec) = mock().await;
        let url = format!("{base}/gmail/v1");
        let http = reqwest::Client::new();
        let listed = call(&http, &url, Some("tok"), "search_threads", &json!({ "query": "from:ada" })).await.unwrap();
        assert!(listed.text.contains("hello from ada"), "{}", listed.text);
        assert!(saw(&rec, "/gmail/v1/users/me/threads"), "{:?}", rec.0.lock().unwrap());
        let read = call(&http, &url, Some("tok"), "get_message", &json!({ "messageId": "m1" })).await.unwrap();
        assert!(read.text.contains("Hello there") && read.text.contains("Hi"), "{}", read.text);
        let draft = call(&http, &url, Some("tok"), "create_draft", &json!({ "to": ["ada@example.com"], "subject": "Hello", "body": "Hi Ada" })).await.unwrap();
        assert!(draft.text.contains("d1"), "{}", draft.text);
        let sent = call(&http, &url, Some("tok"), "send_draft", &json!({ "draftId": "d1" })).await.unwrap();
        assert!(sent.text.contains("m-sent"), "{}", sent.text);
        assert!(saw(&rec, "/drafts/send"));
        let paths = rec.0.lock().unwrap().iter().map(|(_, p, _, _)| p.clone()).collect::<Vec<_>>().join(" ");
        assert!(!paths.contains("mcp"), "{paths}");
    }

    #[tokio::test]
    async fn drive_search_and_read_calendar_list_docs_and_sheets_read() {
        let (base, rec) = mock().await;
        let http = reqwest::Client::new();
        let drive = format!("{base}/drive/v3");
        let found = call(&http, &drive, Some("tok"), "search_files", &json!({ "query": "name contains 'Plan'" })).await.unwrap();
        assert!(found.text.contains("Plan"), "{}", found.text);
        let read = call(&http, &drive, Some("tok"), "read_file_content", &json!({ "fileId": "f1" })).await.unwrap();
        assert!(read.text.contains("exported document text"), "{}", read.text);
        let cal = format!("{base}/calendar/v3");
        let events = call(&http, &cal, Some("tok"), "list_events", &json!({ "startTime": "2026-11-03T00:00:00Z", "endTime": "2026-11-04T00:00:00Z" })).await.unwrap();
        assert!(events.text.contains("Standup"), "{}", events.text);
        let docs = format!("{base}/docs/v1");
        let doc = call(&http, &docs, Some("tok"), "read_doc", &json!({ "documentId": "doc1" })).await.unwrap();
        assert!(doc.text.contains("A paragraph"), "{}", doc.text);
        let sheets = format!("{base}/sheets/v4");
        let cells = call(&http, &sheets, Some("tok"), "get_values", &json!({ "spreadsheetId": "s1", "range": "Sheet1!A1:B2" })).await.unwrap();
        assert!(cells.text.contains("Sheet1"), "{}", cells.text);
        let paths = rec.0.lock().unwrap().iter().map(|(_, p, _, _)| p.clone()).collect::<Vec<_>>().join(" ");
        assert!(!paths.contains("gmailmcp") && !paths.contains("mcp.googleapis"), "{paths}");
    }

    #[tokio::test]
    async fn google_errors_say_what_to_do_instead_of_a_raw_403() {
        let (base, _) = mock().await;
        let url = format!("{base}/gmail/v1");
        let http = reqwest::Client::new();
        let show = |r: Result<CallOutput, CallError>| match r {
            Err(e) => format!("{e:?}"),
            Ok(_) => panic!("expected an error"),
        };
        let reauth = show(call(&http, &url, Some("fail-401"), "search_threads", &json!({})).await);
        let config = show(call(&http, &url, Some("fail-config"), "search_threads", &json!({})).await);
        let scope = show(call(&http, &url, Some("fail-scope"), "search_threads", &json!({})).await);
        let preview = show(call(&http, &url, Some("fail-preview"), "search_threads", &json!({})).await);
        let text = format!("{reauth:?} {config:?} {scope:?} {preview:?}");
        assert!(text.contains("Sign in again"), "{text}");
        assert!(text.contains("not turned on"), "{text}");
        assert!(text.contains("does not include the access"), "{text}");
        assert!(text.contains("preview"), "{text}");
        assert!(!text.contains("49379713533"), "a raw project id is the answer: {text}");
        assert!(handles("https://gmail.googleapis.com/gmail/v1"));
        assert!(!handles("https://gmailmcp.googleapis.com/mcp/v1"));
        assert!(!handles("https://drivemcp.googleapis.com/mcp/v1"));
        assert!(!handles("https://calendarmcp.googleapis.com/mcp/v1"));
        assert!(!handles("https://chatmcp.googleapis.com/mcp/v1"));
        assert!(!handles("https://docsmcp.googleapis.com/mcp/v1"));
    }

    #[test]
    fn every_catalog_google_tool_is_implemented_and_points_at_a_rest_api() {
        for m in crate::catalog::manifests().iter().filter(|m| m.provider.as_deref() == Some("google")) {
            assert!(handles(&m.endpoint), "{} -> {}", m.id, m.endpoint);
            assert!(!m.endpoint.contains("mcp"), "{}", m.endpoint);
            let tools = list_tools(&m.endpoint).unwrap();
            let mut named: Vec<_> = m.tools.keys().cloned().collect();
            let mut live: Vec<_> = tools.iter().map(|t| t.name.clone()).collect();
            named.sort();
            live.sort();
            assert_eq!(named, live, "{}", m.id);
        }
    }
}
