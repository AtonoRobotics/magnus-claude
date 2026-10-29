//! The fixed components an agent may compose an outcome from (U8). The kernel
//! checks shape only; the app draws each component with its own code inside a
//! sandbox, so a composition can choose what to show but never how it runs.

use serde_json::Value;

pub const KINDS: &[&str] = &["heading", "text", "list", "table", "stat", "bar_chart", "line_chart", "image", "file"];

const MAX_COMPONENTS: usize = 40;
const MAX_TEXT: usize = 4000;
pub const MAX_ROWS: usize = 200;
/// A delivered file shown in full on the page holds at most as much text as
/// a composition may, and a table from it at most as many rows; the rest is
/// in the file.
pub const PREVIEW_CHARS: usize = MAX_COMPONENTS * MAX_TEXT;
const MAX_COLUMNS: usize = 12;
const MAX_POINTS: usize = 60;
const MAX_ITEMS: usize = 100;
pub const MAX_SPOKEN: usize = 6000;

fn text(v: &Value, what: &str) -> Result<(), String> {
    let s = v.as_str().ok_or(format!("{what} must be a string"))?;
    if s.trim().is_empty() {
        return Err(format!("{what} is empty"));
    }
    if s.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters"));
    }
    Ok(())
}

fn strings(v: &Value, what: &str, max: usize) -> Result<usize, String> {
    let a = v.as_array().ok_or(format!("{what} must be a list"))?;
    if a.is_empty() || a.len() > max {
        return Err(format!("{what} needs 1 to {max} entries"));
    }
    for (i, x) in a.iter().enumerate() {
        text(x, &format!("{what}[{i}]"))?;
    }
    Ok(a.len())
}

fn numbers(v: &Value, what: &str, len: usize) -> Result<(), String> {
    let a = v.as_array().ok_or(format!("{what} must be a list of numbers"))?;
    if a.len() != len || a.iter().any(|x| !x.is_number()) {
        return Err(format!("{what} must hold {len} numbers, one per label"));
    }
    Ok(())
}

/// Check a composition against the fixed components. `files` are the names of
/// files the run delivered; an image must show one of them.
pub fn validate(view: &Value, files: &[String]) -> Result<(), String> {
    let parts = view.as_array().ok_or("view must be a list of components")?;
    if parts.is_empty() || parts.len() > MAX_COMPONENTS {
        return Err(format!("view needs 1 to {MAX_COMPONENTS} components"));
    }
    for (n, c) in parts.iter().enumerate() {
        let at = format!("component {n}");
        let kind = c["type"].as_str().ok_or(format!("{at} has no type"))?;
        match kind {
            "heading" | "text" => text(&c["text"], &format!("{at}.text"))?,
            "list" => {
                strings(&c["items"], &format!("{at}.items"), MAX_ITEMS)?;
            }
            "table" => {
                let cols = strings(&c["columns"], &format!("{at}.columns"), MAX_COLUMNS)?;
                let rows = c["rows"].as_array().ok_or(format!("{at}.rows must be a list of rows"))?;
                if rows.len() > MAX_ROWS {
                    return Err(format!("{at} has more than {MAX_ROWS} rows; deliver a file instead"));
                }
                for (i, r) in rows.iter().enumerate() {
                    let cells = r.as_array().ok_or(format!("{at}.rows[{i}] must be a list"))?;
                    if cells.len() != cols || cells.iter().any(|x| !(x.is_string() || x.is_number())) {
                        return Err(format!("{at}.rows[{i}] must hold {cols} strings or numbers"));
                    }
                }
            }
            "stat" => {
                text(&c["label"], &format!("{at}.label"))?;
                if !(c["value"].is_string() || c["value"].is_number()) {
                    return Err(format!("{at}.value must be a string or a number"));
                }
            }
            "bar_chart" => {
                let n = strings(&c["labels"], &format!("{at}.labels"), MAX_POINTS)?;
                numbers(&c["values"], &format!("{at}.values"), n)?;
            }
            "line_chart" => {
                let n = strings(&c["labels"], &format!("{at}.labels"), MAX_POINTS)?;
                let series = c["series"].as_array().filter(|s| !s.is_empty() && s.len() <= 6).ok_or(format!("{at}.series needs 1 to 6 series"))?;
                for (i, s) in series.iter().enumerate() {
                    text(&s["name"], &format!("{at}.series[{i}].name"))?;
                    numbers(&s["values"], &format!("{at}.series[{i}].values"), n)?;
                }
            }
            // An image is drawn; a file is shown in full, drawn from the
            // same components by the app.
            "image" | "file" => {
                let f = c["file"].as_str().ok_or(format!("{at}.file must name a delivered file"))?;
                if !files.iter().any(|x| x == f) {
                    return Err(format!("{at}.file {f:?} is not a file you delivered; deliver it with deliver_file first"));
                }
            }
            other => return Err(format!("{at} has unknown type {other:?}; use one of {}", KINDS.join(", "))),
        }
        for t in ["title", "unit", "caption"] {
            if !c[t].is_null() {
                text(&c[t], &format!("{at}.{t}"))?;
            }
        }
    }
    Ok(())
}
