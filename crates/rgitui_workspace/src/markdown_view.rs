use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    div, px, rems, FontStyle, FontWeight, HighlightStyle, SharedString, StyledText, Window,
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rgitui_theme::{ActiveTheme, StyledExt};
use rgitui_ui::{Label, LabelSize};

#[derive(Debug)]
enum MarkdownBlock {
    Heading {
        level: u8,
        spans: Vec<InlineSpan>,
    },
    Paragraph {
        spans: Vec<InlineSpan>,
    },
    CodeBlock {
        code: String,
    },
    BlockQuote {
        blocks: Vec<MarkdownBlock>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<MarkdownBlock>>,
    },
    HorizontalRule,
}

#[derive(Debug, Clone)]
enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    BoldItalic(String),
    Code(String),
    Link { text: String },
}

struct ParseState {
    blocks: Vec<MarkdownBlock>,
    inline_spans: Vec<InlineSpan>,
    bold: bool,
    italic: bool,
    in_heading: Option<u8>,
    in_code_block: bool,
    code_block_content: String,
    blockquote_depth: u32,
    blockquote_blocks: Vec<Vec<MarkdownBlock>>,
    list_stack: Vec<ListState>,
    in_link: bool,
}

struct ListState {
    ordered: bool,
    start: u64,
    items: Vec<Vec<MarkdownBlock>>,
    current_item_blocks: Vec<MarkdownBlock>,
    current_item_spans: Vec<InlineSpan>,
}

fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, options);

    let mut state = ParseState {
        blocks: Vec::new(),
        inline_spans: Vec::new(),
        bold: false,
        italic: false,
        in_heading: None,
        in_code_block: false,
        code_block_content: String::new(),
        blockquote_depth: 0,
        blockquote_blocks: Vec::new(),
        list_stack: Vec::new(),
        in_link: false,
    };

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                state.in_heading = Some(level as u8);
                state.inline_spans.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = state.in_heading.take() {
                    let spans = std::mem::take(&mut state.inline_spans);
                    push_block(&mut state, MarkdownBlock::Heading { level, spans });
                }
            }
            Event::Start(Tag::Paragraph) => {
                state.inline_spans.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                let spans = std::mem::take(&mut state.inline_spans);
                if !spans.is_empty() {
                    push_block(&mut state, MarkdownBlock::Paragraph { spans });
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                state.in_code_block = true;
                state.code_block_content.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                state.in_code_block = false;
                let code = std::mem::take(&mut state.code_block_content);
                let code = code.trim_end().to_string();
                push_block(&mut state, MarkdownBlock::CodeBlock { code });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                state.blockquote_depth += 1;
                state.blockquote_blocks.push(Vec::new());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                state.blockquote_depth = state.blockquote_depth.saturating_sub(1);
                if let Some(blocks) = state.blockquote_blocks.pop() {
                    push_block(&mut state, MarkdownBlock::BlockQuote { blocks });
                }
            }
            Event::Start(Tag::List(first_item)) => {
                let ordered = first_item.is_some();
                let start = first_item.unwrap_or(0);
                state.list_stack.push(ListState {
                    ordered,
                    start,
                    items: Vec::new(),
                    current_item_blocks: Vec::new(),
                    current_item_spans: Vec::new(),
                });
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(list_state) = state.list_stack.pop() {
                    push_block(
                        &mut state,
                        MarkdownBlock::List {
                            ordered: list_state.ordered,
                            start: list_state.start,
                            items: list_state.items,
                        },
                    );
                }
            }
            Event::Start(Tag::Item) => {
                if let Some(list) = state.list_stack.last_mut() {
                    list.current_item_blocks.clear();
                    list.current_item_spans.clear();
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(list) = state.list_stack.last_mut() {
                    let spans = std::mem::take(&mut list.current_item_spans);
                    if !spans.is_empty() {
                        list.current_item_blocks
                            .push(MarkdownBlock::Paragraph { spans });
                    }
                    let blocks = std::mem::take(&mut list.current_item_blocks);
                    list.items.push(blocks);
                }
            }
            Event::Start(Tag::Strong) => {
                state.bold = true;
            }
            Event::End(TagEnd::Strong) => {
                state.bold = false;
            }
            Event::Start(Tag::Emphasis) => {
                state.italic = true;
            }
            Event::End(TagEnd::Emphasis) => {
                state.italic = false;
            }
            Event::Start(Tag::Link { .. }) => {
                state.in_link = true;
            }
            Event::End(TagEnd::Link) => {
                state.in_link = false;
            }
            Event::Text(text) => {
                if state.in_code_block {
                    state.code_block_content.push_str(&text);
                } else if !state.list_stack.is_empty() {
                    let span = make_span(&text, state.bold, state.italic, state.in_link);
                    if let Some(list) = state.list_stack.last_mut() {
                        list.current_item_spans.push(span);
                    }
                } else {
                    let span = make_span(&text, state.bold, state.italic, state.in_link);
                    state.inline_spans.push(span);
                }
            }
            Event::Code(text) => {
                let span = InlineSpan::Code(text.to_string());
                if !state.list_stack.is_empty() {
                    if let Some(list) = state.list_stack.last_mut() {
                        list.current_item_spans.push(span);
                    }
                } else {
                    state.inline_spans.push(span);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                let span = InlineSpan::Text(" ".to_string());
                if !state.list_stack.is_empty() {
                    if let Some(list) = state.list_stack.last_mut() {
                        list.current_item_spans.push(span);
                    }
                } else {
                    state.inline_spans.push(span);
                }
            }
            Event::Rule => {
                push_block(&mut state, MarkdownBlock::HorizontalRule);
            }
            _ => {}
        }
    }

    if !state.inline_spans.is_empty() {
        let spans = std::mem::take(&mut state.inline_spans);
        state.blocks.push(MarkdownBlock::Paragraph { spans });
    }

    state.blocks
}

fn make_span(text: &str, bold: bool, italic: bool, is_link: bool) -> InlineSpan {
    if is_link {
        InlineSpan::Link {
            text: text.to_string(),
        }
    } else if bold && italic {
        InlineSpan::BoldItalic(text.to_string())
    } else if bold {
        InlineSpan::Bold(text.to_string())
    } else if italic {
        InlineSpan::Italic(text.to_string())
    } else {
        InlineSpan::Text(text.to_string())
    }
}

fn push_block(state: &mut ParseState, block: MarkdownBlock) {
    if state.blockquote_depth > 0 {
        if let Some(bq_blocks) = state.blockquote_blocks.last_mut() {
            bq_blocks.push(block);
            return;
        }
    }
    if let Some(list) = state.list_stack.last_mut() {
        list.current_item_blocks.push(block);
        return;
    }
    state.blocks.push(block);
}

fn render_inline_spans(spans: &[InlineSpan], window: &Window, cx: &gpui::App) -> impl IntoElement {
    let colors = cx.colors();
    let mut full_text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

    for span in spans {
        let start = full_text.len();
        match span {
            InlineSpan::Text(t) => {
                full_text.push_str(t);
            }
            InlineSpan::Bold(t) => {
                full_text.push_str(t);
                highlights.push((
                    start..full_text.len(),
                    HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    },
                ));
            }
            InlineSpan::Italic(t) => {
                full_text.push_str(t);
                highlights.push((
                    start..full_text.len(),
                    HighlightStyle {
                        font_style: Some(FontStyle::Italic),
                        ..Default::default()
                    },
                ));
            }
            InlineSpan::BoldItalic(t) => {
                full_text.push_str(t);
                highlights.push((
                    start..full_text.len(),
                    HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        font_style: Some(FontStyle::Italic),
                        ..Default::default()
                    },
                ));
            }
            InlineSpan::Code(t) => {
                full_text.push_str(t);
                highlights.push((
                    start..full_text.len(),
                    HighlightStyle {
                        background_color: Some(gpui::Hsla {
                            a: 0.15,
                            ..colors.text
                        }),
                        ..Default::default()
                    },
                ));
            }
            InlineSpan::Link { text } => {
                full_text.push_str(text);
                highlights.push((
                    start..full_text.len(),
                    HighlightStyle {
                        color: Some(colors.text_accent),
                        ..Default::default()
                    },
                ));
            }
        }
    }

    if full_text.is_empty() {
        return div().into_any_element();
    }

    let mut text_style = window.text_style();
    text_style.color = colors.text_muted;

    if highlights.is_empty() {
        div()
            .child(StyledText::new(SharedString::from(full_text)))
            .into_any_element()
    } else {
        div()
            .child(
                StyledText::new(SharedString::from(full_text))
                    .with_default_highlights(&text_style, highlights),
            )
            .into_any_element()
    }
}

fn render_block(block: &MarkdownBlock, window: &Window, cx: &gpui::App) -> gpui::AnyElement {
    let colors = cx.colors();

    match block {
        MarkdownBlock::Heading { level, spans } => {
            let text = spans_to_plain_text(spans);
            let label = match level {
                1 => Label::new(text)
                    .size(LabelSize::Small)
                    .weight(FontWeight::BOLD),
                2 => Label::new(text)
                    .size(LabelSize::XSmall)
                    .weight(FontWeight::BOLD),
                _ => Label::new(text)
                    .size(LabelSize::XSmall)
                    .weight(FontWeight::SEMIBOLD),
            };
            div().pb(px(1.)).child(label).into_any_element()
        }
        MarkdownBlock::Paragraph { spans } => div()
            .text_xs()
            .child(render_inline_spans(spans, window, cx))
            .into_any_element(),
        MarkdownBlock::CodeBlock { code } => {
            let code_text: SharedString = code.clone().into();
            div()
                .w_full()
                .bg(colors.editor_background)
                .rounded(px(3.))
                .p(px(6.))
                .font_family("monospace")
                .text_size(rems(0.65))
                .text_color(colors.text)
                .child(code_text)
                .into_any_element()
        }
        MarkdownBlock::BlockQuote { blocks } => {
            let mut container = div()
                .v_flex()
                .gap(px(4.))
                .border_l_2()
                .border_color(colors.border_variant)
                .pl(px(12.))
                .text_color(colors.text_muted);
            for b in blocks {
                container = container.child(render_block(b, window, cx));
            }
            container.into_any_element()
        }
        MarkdownBlock::List {
            ordered,
            start,
            items,
        } => {
            let mut list_container = div().v_flex().gap(px(2.)).pl(px(12.));
            for (i, item_blocks) in items.iter().enumerate() {
                let marker: SharedString = if *ordered {
                    format!("{}.", start + i as u64).into()
                } else {
                    "\u{2022}".into()
                };
                let mut item_content = div().v_flex().gap(px(2.)).flex_1().min_w_0();
                for b in item_blocks {
                    item_content = item_content.child(render_block(b, window, cx));
                }
                list_container = list_container.child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap(px(4.))
                        .text_xs()
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(px(16.))
                                .text_color(colors.text_muted)
                                .child(marker),
                        )
                        .child(item_content),
                );
            }
            list_container.into_any_element()
        }
        MarkdownBlock::HorizontalRule => div()
            .w_full()
            .h(px(1.))
            .my(px(4.))
            .bg(colors.border_variant)
            .into_any_element(),
    }
}

fn spans_to_plain_text(spans: &[InlineSpan]) -> SharedString {
    let mut text = String::new();
    for span in spans {
        match span {
            InlineSpan::Text(t)
            | InlineSpan::Bold(t)
            | InlineSpan::Italic(t)
            | InlineSpan::BoldItalic(t)
            | InlineSpan::Code(t) => text.push_str(t),
            InlineSpan::Link { text: t } => text.push_str(t),
        }
    }
    text.into()
}

pub fn render_markdown(text: &str, window: &Window, cx: &gpui::App) -> gpui::AnyElement {
    if text.is_empty() {
        return div().into_any_element();
    }
    let blocks = parse_markdown(text);
    if blocks.is_empty() {
        return div().into_any_element();
    }
    let mut container = div().v_flex().gap(px(6.));
    for block in &blocks {
        container = container.child(render_block(block, window, cx));
    }
    container.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_markdown tests ---

    #[test]
    fn parse_markdown_empty_string() {
        let blocks = parse_markdown("");
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_markdown_plain_text() {
        let blocks = parse_markdown("Hello world");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph, got {:?}", blocks[0]);
        };
        assert_eq!(spans.len(), 1);
        assert!(matches!(&spans[0], InlineSpan::Text(t) if t == "Hello world"));
    }

    #[test]
    fn parse_markdown_heading_level_1() {
        let blocks = parse_markdown("# Hello");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::Heading { level, spans } = &blocks[0] else {
            panic!("expected Heading");
        };
        assert_eq!(*level, 1);
        assert!(matches!(&spans[0], InlineSpan::Text(t) if t == "Hello"));
    }

    #[test]
    fn parse_markdown_heading_level_2() {
        let blocks = parse_markdown("## Hello");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::Heading { level, .. } = &blocks[0] else {
            panic!("expected Heading");
        };
        assert_eq!(*level, 2);
    }

    #[test]
    fn parse_markdown_heading_level_6() {
        let blocks = parse_markdown("###### H6");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::Heading { level, .. } = &blocks[0] else {
            panic!("expected Heading");
        };
        assert_eq!(*level, 6);
    }

    #[test]
    fn parse_markdown_code_block() {
        let blocks = parse_markdown("```\nlet x = 1;\n```");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::CodeBlock { code } = &blocks[0] else {
            panic!("expected CodeBlock");
        };
        assert_eq!(code, "let x = 1;");
    }

    #[test]
    fn parse_markdown_code_block_trims_trailing_whitespace() {
        let blocks = parse_markdown("```\nlet x = 1;   \n```");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::CodeBlock { code } = &blocks[0] else {
            panic!("expected CodeBlock");
        };
        assert_eq!(code, "let x = 1;");
    }

    #[test]
    fn parse_markdown_blockquote() {
        let blocks = parse_markdown("> Hello");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::BlockQuote { blocks: inner } = &blocks[0] else {
            panic!("expected BlockQuote");
        };
        assert_eq!(inner.len(), 1);
        assert!(matches!(&inner[0], MarkdownBlock::Paragraph { .. }));
    }

    #[test]
    fn parse_markdown_unordered_list() {
        let blocks = parse_markdown("- item1\n- item2");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::List { ordered, items, .. } = &blocks[0] else {
            panic!("expected List");
        };
        assert!(!*ordered);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn parse_markdown_ordered_list() {
        let blocks = parse_markdown("1. first\n2. second");
        assert_eq!(blocks.len(), 1);
        let MarkdownBlock::List { ordered, .. } = &blocks[0] else {
            panic!("expected List");
        };
        assert!(*ordered);
    }

    #[test]
    fn parse_markdown_horizontal_rule() {
        let blocks = parse_markdown("---\ntext");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], MarkdownBlock::HorizontalRule));
    }

    #[test]
    fn parse_markdown_bold() {
        let blocks = parse_markdown("**bold**");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        assert!(matches!(&spans[0], InlineSpan::Bold(t) if t == "bold"));
    }

    #[test]
    fn parse_markdown_italic() {
        let blocks = parse_markdown("*italic*");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        assert!(matches!(&spans[0], InlineSpan::Italic(t) if t == "italic"));
    }

    #[test]
    fn parse_markdown_bold_italic() {
        let blocks = parse_markdown("***bold italic***");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        assert!(matches!(&spans[0], InlineSpan::BoldItalic(t) if t == "bold italic"));
    }

    #[test]
    fn parse_markdown_inline_code() {
        let blocks = parse_markdown("`code`");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        assert!(matches!(&spans[0], InlineSpan::Code(t) if t == "code"));
    }

    #[test]
    fn parse_markdown_link() {
        let blocks = parse_markdown("[link text](http://example.com)");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        assert!(matches!(&spans[0], InlineSpan::Link { text } if text == "link text"));
    }

    #[test]
    fn parse_markdown_strikethrough() {
        // Strikethrough events are not explicitly handled in the parser,
        // so ~~strike~~ becomes plain text "strike" (markers stripped by pulldown_cmark
        // when ENABLE_STRIKETHROUGH is set but no handler exists).
        let blocks = parse_markdown("~~strike~~");
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        // strikethrough is not handled, falls through to default; text is emitted as-is
        assert!(!spans.is_empty());
    }

    #[test]
    fn parse_markdown_multiline_paragraph() {
        let blocks = parse_markdown("Line one\nLine two");
        // pulldown_cmark emits SoftBreak between lines in the same paragraph
        let MarkdownBlock::Paragraph { spans } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        // SoftBreak becomes a space
        assert!(!spans.is_empty());
    }

    #[test]
    fn parse_markdown_table() {
        let blocks = parse_markdown("| a | b |\n|---|---|\n| 1 | 2 |");
        // Tables are supported via ENABLE_TABLES
        assert!(!blocks.is_empty());
    }

    // --- spans_to_plain_text tests ---

    #[test]
    fn spans_to_plain_text_text_only() {
        let spans = vec![InlineSpan::Text("hello".to_string())];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "hello");
    }

    #[test]
    fn spans_to_plain_text_bold() {
        let spans = vec![InlineSpan::Bold("bold".to_string())];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "bold");
    }

    #[test]
    fn spans_to_plain_text_italic() {
        let spans = vec![InlineSpan::Italic("italic".to_string())];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "italic");
    }

    #[test]
    fn spans_to_plain_text_bold_italic() {
        let spans = vec![InlineSpan::BoldItalic("bi".to_string())];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "bi");
    }

    #[test]
    fn spans_to_plain_text_code() {
        let spans = vec![InlineSpan::Code("code".to_string())];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "code");
    }

    #[test]
    fn spans_to_plain_text_link() {
        let spans = vec![InlineSpan::Link {
            text: "link text".to_string(),
        }];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "link text");
    }

    #[test]
    fn spans_to_plain_text_multiple_spans() {
        let spans = vec![
            InlineSpan::Text("hello".to_string()),
            InlineSpan::Bold(" bold".to_string()),
            InlineSpan::Italic(" italic".to_string()),
        ];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "hello bold italic");
    }

    #[test]
    fn spans_to_plain_text_empty() {
        let spans: Vec<InlineSpan> = vec![];
        let result: SharedString = spans_to_plain_text(&spans);
        assert_eq!(result.to_string(), "");
    }
}
