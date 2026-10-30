//! The window's material (U7, amended): glass uses the system's own window
//! material where it has one (Mica on Windows 11, Acrylic on Windows 10),
//! and the page draws its backdrop where it has none (Linux). Reading
//! surfaces are the page's and stay solid either way.

use serde::Serialize;

#[derive(Serialize)]
pub struct Material {
    /// Whether the system draws the glass behind the page; when not, the
    /// page draws its own backdrop.
    native: bool,
}

/// Put the window on glass or take it off. `dark` picks the material's
/// dark variant where the system has one.
#[tauri::command]
pub fn set_material(window: tauri::WebviewWindow, glass: bool, dark: bool) -> Material {
    Material { native: apply(&window, glass, dark) }
}

#[cfg(target_os = "windows")]
fn apply(window: &tauri::WebviewWindow, glass: bool, dark: bool) -> bool {
    use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};
    let _ = clear_mica(window);
    let _ = clear_acrylic(window);
    if !glass {
        return false;
    }
    // Mica on Windows 11; Acrylic, tinted for the mode, on Windows 10.
    apply_mica(window, Some(dark)).is_ok() || apply_acrylic(window, Some(if dark { (24, 24, 28, 160) } else { (248, 248, 246, 160) })).is_ok()
}

#[cfg(not(target_os = "windows"))]
fn apply(_window: &tauri::WebviewWindow, _glass: bool, _dark: bool) -> bool {
    false
}

#[derive(Serialize)]
pub struct System {
    /// The person turned transparency effects off in the system's settings.
    reduce_transparency: bool,
}

/// The system's settings the page cannot read itself. Reduced motion
/// reaches the page as a media query on every system; reduced transparency
/// does not reach WebView2, so it is read here (Windows: Personalize,
/// EnableTransparency = 0).
#[tauri::command]
pub fn system_look() -> System {
    System { reduce_transparency: reduce_transparency() }
}

#[cfg(target_os = "windows")]
fn reduce_transparency() -> bool {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "EnableTransparency"])
        .creation_flags(NO_WINDOW)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().last() == Some("0x0"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn reduce_transparency() -> bool {
    false
}
