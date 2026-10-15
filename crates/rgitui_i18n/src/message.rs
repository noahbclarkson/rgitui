//! Messages translated where they are shown rather than where they are made.

use std::fmt;

use gpui::SharedString;

use crate::catalog::{interpolate, PluralRule};
use crate::keys::find_english;

/// A user-facing message: a checked key with its values, or text that is
/// shown as it is.
///
/// Built with [`crate::msg!`] or [`crate::msgp!`] wherever the text arises —
/// a background thread included — and rendered with [`Message::translated`]
/// wherever it is displayed, so it comes out in the language active at that
/// moment. `Display` is always English, which keeps logs, error chains and bug
/// reports readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message(Inner);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Inner {
    Keyed {
        key: &'static str,
        count: Option<i64>,
        args: Vec<(&'static str, MessageArg)>,
    },
    Verbatim(SharedString),
}

/// One value substituted into a [`Message`]: literal text, or another message
/// translated along with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageArg {
    Text(String),
    Message(Message),
}

impl MessageArg {
    fn render(&self, english: bool) -> String {
        match self {
            MessageArg::Text(text) => text.clone(),
            MessageArg::Message(message) if english => message.english(),
            MessageArg::Message(message) => message.translated().to_string(),
        }
    }
}

/// Values usable as a [`Message`] argument.
pub trait IntoMessageArg {
    fn into_message_arg(self) -> MessageArg;
}

impl IntoMessageArg for Message {
    fn into_message_arg(self) -> MessageArg {
        MessageArg::Message(self)
    }
}

impl<T: IntoMessageArg + Clone> IntoMessageArg for &T {
    fn into_message_arg(self) -> MessageArg {
        self.clone().into_message_arg()
    }
}

impl IntoMessageArg for std::path::Display<'_> {
    fn into_message_arg(self) -> MessageArg {
        MessageArg::Text(self.to_string())
    }
}

macro_rules! impl_text_arg {
    ($($ty:ty),+) => {
        $(impl IntoMessageArg for $ty {
            fn into_message_arg(self) -> MessageArg {
                MessageArg::Text(self.to_string())
            }
        })+
    };
}

impl_text_arg!(
    String,
    &str,
    SharedString,
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize
);

impl Message {
    #[doc(hidden)]
    pub fn keyed(key: &'static str, args: Vec<(&'static str, MessageArg)>) -> Self {
        Self(Inner::Keyed {
            key,
            count: None,
            args,
        })
    }

    #[doc(hidden)]
    pub fn plural(stem: &'static str, count: i64, args: Vec<(&'static str, MessageArg)>) -> Self {
        Self(Inner::Keyed {
            key: stem,
            count: Some(count),
            args,
        })
    }

    /// Text shown as it is in every language, such as git's own output.
    pub fn verbatim(text: impl Into<SharedString>) -> Self {
        Self(Inner::Verbatim(text.into()))
    }

    /// The message in the active language.
    pub fn translated(&self) -> SharedString {
        match &self.0 {
            Inner::Verbatim(text) => text.clone(),
            Inner::Keyed { key, count, args } => {
                let args: Vec<(&str, String)> = args
                    .iter()
                    .map(|(name, value)| (*name, value.render(false)))
                    .collect();
                match count {
                    Some(count) => crate::translate_plural(key, *count, &args),
                    None => crate::translate_args(key, &args),
                }
            }
        }
    }

    /// The message in English, whatever the active language.
    pub fn english(&self) -> String {
        match &self.0 {
            Inner::Verbatim(text) => text.to_string(),
            Inner::Keyed { key, count, args } => {
                let mut args: Vec<(&str, String)> = args
                    .iter()
                    .map(|(name, value)| (*name, value.render(true)))
                    .collect();
                let template = match count {
                    Some(count) => {
                        args.push(("count", count.to_string()));
                        let suffix = PluralRule::OneOther.select(*count).suffix();
                        find_english(&format!("{key}.{suffix}"))
                    }
                    None => find_english(key),
                };
                match template {
                    Some(template) => interpolate(template, &args),
                    None => key.to_string(),
                }
            }
        }
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.english())
    }
}

/// An error whose text is a [`Message`], so whoever shows it can translate it.
/// Its `Display` is the English text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageError(pub Message);

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for MessageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_messages_render_english_with_their_values() {
        let message = crate::msg!("settings.language.system_detail", language = "Deutsch");
        assert_eq!(message.english(), "Detected: Deutsch");
        assert_eq!(message.to_string(), "Detected: Deutsch");
        assert_eq!(message.translated().as_ref(), "Detected: Deutsch");
    }

    #[test]
    fn plural_messages_pick_the_english_form_and_fill_the_count() {
        assert_eq!(
            crate::msgp!("settings.language.missing", 1).english(),
            "1 string not translated"
        );
        assert_eq!(
            crate::msgp!("settings.language.missing", 3).english(),
            "3 strings not translated"
        );
    }

    #[test]
    fn nested_messages_are_rendered_in_the_same_language() {
        let inner = crate::msg!("settings.nav.general");
        let outer = crate::msg!("settings.language.system_detail", language = inner);
        assert_eq!(outer.english(), "Detected: General");
    }

    #[test]
    fn values_are_inserted_once_and_never_rescanned() {
        let message = crate::msg!("settings.language.system_detail", language = "{language}");
        assert_eq!(message.english(), "Detected: {language}");
    }

    #[test]
    fn verbatim_text_is_never_translated() {
        let message = Message::verbatim("fatal: not a git repository");
        assert_eq!(message.english(), "fatal: not a git repository");
        assert_eq!(message.translated().as_ref(), "fatal: not a git repository");
    }

    #[test]
    fn a_message_error_displays_its_english_text() {
        let error = MessageError(crate::msg!("settings.nav.general"));
        assert_eq!(error.to_string(), "General");
    }
}
