//! Writes a panic into the log file before the process goes down.
//!
//! On Windows the app has no console, so the default hook's message goes to a
//! stderr nobody can read. A panic on the main thread (a tray click, a window
//! event, a synchronous command) is caught by the event loop and re-raised out
//! of `run()`, which ends the process with exit code 101. That leaves no Windows
//! Error Reporting entry and no line in speakoflow.log: the app just vanishes
//! (issue #93). This hook logs the panic first, then runs the default hook.

/// Longest panic message kept. A message can quote the string it failed on
/// (a slice that is not on a char boundary quotes up to 256 bytes of it), and
/// dictated text is otherwise never written to the log.
const MAX_MESSAGE_CHARS: usize = 200;

pub fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");

        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let message = truncate_chars(&message, MAX_MESSAGE_CHARS);

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "an unknown location".to_string());

        // No backtrace: release builds are stripped, so it would only be a list
        // of unnamed frames. The location already names the failing line.
        log::error!("Panic on thread '{thread_name}' at {location}: {message}");
        log::logger().flush();

        default_hook(info);
    }));
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_chars;

    #[test]
    fn short_messages_are_kept_whole() {
        assert_eq!(
            truncate_chars("index out of bounds", 200),
            "index out of bounds"
        );
    }

    #[test]
    fn long_messages_are_cut_on_a_char_boundary() {
        let text = "é".repeat(300);
        let cut = truncate_chars(&text, 200);
        assert_eq!(cut.chars().count(), 201);
        assert!(cut.ends_with('…'));
    }
}
