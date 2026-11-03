//! The Drive server's own pieces: its tools against its manifest, a search
//! as Drive's query language takes it, an upload, and a share.

use super::*;

#[test]
fn every_tool_is_classed_in_the_manifest() {
    let m: toml::Table = include_str!("../../catalog/google-drive.toml").parse().unwrap();
    let mut classed: Vec<&str> = m["tools"].as_table().unwrap().keys().map(String::as_str).collect();
    let mut listed: Vec<&str> = tools().iter().map(|t| t.name).collect();
    classed.sort();
    listed.sort();
    assert_eq!(classed, listed);
    // Sharing reaches other people, and a delete has no trash to come back from.
    for t in ["share_file", "delete_file"] {
        assert_eq!(m["tools"][t].as_str(), Some("irreversible"), "{t}");
    }
}

#[test]
fn every_tool_names_its_schemas() {
    let ts = tools();
    assert_eq!(ts.len(), 15);
    for t in &ts {
        assert_eq!(t.input["type"], "object", "{}", t.name);
        assert_eq!(t.output["type"], "object", "{}", t.name);
    }
    assert_eq!(file_id(&json!({ "file_id": "../about" })).unwrap(), "..%2Fabout");
    assert!(file_id(&json!({})).is_err());
}

/// Words searched for are quoted, so none of them becomes Drive query
/// syntax; the trash is left out unless asked.
#[test]
fn a_search_is_said_in_drive_s_query_language() {
    assert_eq!(drive_query(&json!({})), "trashed = false");
    assert_eq!(drive_query(&json!({ "text": "Bob's plan", "folder_id": "f1" })), "fullText contains 'Bob\\'s plan' and 'f1' in parents and trashed = false");
    assert_eq!(drive_query(&json!({ "text": "x' or name contains '" })), "fullText contains 'x\\' or name contains \\'' and trashed = false");
    assert_eq!(drive_query(&json!({ "query": "name contains 'a' or name contains 'b'", "include_trashed": true })), "(name contains 'a' or name contains 'b')");
}

/// An upload carries the metadata and the bytes in their parts, with a
/// boundary the bytes do not hold.
#[test]
fn an_upload_carries_its_metadata_and_bytes() {
    let (kind, body) = multipart(&json!({ "name": "a.txt" }), "text/plain", b"hello");
    assert_eq!(kind, "multipart/related; boundary=magnus-part");
    assert_eq!(String::from_utf8(body).unwrap(), "--magnus-part\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"a.txt\"}\r\n--magnus-part\r\nContent-Type: text/plain\r\n\r\nhello\r\n--magnus-part--\r\n");
    let (kind, body) = multipart(&json!({}), "text/plain", b"x\r\n--magnus-part--\r\n");
    assert_eq!(kind, "multipart/related; boundary=magnus-partx");
    assert!(body.ends_with(b"\r\n--magnus-partx--\r\n"));
    assert_eq!(given(&json!({ "base64": "AP8=" })).unwrap(), Some(vec![0, 255]));
    assert!(given(&json!({ "base64": "not base64!" })).is_err());
    assert_eq!(given(&json!({})).unwrap(), None);
}

/// A share is to a person, a group or anyone with the link, never to a new
/// owner.
#[test]
fn a_share_never_gives_a_file_away() {
    assert_eq!(grant(&json!({ "email": "bo@example.com", "role": "writer" })).unwrap(), json!({ "type": "user", "role": "writer", "emailAddress": "bo@example.com" }));
    assert_eq!(grant(&json!({ "anyone_with_link": true, "role": "reader" })).unwrap(), json!({ "type": "anyone", "role": "reader" }));
    assert!(grant(&json!({ "email": "bo@example.com", "role": "owner" })).is_err());
    assert!(grant(&json!({ "role": "reader" })).is_err());
}

#[test]
fn a_file_is_read_as_a_person_reads_it() {
    let f = json!({ "id": "f1", "name": "Plan", "mimeType": "application/vnd.google-apps.document", "size": "2048", "parents": ["root"], "owners": [{ "emailAddress": "al@example.com" }], "shared": true });
    let v = file(&f);
    assert_eq!((v["file_id"].as_str(), v["size"].as_u64(), v["shared"].as_bool(), v["trashed"].as_bool()), (Some("f1"), Some(2048), Some(true), Some(false)));
    assert_eq!(v["owners"], json!(["al@example.com"]));
    assert_eq!(export_as("application/vnd.google-apps.spreadsheet"), Some("text/csv"));
    assert_eq!(export_as("application/vnd.google-apps.document"), Some("text/plain"));
    assert_eq!(export_as("application/pdf"), None);
    assert!(textual("text/markdown") && textual("application/json") && !textual("image/png"));
}
