//! Small native PHP formatter used when no language-server formatter is available.

pub fn format_php(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + text.len() / 4);
    let mut indent = 0usize;
    let mut block_comment = false;
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.trim();
        if let Some(delimiter) = heredoc.as_ref() {
            if line_index > 0 {
                result.push('\n');
            }
            result.push_str(raw_line);
            if trimmed == delimiter || trimmed == format!("{delimiter};") {
                heredoc = None;
            }
            continue;
        }
        let preserves_quoted_text = quote.is_some();
        if trimmed.is_empty() && !preserves_quoted_text {
            if line_index > 0 {
                result.push('\n');
            }
            continue;
        }
        let heredoc_start = (!preserves_quoted_text)
            .then(|| heredoc_delimiter(trimmed))
            .flatten();
        let closes = !preserves_quoted_text
            && (trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')'));
        if closes {
            indent = indent.saturating_sub(1);
        }
        if line_index > 0 {
            result.push('\n');
        }
        if preserves_quoted_text {
            result.push_str(raw_line);
        } else {
            result.push_str(&"    ".repeat(indent));
            result.push_str(&normalize_line(trimmed));
        }
        if let Some(delimiter) = heredoc_start.filter(|delimiter| !delimiter.is_empty()) {
            heredoc = Some(delimiter);
            continue;
        }
        let mut chars = if preserves_quoted_text {
            raw_line
        } else {
            trimmed
        }
        .chars()
        .peekable();
        let mut opens = 0usize;
        let mut closes_on_line = 0usize;
        while let Some(ch) = chars.next() {
            if block_comment {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    block_comment = false;
                }
                continue;
            }
            if let Some(active_quote) = quote {
                if ch == '\\' {
                    chars.next();
                } else if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_comment = true;
            } else if (ch == '/' && chars.peek() == Some(&'/')) || ch == '#' {
                break;
            } else if matches!(ch, '\'' | '"' | '`') {
                quote = Some(ch);
            } else if matches!(ch, '{' | '[' | '(') {
                opens += 1;
            } else if matches!(ch, '}' | ']' | ')') {
                closes_on_line += 1;
            }
        }
        indent = indent.saturating_add(opens);
        indent = indent.saturating_sub(closes_on_line.saturating_sub(usize::from(closes)));
    }
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn normalize_line(line: &str) -> String {
    if line.contains(['\'', '"', '`']) || line.contains("//") || line.contains("/*") {
        return line.to_owned();
    }
    let mut out = String::new();
    let mut code = String::new();
    let mut quote = None;
    let flush = |out: &mut String, code: &mut String| {
        if !code.is_empty() {
            let mut s = code.split_whitespace().collect::<Vec<_>>().join(" ");
            s = s.replace("=>", "\0AXIOM_ARROW\0");
            for op in ["===", "??", "=", "+"] {
                let spaced = format!(" {op} ");
                s = s
                    .replace(op, &spaced)
                    .replace(&format!("  {op}  "), &spaced);
            }
            s = s
                .replace("private (set)", "private(set)")
                .replace("protected (set)", "protected(set)")
                .replace("public (set)", "public(set)")
                .replace(" ? - > ", "?->")
                .replace(" ?-> ", "?->")
                .replace(" -> ", "->")
                .replace(" :: ", "::")
                .replace(" - > ", "->")
                .replace(" : : ", "::")
                .replace(" ( ", "(")
                .replace(" )", ")")
                .replace(" ,", ",")
                .replace(",  ", ", ")
                .replace("\0AXIOM_ARROW\0", "=>");
            out.push_str(&s);
            code.clear();
        }
    };
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            out.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            flush(&mut out, &mut code);
            quote = Some(ch);
            out.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            flush(&mut out, &mut code);
            out.push(ch);
            out.push(chars.next().unwrap());
            out.extend(chars);
            break;
        }
        code.push(ch);
    }
    flush(&mut out, &mut code);
    out
}

fn heredoc_delimiter(line: &str) -> Option<String> {
    let marker = line.find("<<<")?;
    let value = line[marker + 3..].trim_start();
    let (delimiter, rest) = match value.chars().next()? {
        quote @ ('\'' | '"') => {
            let value = &value[quote.len_utf8()..];
            let end = value.find(quote)?;
            (&value[..end], &value[end + quote.len_utf8()..])
        }
        _ => {
            let end = value
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(value.len());
            (&value[..end], &value[end..])
        }
    };
    (!delimiter.is_empty()
        && delimiter
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && rest.trim().is_empty())
    .then(|| delimiter.to_owned())
}

#[cfg(test)]
mod tests {
    use super::format_php;

    #[test]
    fn preserves_php84_asymmetric_visibility_and_property_hooks() {
        let input = "<?php\nclass User {\npublic private (set) string $name { get => $this->_name; }\nprotected private (set) ?int $id = null;\npublic readonly private (set) string $uuid;\npublic string $title {\nget {\nreturn ucfirst($this->_title);\n}\n}\n}\n";
        let output = format_php(input);
        assert!(output.contains("public private(set) string $name { get => $this->_name; }"));
        assert!(output.contains("protected private(set) ?int $id = null;"));
        assert!(output.contains("public readonly private(set) string $uuid;"));
        assert!(output.contains("get =>"));
        assert!(output.contains("return ucfirst($this->_title);"));
        assert!(!output.contains("private (set)"));
        assert!(!output.contains("get = >"));
    }

    #[test]
    fn formats_arrow_operators_without_rewriting_assignment_spacing() {
        assert_eq!(
            format_php("<?php\n$fn = fn($x) => $x;\n"),
            "<?php\n$fn = fn($x) => $x;\n"
        );
    }
}
