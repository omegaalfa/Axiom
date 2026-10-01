use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetPlaceholder {
    pub index: u32,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetExpansion {
    pub text: String,
    pub placeholders: Vec<SnippetPlaceholder>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetSession {
    pub placeholders: Vec<SnippetPlaceholder>,
    pub current: usize,
}

pub fn expand(source: &str) -> SnippetExpansion {
    let mut text = String::new();
    let mut placeholders = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if ch != '$' {
            text.push(ch);
            continue;
        }
        if chars.peek().is_some_and(|(_, next)| *next == '{') {
            chars.next();
            let start = text.len();
            let mut body = String::new();
            for (_, next) in chars.by_ref() {
                if next == '}' {
                    break;
                }
                body.push(next);
            }
            let Some((index, default)) = body.split_once(':') else {
                continue;
            };
            let Ok(index) = index.parse::<u32>() else { continue };
            text.push_str(default);
            placeholders.push(SnippetPlaceholder {
                index,
                range: start..text.len(),
            });
        } else {
            let start = offset;
            let mut end = start + 1;
            while chars.peek().is_some_and(|(_, next)| next.is_ascii_digit()) {
                end = chars.next().map(|(at, next)| at + next.len_utf8()).unwrap_or(end);
            }
            let index = source[start + 1..end].parse::<u32>().ok();
            if let Some(0) = index {
                placeholders.push(SnippetPlaceholder {
                    index: 0,
                    range: text.len()..text.len(),
                });
            }
        }
    }
    placeholders.sort_by_key(|placeholder| (placeholder.index == 0, placeholder.index));
    SnippetExpansion { text, placeholders }
}

impl SnippetSession {
    pub fn new(mut placeholders: Vec<SnippetPlaceholder>) -> Option<Self> {
        placeholders.sort_by_key(|placeholder| (placeholder.index == 0, placeholder.index));
        (!placeholders.is_empty()).then_some(Self { placeholders, current: 0 })
    }

    pub fn active_range(&self) -> Option<Range<usize>> {
        self.placeholders.get(self.current).map(|placeholder| placeholder.range.clone())
    }

    pub fn next(&mut self) -> Option<Range<usize>> {
        if self.current + 1 >= self.placeholders.len() {
            self.placeholders.clear();
            return None;
        }
        self.current += 1;
        self.active_range()
    }

    pub fn previous(&mut self) -> Option<Range<usize>> {
        if self.current == 0 { return self.active_range(); }
        self.current -= 1;
        self.active_range()
    }
}

#[cfg(test)]
mod tests {
    use super::expand;

    #[test]
    fn expands_supported_placeholders() {
        let result = expand("foreach (${1:$items} as ${2:$item}) {\n$0\n}");
        assert_eq!(result.text, "foreach ($items as $item) {\n\n}");
        assert_eq!(result.placeholders.iter().map(|p| p.index).collect::<Vec<_>>(), vec![1, 2, 0]);
    }

    #[test]
    fn session_navigates_forward_and_backward() {
        let expansion = expand("${1:á} ${2:value} $0");
        let mut session = super::SnippetSession::new(expansion.placeholders).unwrap();
        assert_eq!(session.active_range(), Some(0..2));
        assert_eq!(session.next(), Some(3..8));
        assert_eq!(session.next(), Some(9..9));
        assert_eq!(session.previous(), Some(3..8));
    }
}
