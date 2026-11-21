//! A small syntax highlighter for the Inspect code blocks (CSS, Tailwind/HTML/JSX, Swift, Kotlin).
//!
//! It is a one-pass tokenizer per line, not a parser: good enough to color properties, numbers,
//! strings, types and calls consistently in the generated code.

use super::Language;
use crate::theme;
use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Property,
    Keyword,
    Str,
    Number,
    Type,
    Function,
    Comment,
    Punct,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub tok: Tok,
    pub text: String,
}

fn span(tok: Tok, text: impl Into<String>) -> Span {
    Span { tok, text: text.into() }
}

pub fn color(t: Tok) -> Color32 {
    match t {
        Tok::Plain => theme::SYNTAX_PLAIN,
        Tok::Property => theme::SYNTAX_PROPERTY,
        Tok::Keyword => theme::SYNTAX_KEYWORD,
        Tok::Str => theme::SYNTAX_STRING,
        Tok::Number => theme::SYNTAX_NUMBER,
        Tok::Type => theme::SYNTAX_TYPE,
        Tok::Function => theme::SYNTAX_FUNCTION,
        Tok::Comment => theme::SYNTAX_COMMENT,
        Tok::Punct => theme::SYNTAX_PUNCT,
    }
}

fn keywords(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Css | Language::Tailwind => &["true", "false"],
        Language::SwiftUi => &["let", "var", "func", "struct", "some", "return", "if", "else", "true", "false", "nil", "import", "self", "in"],
        Language::Compose => &["val", "var", "fun", "return", "if", "else", "true", "false", "null", "import", "class", "object"],
        Language::React => &["export", "function", "return", "const", "true", "false", "null", "import"],
        Language::Flutter => &["class", "extends", "return", "const", "final", "Widget", "build", "true", "false", "null"],
        Language::Vue => &["template", "script", "setup", "style", "scoped", "true", "false", "null"],
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '@'
}

/// Tokenizes `code` line by line.
pub fn highlight(lang: Language, code: &str) -> Vec<Vec<Span>> {
    code.lines().map(|l| line(lang, l)).collect()
}

fn line(lang: Language, src: &str) -> Vec<Span> {
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut i = 0;
    let hyphen_idents = lang == Language::Css;
    while i < chars.len() {
        let c = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        if c.is_whitespace() {
            let s = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push(span(Tok::Plain, chars[s..i].iter().collect::<String>()));
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            out.push(span(Tok::Comment, chars[i..].iter().collect::<String>()));
            i = chars.len();
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let s = i;
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            out.push(span(Tok::Comment, chars[s..i].iter().collect::<String>()));
        } else if c == '"' || c == '\'' {
            let s = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            out.push(span(Tok::Str, chars[s..i].iter().collect::<String>()));
        } else if c == '#' && chars.get(i + 1).is_some_and(|d| d.is_ascii_hexdigit()) {
            let s = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            out.push(span(Tok::Number, chars[s..i].iter().collect::<String>()));
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) && !prev.is_some_and(|p| p.is_alphanumeric() || p == ')'))
        {
            let s = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
                i += 1;
            }
            // unit suffix: px, %, deg, f ...
            while i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '%') {
                i += 1;
            }
            out.push(span(Tok::Number, chars[s..i].iter().collect::<String>()));
        } else if is_ident_start(c) {
            let s = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || (hyphen_idents && chars[i] == '-')) {
                i += 1;
            }
            let word: String = chars[s..i].iter().collect();
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let next = chars.get(j).copied();
            let after = chars.get(j + 1).copied();
            let tok = if prev == Some('<') || (prev == Some('/') && s >= 2 && chars[s - 2] == '<') {
                Tok::Type
            } else if prev == Some('.') && s >= 1 {
                if next == Some('(') {
                    Tok::Function
                } else {
                    Tok::Plain
                }
            } else if next == Some('(') {
                if word.chars().next().is_some_and(char::is_uppercase) {
                    Tok::Type
                } else {
                    Tok::Function
                }
            } else if (next == Some(':') || (next == Some('=') && after != Some('=') && after != Some('>'))) && !keywords(lang).contains(&word.as_str()) {
                Tok::Property
            } else if keywords(lang).contains(&word.as_str()) {
                Tok::Keyword
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                Tok::Type
            } else {
                Tok::Plain
            };
            out.push(span(tok, word));
        } else {
            let punct = "{}()[],;:.<>/=+*!?|&";
            out.push(span(if punct.contains(c) { Tok::Punct } else { Tok::Plain }, c.to_string()));
            i += 1;
        }
    }
    out
}

/// Splits a tokenized line into rows of at most `cols` characters, breaking at spaces and
/// indenting continuation rows two columns deeper than the line.
pub fn wrap(line: &[Span], cols: usize) -> Vec<Vec<Span>> {
    let cols = cols.max(8);
    let len = |row: &[Span]| row.iter().map(|s| s.text.chars().count()).sum::<usize>();
    let indent = line.first().filter(|s| s.tok == Tok::Plain && s.text.trim().is_empty()).map(|s| s.text.chars().count()).unwrap_or(0);
    let mut rows: Vec<Vec<Span>> = vec![Vec::new()];
    for s in line {
        for piece in s.text.split_inclusive(' ') {
            let w = piece.chars().count();
            let row_len = len(rows.last().unwrap());
            // closing brackets and commas may hang over the edge instead of wrapping alone
            let hang = piece.trim().chars().count() <= 2 && piece.trim().chars().all(|c| "),;]}".contains(c));
            if row_len + w > cols && row_len > indent + 2 && !piece.trim().is_empty() && !hang {
                rows.push(vec![span(Tok::Plain, " ".repeat(indent + 2))]);
            }
            rows.last_mut().unwrap().push(span(s.tok, piece));
        }
    }
    if rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(l: &[Span]) -> Vec<(Tok, &str)> {
        l.iter().map(|s| (s.tok, s.text.as_str())).collect()
    }

    #[test]
    fn css_declaration() {
        let l = &highlight(Language::Css, "border-radius: 8px;")[0];
        assert_eq!(kinds(l), vec![(Tok::Property, "border-radius"), (Tok::Punct, ":"), (Tok::Plain, " "), (Tok::Number, "8px"), (Tok::Punct, ";")]);
        let l = &highlight(Language::Css, "background: var(--color-primary);")[0];
        assert!(kinds(l).contains(&(Tok::Function, "var")));
        let l = &highlight(Language::Css, "color: #FFF;")[0];
        assert!(kinds(l).contains(&(Tok::Number, "#FFF")));
    }

    #[test]
    fn swift_and_kotlin() {
        let l = &highlight(Language::SwiftUi, "    .padding(16)")[0];
        assert_eq!(kinds(l)[2], (Tok::Function, "padding"));
        assert!(kinds(l).contains(&(Tok::Number, "16")));
        let l = &highlight(Language::SwiftUi, "VStack(alignment: .leading, spacing: 8) {")[0];
        assert_eq!(kinds(l)[0], (Tok::Type, "VStack"));
        assert!(kinds(l).contains(&(Tok::Property, "alignment")));
        let l = &highlight(Language::Compose, "text = \"Hi\",")[0];
        assert_eq!(kinds(l)[0], (Tok::Property, "text"));
        assert!(kinds(l).contains(&(Tok::Str, "\"Hi\"")));
        let l = &highlight(Language::Compose, "color = Color(0xFF111111)")[0];
        assert!(kinds(l).contains(&(Tok::Type, "Color")));
    }

    #[test]
    fn markup() {
        let l = &highlight(Language::Tailwind, r#"<div class="flex p-4">"#)[0];
        assert_eq!(kinds(l)[1], (Tok::Type, "div"));
        assert!(kinds(l).contains(&(Tok::Property, "class")));
        assert!(kinds(l).contains(&(Tok::Str, "\"flex p-4\"")));
        let l = &highlight(Language::React, "export function Card() {")[0];
        assert_eq!(kinds(l)[0], (Tok::Keyword, "export"));
        assert!(kinds(l).contains(&(Tok::Type, "Card")));
    }

    #[test]
    fn comments_and_strings_do_not_leak() {
        let l = &highlight(Language::SwiftUi, "Text(\"a // b\") // note")[0];
        assert!(kinds(l).contains(&(Tok::Str, "\"a // b\"")));
        assert_eq!(kinds(l).last().unwrap().0, Tok::Comment);
    }

    #[test]
    fn wrapping_keeps_every_character_and_indents_continuations() {
        let l = &highlight(Language::Css, "    box-shadow: 0px 4px 8px 0px rgba(0, 0, 0, 0.25), 0px 1px 2px 0px rgba(0, 0, 0, 0.5);")[0];
        let rows = wrap(l, 30);
        assert!(rows.len() > 2);
        let joined: String = rows.iter().map(|r| r.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("");
        let orig: String = l.iter().map(|s| s.text.as_str()).collect();
        // continuation rows add indentation only
        assert_eq!(joined.replace("      ", "").replace(' ', ""), orig.replace(' ', ""));
        assert!(rows[1][0].text.starts_with("      "));
    }
}
