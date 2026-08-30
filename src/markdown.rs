// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! CommonMark → [`ratatui`] text rendering for TUI hosts.
//!
//! This is a display-only renderer: it parses a markdown string and produces
//! styled [`Line`]s — headings, emphasis (bold/italic/strikethrough), inline
//! and block code, bullet/ordered lists, block quotes, task lists, links and
//! horizontal rules.
//!
//! Line breaks in the source are preserved (soft breaks become new lines), so
//! multi-line LLM replies read like the raw text instead of being collapsed
//! onto a single line.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Style palette used by the markdown renderer. Tweak these to fit a host theme.
#[derive(Debug, Clone)]
pub struct MarkdownStyles {
    /// Headings (`#` … `######`).
    pub heading: Style,
    /// Inline `` `code` `` and fenced/indented code blocks.
    pub code: Style,
    /// Link text `[text](url)`.
    pub link: Style,
    /// Block-quote marker (`>`).
    pub quote: Style,
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        Self {
            heading: Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            code: Style::default().fg(Color::DarkGray),
            link: Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::UNDERLINED),
            quote: Style::default().fg(Color::Gray),
        }
    }
}

/// Render `markdown` into styled [`Line`]s using the default style palette.
pub fn render_markdown(markdown: &str) -> Vec<Line<'static>> {
    render_markdown_with_styles(markdown, &MarkdownStyles::default())
}

/// Render `markdown` into styled [`Line`]s using a custom style palette.
pub fn render_markdown_with_styles(markdown: &str, styles: &MarkdownStyles) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut modifiers: Vec<Modifier> = Vec::new();
    let mut link_depth = 0usize;
    let mut block_style = Style::default();
    let mut in_code_block = false;
    let mut code_buf = String::new();

    // Block container state: quote markers / list indents / item bullets.
    let mut prefixes: Vec<Span<'static>> = Vec::new();
    let mut bullet_index: Option<usize> = None;
    let mut ordered_lists: Vec<Option<u64>> = Vec::new();
    let mut item_counter = 0u64;
    let mut container_depth = 0usize;

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => blank_between_blocks(&mut lines, container_depth),
                Tag::Heading { .. } => {
                    blank_between_blocks(&mut lines, container_depth);
                    block_style = styles.heading;
                }
                Tag::CodeBlock(_) => {
                    blank_between_blocks(&mut lines, container_depth);
                    in_code_block = true;
                    code_buf.clear();
                }
                Tag::BlockQuote(_) => {
                    // Complete any pending text (e.g. the tail of an item)
                    // before the quote's own lines start.
                    push_line(&mut lines, &mut cur, &prefixes);
                    blank_between_blocks(&mut lines, container_depth);
                    container_depth += 1;
                    prefixes.push(Span::styled("▎ ".to_string(), styles.quote));
                }
                Tag::List(number) => {
                    // A list directly inside an item: close the item's current
                    // line and turn its bullet into an indent so the nested
                    // list lines up underneath it.
                    if !ordered_lists.is_empty() {
                        push_line(&mut lines, &mut cur, &prefixes);
                        if let Some(idx) = bullet_index {
                            if let Some(bullet) = prefixes.get(idx) {
                                let width = bullet.width();
                                prefixes[idx] = Span::raw(" ".repeat(width));
                            }
                            bullet_index = None;
                        }
                    }
                    blank_between_blocks(&mut lines, container_depth);
                    ordered_lists.push(number);
                    item_counter = 0;
                    container_depth += 1;
                }
                Tag::Item => {
                    item_counter += 1;
                    let bullet = match ordered_lists.last() {
                        Some(Some(start)) => format!("{}. ", start + item_counter - 1),
                        _ => "• ".to_string(),
                    };
                    prefixes.push(Span::raw(bullet));
                    bullet_index = Some(prefixes.len() - 1);
                }
                Tag::Emphasis => modifiers.push(Modifier::ITALIC),
                Tag::Strong => modifiers.push(Modifier::BOLD),
                Tag::Strikethrough => modifiers.push(Modifier::CROSSED_OUT),
                Tag::Link { .. } => link_depth += 1,
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph | TagEnd::Heading(_) => {
                    push_line(&mut lines, &mut cur, &prefixes);
                    block_style = Style::default();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    // A trailing newline is a terminator, not an empty line.
                    let body = code_buf.strip_suffix('\n').unwrap_or(&code_buf);
                    for segment in body.split('\n') {
                        lines.push(Line::from(Span::styled(segment.to_string(), styles.code)));
                    }
                    code_buf.clear();
                    block_style = Style::default();
                }
                TagEnd::BlockQuote(_) => {
                    prefixes.pop();
                    container_depth = container_depth.saturating_sub(1);
                    push_line(&mut lines, &mut cur, &prefixes);
                    block_style = Style::default();
                }
                TagEnd::List(_) => {
                    ordered_lists.pop();
                    container_depth = container_depth.saturating_sub(1);
                    push_line(&mut lines, &mut cur, &prefixes);
                }
                TagEnd::Item => {
                    // For tight lists the item text is still pending here, so
                    // flush it *before* removing the bullet prefix.
                    push_line(&mut lines, &mut cur, &prefixes);
                    prefixes.pop();
                    bullet_index = None;
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    modifiers.pop();
                }
                TagEnd::Link => link_depth = link_depth.saturating_sub(1),
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_buf.push_str(&text);
                } else {
                    cur.push(Span::styled(
                        text.to_string(),
                        inline_style(block_style, &modifiers, link_depth, styles),
                    ));
                }
            }
            Event::Code(text) => {
                cur.push(Span::styled(text.to_string(), styles.code));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                cur.push(Span::styled(text.to_string(), styles.code));
            }
            Event::SoftBreak | Event::HardBreak => {
                push_line(&mut lines, &mut cur, &prefixes);
                // A wrapped list item indents under its bullet instead of
                // repeating the bullet on the continuation line.
                if let Some(idx) = bullet_index {
                    if let Some(bullet) = prefixes.get(idx) {
                        let width = bullet.width();
                        prefixes[idx] = Span::raw(" ".repeat(width));
                    }
                }
            }
            Event::Rule => {
                push_line(&mut lines, &mut cur, &prefixes);
                lines.push(Line::from(Span::styled("─".repeat(40), styles.quote)));
            }
            Event::TaskListMarker(checked) => {
                cur.push(Span::styled(
                    if checked { "☑ " } else { "☐ " }.to_string(),
                    Style::default(),
                ));
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                cur.push(Span::styled(text.to_string(), Style::default()));
            }
            Event::FootnoteReference(_) => {}
        }
    }

    push_line(&mut lines, &mut cur, &prefixes);
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

/// Flush the current inline spans into `lines`, prefixing them with any active
/// block markers (block-quote arrows, list bullets, indents).
fn push_line(
    lines: &mut Vec<Line<'static>>,
    cur: &mut Vec<Span<'static>>,
    prefixes: &[Span<'static>],
) {
    if cur.is_empty() {
        return;
    }
    let mut spans = Vec::with_capacity(cur.len() + prefixes.len());
    for prefix in prefixes {
        spans.push(prefix.clone());
    }
    spans.append(cur);
    lines.push(Line::from(spans));
    cur.clear();
}

/// Insert a blank line between top-level blocks so paragraphs don't run
/// together. Container content (lists, block quotes) is left untouched.
fn blank_between_blocks(lines: &mut Vec<Line<'static>>, container_depth: usize) {
    if container_depth > 0 {
        return;
    }
    if let Some(last) = lines.last() {
        if !last.spans.is_empty() {
            lines.push(Line::from(""));
        }
    }
}

/// Combine the current block style with active inline modifiers.
fn inline_style(
    base: Style,
    modifiers: &[Modifier],
    link_depth: usize,
    styles: &MarkdownStyles,
) -> Style {
    let mut style = base;
    for modifier in modifiers {
        style = style.add_modifier(*modifier);
    }
    if link_depth > 0 {
        style = style.patch(styles.link);
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect()
    }

    #[test]
    fn soft_breaks_become_lines() {
        let out = render_markdown("line one\nline two");
        assert_eq!(text(&out), ["line one", "line two"]);
    }

    #[test]
    fn paragraphs_are_separated_by_blank_line() {
        let out = render_markdown("first paragraph\n\nsecond paragraph");
        assert_eq!(text(&out), ["first paragraph", "", "second paragraph"]);
    }

    #[test]
    fn bold_italic_and_code_are_styled() {
        let out = render_markdown("**bold** and *italic* and `code`");
        assert_eq!(out.len(), 1);
        let spans = &out[0].spans;
        assert!(spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
        assert!(spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::ITALIC)));
        assert!(spans.iter().any(|s| s.style.fg == Some(Color::DarkGray)));
    }

    #[test]
    fn heading_gets_heading_style() {
        let out = render_markdown("# Title");
        assert_eq!(text(&out), ["Title"]);
        assert!(out[0]
            .spans
            .iter()
            .all(|s| s.style.fg == Some(Color::LightYellow)
                && s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn fenced_code_block_preserves_lines() {
        let out = render_markdown("before\n\n```\nlet x = 1;\nlet y = 2;\n```\n\nafter");
        assert_eq!(
            text(&out),
            ["before", "", "let x = 1;", "let y = 2;", "", "after"]
        );
        assert!(out
            .iter()
            .any(|l| l.spans.iter().any(|s| s.style.fg == Some(Color::DarkGray))));
    }

    #[test]
    fn bullet_list_prefixes_items() {
        let out = render_markdown("- alpha\n- beta");
        assert_eq!(text(&out), ["• alpha", "• beta"]);
    }

    #[test]
    fn ordered_list_numbers_items() {
        let out = render_markdown("1. first\n2. second");
        assert_eq!(text(&out), ["1. first", "2. second"]);
    }

    #[test]
    fn blockquote_gets_marker() {
        let out = render_markdown("> quoted text");
        assert_eq!(text(&out), ["▎ quoted text"]);
    }

    #[test]
    fn empty_input_renders_one_empty_line() {
        assert_eq!(text(&render_markdown("")), [""]);
    }

    #[test]
    fn nested_list_indents_inner_items() {
        let out = render_markdown("- outer\n  - inner\n- again");
        assert_eq!(text(&out), ["• outer", "  • inner", "• again"]);
    }

    #[test]
    fn wrapped_list_item_uses_indent_for_continuation() {
        let out = render_markdown("- first line of a long item\n  second line");
        assert_eq!(text(&out), ["• first line of a long item", "  second line"]);
    }

    #[test]
    fn link_text_is_styled_and_plain_text_kept() {
        let out = render_markdown("see [fastclaw](https://example.com) now");
        assert_eq!(text(&out), ["see fastclaw now"]);
        assert!(out[0].spans.iter().any(|s| s.content.as_ref() == "fastclaw"
            && s.style.add_modifier.contains(Modifier::UNDERLINED)
            && s.style.fg == Some(Color::LightBlue)));
    }

    #[test]
    fn realistic_reply_keeps_structure() {
        let md = "## Summary\n\nHere is a **bold** point.\n\n- item one\n- item two\n\n```rust\nfn main() {}\n```";
        let out = render_markdown(md);
        assert_eq!(
            text(&out),
            [
                "Summary",
                "",
                "Here is a bold point.",
                "",
                "• item one",
                "• item two",
                "",
                "fn main() {}",
            ]
        );
    }
}
