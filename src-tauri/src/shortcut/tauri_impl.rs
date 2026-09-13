//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{error, warn};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(not(target_os = "linux"))]
use crate::settings::get_settings;
use crate::settings::{self, ShortcutBinding};

use super::handler::handle_shortcut_event;

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) {
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if id == "cancel" {
            continue; // Skip cancel shortcut, it will be registered dynamically
        }
        // Skip the post-processing (AI Correction) shortcut when the feature is
        // turned off. Gated only by its own toggle now — not by Experimental.
        if id == "transcribe_with_post_process" && !user_settings.post_process_enabled {
            continue;
        }
        // Same for the assistant's own shortcuts when the master switch is off:
        // `set_assistant_enabled` unregisters them at runtime, and without this
        // the next launch would silently grab those combos again — swallowing
        // them from other apps for a feature that does nothing.
        if crate::assistant::is_assistant_binding(&id) && !user_settings.assistant_enabled {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        // An empty binding means "disabled" — nothing to register.
        if binding.current_binding.trim().is_empty() {
            continue;
        }

        if let Err(e) = register_shortcut(app, binding) {
            error!("Failed to register shortcut {} during init: {}", id, e);
        }
    }
}

/// Modifier names accepted in a binding string, in the generic form Tauri
/// understands.
const MODIFIERS: &[&str] = &[
    "ctrl", "control", "shift", "alt", "option", "meta", "command", "cmd", "super", "win",
    "windows",
];

/// Flatten a binding string into the form Tauri's global-shortcut plugin can
/// parse: side-specific modifiers lose their handedness.
///
/// `handy-keys` distinguishes `ctrl_left` from `ctrl_right`, which is why the
/// Windows defaults are written that way (AltGr on international layouts reports
/// as Left Ctrl + Right Alt, and must not trigger the assistant). Tauri's plugin
/// has no way to express handedness at all — worse, `global-hotkey` parses
/// `ctrl_left` as a *key name* and fails with `UnsupportedKey`, so a combo that
/// looked valid was rejected at registration with only an `error!` line.
///
/// The suffix is only stripped when what remains is actually a modifier, so a
/// hypothetical `arrow_left` is left alone.
pub fn normalize_for_tauri(raw: &str) -> String {
    raw.split('+')
        .map(|part| {
            let part = part.trim().to_lowercase();
            let base = part
                .strip_suffix("_left")
                .or_else(|| part.strip_suffix("_right"));
            match base {
                Some(base) if MODIFIERS.contains(&base) => base.to_string(),
                _ => part,
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Derive a combo this engine can actually register from one it cannot.
///
/// Tauri's plugin requires a non-modifier key, so a modifier-only combo has no
/// valid form — `Space` is appended, keeping whichever modifiers the user chose.
/// This is not cosmetic: on Windows the shipped defaults for **both** dictation
/// (`ctrl_left+super`) and the assistant (`ctrl_left+alt_left`) are
/// modifier-only. A Windows install that fell back to this engine — which
/// happens on any handy-keys init failure, and is then persisted, so it never
/// retries — therefore had no working dictation hotkey and no working assistant
/// hotkey. Resetting to the default could not help, because the default is the
/// offending value.
pub fn tauri_safe_binding(raw: &str) -> String {
    let normalized = normalize_for_tauri(raw);
    if normalized.trim().is_empty() {
        return normalized;
    }
    let has_main_key = normalized
        .split('+')
        .any(|part| !MODIFIERS.contains(&part.trim()));
    if has_main_key {
        normalized
    } else {
        format!("{normalized}+space")
    }
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
/// Tauri requires at least one non-modifier key and doesn't support the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }

    // Compare against the flattened form, so a side-specific modifier is
    // recognised as the modifier it is. Without this, `ctrl_left+alt_left` was
    // read as "one modifier plus a main key called ctrl_left" and passed
    // validation, only to fail unparseable a moment later.
    let parts: Vec<String> = normalize_for_tauri(raw)
        .split('+')
        .map(|p| p.trim().to_string())
        .collect();

    // Check for fn key which Tauri doesn't support
    for part in &parts {
        if part == "fn" || part == "function" {
            return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
        }
    }

    // Check for at least one non-modifier key
    let has_non_modifier = parts.iter().any(|part| !MODIFIERS.contains(&part.as_str()));

    if has_non_modifier {
        Ok(())
    } else {
        Err("Tauri shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into())
    }
}

/// Register a shortcut using Tauri's global-shortcut plugin.
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // An empty binding means "disabled" — nothing to register.
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    register_one(app, &binding.id, &binding.current_binding)
}

/// Register a single (id, hotkey) pair with Tauri's global-shortcut plugin.
fn register_one(app: &AppHandle, binding_id: &str, hotkey: &str) -> Result<(), String> {
    // Validate for Tauri requirements
    if let Err(e) = validate_shortcut(hotkey) {
        warn!(
            "register_tauri_shortcut validation error for '{}': {}",
            hotkey, e
        );
        return Err(e);
    }

    // Parse the flattened combo: Tauri cannot express handedness, so
    // `ctrl_left+alt_left+space` has to reach the parser as `ctrl+alt+space`.
    // Parsing the raw string made `ctrl_left` an unknown key and failed.
    let normalized = normalize_for_tauri(hotkey);
    let shortcut = match normalized.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!("Failed to parse shortcut '{}': {}", hotkey, e);
            error!("register_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", hotkey);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    let binding_id_for_closure = binding_id.to_string();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if scut == &shortcut {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!("Couldn't register shortcut '{}': {}", hotkey, e);
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin.
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    unregister_one(app, &binding.current_binding)
}

/// Unregister a single hotkey from Tauri's global-shortcut plugin.
fn unregister_one(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut = match hotkey.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}' for unregistration: {}",
                hotkey, e
            );
            error!("unregister_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!("Failed to unregister shortcut '{}': {}", hotkey, e);
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;

    Ok(())
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                // Empty binding = cancel shortcut disabled.
                if cancel_binding.current_binding.trim().is_empty() {
                    return;
                }
                if let Err(e) = register_shortcut(&app_clone, cancel_binding) {
                    error!("Failed to register cancel shortcut: {}", e);
                }
            }
        });
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                // We ignore errors here as it might already be unregistered
                let _ = unregister_shortcut(&app_clone, cancel_binding);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped Windows defaults for dictation and the assistant are
    /// modifier-only and side-specific. Both properties are invisible to
    /// `global-hotkey`, so this engine could register neither — and the failure
    /// was an `error!` line, leaving a Windows user who fell back to this engine
    /// with no dictation hotkey and no assistant hotkey at all.
    #[test]
    fn the_windows_modifier_only_defaults_are_rejected_with_an_accurate_reason() {
        for combo in ["ctrl_left+super", "ctrl_left+alt_left"] {
            let err = validate_shortcut(combo)
                .expect_err("a modifier-only combo cannot be registered by Tauri");
            assert!(
                err.contains("main key"),
                "expected a 'needs a main key' error for {combo}, got: {err}"
            );
        }
    }

    /// Validation used to pass these, because `ctrl_left` was not in the
    /// modifier list and so counted as the main key. Registration then failed on
    /// parse, one layer further down and far less visibly.
    #[test]
    fn handedness_is_recognised_as_a_modifier_not_as_a_main_key() {
        assert_eq!(normalize_for_tauri("ctrl_left+alt_left"), "ctrl+alt");
        assert_eq!(normalize_for_tauri("CTRL_Left+Shift"), "ctrl+shift");
        assert_eq!(
            normalize_for_tauri("ctrl_left+alt_left+space"),
            "ctrl+alt+space"
        );
    }

    /// Only a modifier loses its handedness. Anything else keeps its name, so a
    /// key that merely ends in `_left` is not mangled into something else.
    #[test]
    fn a_non_modifier_ending_in_left_is_left_alone() {
        assert_eq!(normalize_for_tauri("ctrl+arrow_left"), "ctrl+arrow_left");
    }

    /// A combo with a real key already in it must survive untouched apart from
    /// the handedness flattening — appending Space to it would silently change a
    /// working shortcut.
    #[test]
    fn a_combo_that_already_has_a_main_key_is_not_given_another() {
        assert_eq!(tauri_safe_binding("ctrl+shift+a"), "ctrl+shift+a");
        assert_eq!(tauri_safe_binding("ctrl+shift+space"), "ctrl+shift+space");
        assert_eq!(
            tauri_safe_binding("ctrl_left+alt_left+space"),
            "ctrl+alt+space"
        );
    }

    /// The point of the fallback: a modifier-only default becomes something this
    /// engine can actually register, keeping the modifiers the user chose. The
    /// results must themselves validate, or the fallback just moves the failure.
    #[test]
    fn a_modifier_only_combo_gains_a_main_key_and_then_validates() {
        assert_eq!(tauri_safe_binding("ctrl_left+alt_left"), "ctrl+alt+space");
        assert_eq!(tauri_safe_binding("ctrl_left+super"), "ctrl+super+space");
        for combo in ["ctrl_left+super", "ctrl_left+alt_left"] {
            let safe = tauri_safe_binding(combo);
            assert!(
                validate_shortcut(&safe).is_ok(),
                "the fallback for {combo} ({safe}) must itself be registerable"
            );
        }
    }

    /// An empty binding means "disabled" and must stay empty — handing it a
    /// Space would arm a shortcut the user deliberately switched off. This is the
    /// shipped default for the cancel binding.
    #[test]
    fn an_empty_binding_stays_empty() {
        assert_eq!(tauri_safe_binding(""), "");
        assert!(validate_shortcut("").is_err());
    }
}
