//! Errors whose text the UI translates.
//!
//! Operations report what happened as [`Message`]s, which the workspace renders
//! in the active language when it shows them. Failures built here carry their
//! message inside the `anyhow::Error`; anything else — git's own output,
//! libgit2 and filesystem errors — is shown as it is.

use rgitui_i18n::{Message, MessageError};

/// An error whose text is `message`. Its `Display` is the English text, so
/// logs and error chains stay readable.
pub(crate) fn error(message: Message) -> anyhow::Error {
    anyhow::Error::new(MessageError(message))
}

/// What a failure shows for `error`: its [`Message`] when the text a user
/// would read is one, and the error's own text otherwise.
///
/// Only the outermost text counts — the one `error.to_string()` gives — so a
/// message wrapped by further context is shown as that context, exactly as it
/// would be without translation.
pub(crate) fn error_message(error: &anyhow::Error) -> Message {
    let text = error.to_string();
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<MessageError>())
        .filter(|message| message.to_string() == text)
        .map(|message| message.0.clone())
        .unwrap_or_else(|| Message::verbatim(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use rgitui_i18n::msg;

    #[test]
    fn a_message_error_keeps_its_message() {
        let message = msg!("git.error.head_detached");
        assert_eq!(error_message(&error(message.clone())), message);
    }

    #[test]
    fn outer_context_wins_over_an_inner_message() {
        let wrapped = Err::<(), _>(error(msg!("git.error.head_detached")))
            .context("Could not read the repository")
            .unwrap_err();
        assert_eq!(
            error_message(&wrapped),
            Message::verbatim("Could not read the repository")
        );
    }

    #[test]
    fn other_errors_are_shown_as_they_are() {
        let plain = anyhow::anyhow!("fatal: not a git repository");
        assert_eq!(
            error_message(&plain),
            Message::verbatim("fatal: not a git repository")
        );
    }
}
