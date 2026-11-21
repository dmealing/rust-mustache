use std::error::Error as StdError;
use std::mem;
use std::fmt;

// for bug!
use log::error;

/// `Token` is a section of a compiled mustache string.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Text(String),
    EscapedTag(Vec<String>, String),
    UnescapedTag(Vec<String>, String),
    Section(Vec<String>, bool, Vec<Token>, String, String, String, String, String),
    IncompleteSection(Vec<String>, bool, String),
    Partial(String, String, String),
}

/// Error type to represent parsing failure.
///
/// This type is not intended to be matched exhaustively as new variants
/// may be added in future without a version bump.
#[derive(Debug, PartialEq)]
#[allow(clippy::manual_non_exhaustive)]
pub enum Error {
    BadClosingTag(char, char),
    UnclosedTag,
    UnclosedSection(String),
    UnbalancedUnescapeTag,
    EmptyTag,
    EarlySectionClose(String),
    MissingSetDelimeterClosingTag,
    InvalidSetDelimeterSyntax,

    #[doc(hidden)]
    __Nonexhaustive,
}

impl StdError for Error { }

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Provide more information where possible
        match *self {
            Error::BadClosingTag(actual, expected) => write!(f, "character {:?} was unexpected in the closing tag, expected {:?}", actual, expected),
            Error::UnclosedSection(ref name) => write!(f, "found an unclosed section: {:?}", name),
            Error::EarlySectionClose(ref name) => write!(f, "found a closing tag for an unopened section {:?}", name),
            Error::UnclosedTag => write!(f, "found an unclosed tag"),
            Error::UnbalancedUnescapeTag => write!(f, "found an unbalanced unescape tag"),
            Error::EmptyTag => write!(f, "found an empty tag",),
            Error::MissingSetDelimeterClosingTag => write!(f, "missing the new closing tag in set delimeter tag"),
            Error::InvalidSetDelimeterSyntax => write!(f, "invalid set delimeter tag syntax"),
            Error::__Nonexhaustive => unreachable!(),
        }
    }
}

/// A piece of the template as it appears in the source, before any
/// whitespace handling has been applied.
enum Item {
    Text(String),
    Tag {
        /// Everything between the delimiters.
        content: String,
        /// The tag as written, including the delimiters in force at that point.
        source: String,
    },
}

/// `Parser` parses a string into a series of `Token`s.
///
/// Parsing happens in three passes so that whether a tag is "standalone"
/// is always decided against the original text, never against text that
/// an earlier tag has already trimmed:
///
/// 1. `scan` splits the input into raw text and tag items.
/// 2. `analyze` decides which tags are standalone and how much whitespace
///    around each of them has to go.
/// 3. `build` applies those decisions and assembles the tokens.
pub struct Parser<'a, T: 'a> {
    reader: &'a mut T,
    ch: Option<char>,
    content: String,
    state: ParserState,
    opening_tag: String,
    closing_tag: String,
    opening_tag_chars: Vec<char>,
    closing_tag_chars: Vec<char>,
    tag_position: usize,
    items: Vec<Item>,
    tokens: Vec<Token>,
    partials: Vec<String>,
}

enum ParserState {
    Text,
    OpeningTag,
    Tag,
    ClosingTag,
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Whitespace to remove around the item at the same index.
#[derive(Clone, Copy, Default)]
struct Cut {
    /// Bytes to drop from the start of a text item.
    start: usize,
    /// Bytes to drop from the end of a text item.
    end: usize,
}

impl<'a, T: Iterator<Item = char>> Parser<'a, T> {
    pub fn new(reader: &'a mut T, opening_tag: &str, closing_tag: &str) -> Parser<'a, T> {
        let mut parser = Parser {
            reader,
            ch: None,
            content: String::new(),
            state: ParserState::Text,
            opening_tag: opening_tag.to_string(),
            closing_tag: closing_tag.to_string(),
            opening_tag_chars: opening_tag.chars().collect(),
            closing_tag_chars: closing_tag.chars().collect(),
            tag_position: 0,
            items: Vec::new(),
            tokens: Vec::new(),
            partials: Vec::new(),
        };

        parser.bump();
        parser
    }

    fn bump(&mut self) {
        self.ch = self.reader.next();
    }

    /// Parse the template into tokens and a list of partial files.
    pub fn parse(mut self) -> Result<(Vec<Token>, Vec<String>), Error> {
        self.scan()?;
        let (cuts, indents) = self.analyze();
        self.build(&cuts, &indents)?;

        // Check that we don't have any incomplete sections.
        for token in self.tokens.iter().rev() {
            if let Token::IncompleteSection(ref path, _, _) = *token {
                return Err(Error::UnclosedSection(path.join(".")))
            }
        }

        let Parser { tokens, partials, .. } = self;

        Ok((tokens, partials))
    }

    /// Pass 1: split the input into text and tags. Delimiter changes take
    /// effect here, since they decide where the following tags start.
    fn scan(&mut self) -> Result<(), Error> {
        let mut curly_brace_tag = false;

        while let Some(ch) = self.ch {
            match self.state {
                ParserState::Text => {
                    if ch == self.opening_tag_chars[0] {
                        if self.opening_tag_chars.len() > 1 {
                            self.tag_position = 1;
                            self.state = ParserState::OpeningTag;
                        } else {
                            self.add_text();
                            self.state = ParserState::Tag;
                        }
                    } else {
                        self.content.push(ch);
                    }
                    self.bump();
                }
                ParserState::OpeningTag => {
                    if ch == self.opening_tag_chars[self.tag_position] {
                        if self.tag_position == self.opening_tag_chars.len() - 1 {
                            self.add_text();
                            curly_brace_tag = false;
                            self.state = ParserState::Tag;
                        } else {
                            self.tag_position += 1;
                        }
                    } else {
                        // We don't have a tag, so add all the tag parts we've seen
                        // so far to the string.
                        self.state = ParserState::Text;
                        self.not_otag();
                        self.content.push(ch);
                    }
                    self.bump();
                }
                ParserState::Tag => {
                    if self.content.is_empty() && ch == '{' {
                        curly_brace_tag = true;
                        self.content.push(ch);
                        self.bump();
                    } else if curly_brace_tag && ch == '}' {
                        curly_brace_tag = false;
                        self.content.push(ch);
                        self.bump();
                    } else if ch == self.closing_tag_chars[0] {
                        if self.closing_tag_chars.len() > 1 {
                            self.tag_position = 1;
                            self.state = ParserState::ClosingTag;
                            self.bump();
                        } else {
                            self.scan_tag()?;
                            self.state = ParserState::Text;
                        }
                    } else {
                        self.content.push(ch);
                        self.bump();
                    }
                }
                ParserState::ClosingTag => {
                    if ch == self.closing_tag_chars[self.tag_position] {
                        if self.tag_position == self.closing_tag_chars.len() - 1 {
                            self.scan_tag()?;
                            self.state = ParserState::Text;
                        } else {
                            self.state = ParserState::Tag;
                            self.not_ctag();
                            self.content.push(ch);
                            self.bump();
                        }
                    } else {
                        let expected = self.closing_tag_chars[self.tag_position];
                        return Err(Error::BadClosingTag(ch, expected));
                    }
                }
            }
        }

        match self.state {
            ParserState::Text => {
                self.add_text();
            }
            ParserState::OpeningTag => {
                self.not_otag();
                self.add_text();
            }
            ParserState::ClosingTag => {
                self.not_ctag();
                self.add_text();
            }
            ParserState::Tag => return Err(Error::UnclosedTag),
        }

        Ok(())
    }

    fn add_text(&mut self) {
        if !self.content.is_empty() {
            let content = mem::take(&mut self.content);
            self.items.push(Item::Text(content));
        }
    }

    /// Record the tag that has just been closed, applying a delimiter
    /// change if that is what it is.
    fn scan_tag(&mut self) -> Result<(), Error> {
        self.bump();

        let source = self.opening_tag.clone() + &self.content + &self.closing_tag;
        let content = mem::take(&mut self.content);
        deny_blank(&content)?;

        if content.starts_with('=') {
            self.set_delimiters(&content)?;
        }

        self.items.push(Item::Tag { content, source });
        Ok(())
    }

    fn set_delimiters(&mut self, content: &str) -> Result<(), Error> {
        let len = content.len();

        if len > 2usize && content.ends_with('=') {
            let s = deny_blank(&content[1..len - 1])?;

            let pos = s.find(char::is_whitespace);
            let pos = match pos {
                None => return Err(Error::MissingSetDelimeterClosingTag),
                Some(pos) => pos,
            };

            self.opening_tag = s[0..pos].to_string();
            self.opening_tag_chars = self.opening_tag.chars().collect();

            let s2 = &s[pos..];
            let pos = s2.find(|c: char| !c.is_whitespace());
            let pos = match pos {
                None => return Err(Error::MissingSetDelimeterClosingTag),
                Some(pos) => pos,
            };

            self.closing_tag = s2[pos..].to_string();
            self.closing_tag_chars = self.closing_tag.chars().collect();
            Ok(())
        } else {
            Err(Error::InvalidSetDelimeterSyntax)
        }
    }

    /// Pass 2: decide, for every tag, whether it is standalone, looking only
    /// at the original items. A standalone tag is a comment, section,
    /// inverted section, close, partial or delimiter tag that is alone on its
    /// line, apart from blanks:
    ///
    ///   (line start) blank* tag blank* ("\n" | "\r\n" | end of input)
    ///
    /// Returns the whitespace to cut from each text item, and, for each
    /// standalone tag, the length in bytes of its indentation.
    fn analyze(&self) -> (Vec<Cut>, Vec<Option<usize>>) {
        let mut cuts = vec![Cut::default(); self.items.len()];
        let mut indents = vec![None; self.items.len()];

        for (i, item) in self.items.iter().enumerate() {
            let content = match *item {
                Item::Tag { ref content, .. } => content,
                Item::Text(_) => continue,
            };

            if !content.starts_with(&['!', '#', '^', '/', '>', '='][..]) {
                continue;
            }

            if let (Some(indent), Some(trailing)) = (self.indent_before(i), self.trailing_after(i)) {
                indents[i] = Some(indent);
                if indent > 0 {
                    cuts[i - 1].end = indent;
                }
                if trailing > 0 {
                    cuts[i + 1].start = trailing;
                }
            }
        }

        (cuts, indents)
    }

    /// If only blanks sit between the start of the line and the tag at
    /// `index`, returns how many bytes of them there are.
    fn indent_before(&self, index: usize) -> Option<usize> {
        if index == 0 {
            return Some(0);
        }

        match self.items[index - 1] {
            Item::Tag { .. } => None,
            Item::Text(ref s) => {
                let line_start = match s.rfind('\n') {
                    Some(pos) => pos + 1,
                    // No line break: the line only starts here if this text
                    // is the very beginning of the input.
                    None if index == 1 => 0,
                    None => return None,
                };
                let indent = &s[line_start..];
                if indent.chars().all(is_blank) {
                    Some(indent.len())
                } else {
                    None
                }
            }
        }
    }

    /// If only blanks sit between the tag at `index` and the end of the line,
    /// returns how many bytes (blanks and line break) to remove after the tag.
    fn trailing_after(&self, index: usize) -> Option<usize> {
        match self.items.get(index + 1) {
            None => Some(0),
            Some(&Item::Tag { .. }) => None,
            Some(Item::Text(s)) => {
                let blanks = s.len() - s.trim_start_matches(is_blank).len();
                let rest = &s[blanks..];
                if rest.starts_with('\n') {
                    Some(blanks + 1)
                } else if rest.starts_with("\r\n") {
                    Some(blanks + 2)
                } else if rest.is_empty() && index + 2 == self.items.len() {
                    // Blanks up to the end of the input.
                    Some(blanks)
                } else {
                    None
                }
            }
        }
    }

    /// Pass 3: apply the whitespace decisions and assemble the tokens.
    fn build(&mut self, cuts: &[Cut], indents: &[Option<usize>]) -> Result<(), Error> {
        let items = mem::take(&mut self.items);

        for (i, item) in items.iter().enumerate() {
            match *item {
                Item::Text(ref s) => {
                    let text = &s[cuts[i].start..s.len() - cuts[i].end];
                    if !text.is_empty() {
                        self.tokens.push(Token::Text(text.to_string()));
                    }
                }
                Item::Tag { ref content, ref source } => {
                    let indent = match (indents[i], i.checked_sub(1).map(|prev| &items[prev])) {
                        (Some(len), Some(Item::Text(s))) => &s[s.len() - len..],
                        _ => "",
                    };
                    self.add_tag(content, source.clone(), indent)?;
                }
            }
        }

        Ok(())
    }

    fn add_tag(&mut self, content: &str, tag: String, indent: &str) -> Result<(), Error> {
        let len = content.len();

        match content.as_bytes()[0] as char {
            '!' | '=' => {
                // Comments are ignored, and delimiter changes were
                // already applied while scanning.
            }
            '&' => {
                let name = &content[1..len];
                let name = get_name_or_implicit(name)?;
                self.tokens.push(Token::UnescapedTag(name, tag));
            }
            '{' => {
                if content.ends_with('}') {
                    let name = &content[1..len - 1];
                    let name = get_name_or_implicit(name)?;
                    self.tokens.push(Token::UnescapedTag(name, tag));
                } else {
                    return Err(Error::UnbalancedUnescapeTag)
                }
            }
            '#' => {
                let name = get_name_or_implicit(&content[1..len])?;
                self.tokens.push(Token::IncompleteSection(name, false, tag));
            }
            '^' => {
                let name = get_name_or_implicit(&content[1..len])?;
                self.tokens.push(Token::IncompleteSection(name, true, tag));
            }
            '/' => {
                let name = get_name_or_implicit(&content[1..len])?;
                let mut children: Vec<Token> = Vec::new();

                loop {
                    if self.tokens.is_empty() {
                        return Err(Error::EarlySectionClose(name.join(".")))
                    }

                    let last = self.tokens.pop();

                    match last {
                        Some(Token::IncompleteSection(section_name, inverted, osection)) => {
                            children.reverse();

                            // Collect all the children's sources.
                            let mut srcs = Vec::new();
                            for child in children.iter() {
                                match *child {
                                    Token::Text(ref s) |
                                    Token::EscapedTag(_, ref s) |
                                    Token::UnescapedTag(_, ref s) |
                                    Token::Partial(_, _, ref s) => srcs.push(s.clone()),
                                    Token::Section(_, _, _, _, ref osection, ref src, ref csection, _) => {
                                        srcs.push(osection.clone());
                                        srcs.push(src.clone());
                                        srcs.push(csection.clone());
                                    }
                                    _ => bug!("Incomplete sections should not be nested"),
                                }
                            }

                            if section_name == name {
                                // Cache the combination of all the sources in the
                                // section. It's unfortunate, but we need to do this in
                                // case the user uses a function to instantiate the
                                // tag.
                                let mut src = String::new();
                                for s in srcs.iter() {
                                    src.push_str(s);
                                }

                                self.tokens.push(Token::Section(name,
                                                         inverted,
                                                         children,
                                                         self.opening_tag.clone(),
                                                         osection,
                                                         src,
                                                         tag,
                                                         self.closing_tag.clone()));
                                break;
                            } else {
                                return Err(Error::UnclosedSection(section_name.join(".")))
                            }
                        }
                        Some(last_token) => children.push(last_token),
                        None => (),
                    }
                }
            }
            '>' => self.add_partial(content, tag, indent)?,
            _ => {
                // If the name is "." then we want the top element, which we represent with
                // an empty name.
                let name = get_name_or_implicit(content)?;
                self.tokens.push(Token::EscapedTag(name, tag));
            }
        };

        Ok(())
    }

    fn add_partial(&mut self, content: &str, tag: String, indent: &str) -> Result<(), Error> {
        // We can't inline the tokens directly as we may have a recursive
        // partial. So instead, we'll cache the partials we used and look them
        // up later.
        let name = &content[1..content.len()];
        let name = deny_blank(name)?;

        self.tokens.push(Token::Partial(name.into(), indent.to_string(), tag));
        self.partials.push(name.into());

        Ok(())
    }

    fn not_otag(&mut self) {
        for (i, ch) in self.opening_tag_chars.iter().enumerate() {
            if i >= self.tag_position {
                break;
            }
            self.content.push(*ch);
        }
    }

    fn not_ctag(&mut self) {
        for (i, ch) in self.closing_tag_chars.iter().enumerate() {
            if i >= self.tag_position {
                break;
            }
            self.content.push(*ch);
        }
    }
}

fn get_name_or_implicit(name: &str) -> Result<Vec<String>, Error> {
    // If the name is "." then we want the top element, which we represent with
    // an empty name.
    let name = deny_blank(name)?;
    Ok(if name == "." {
        Vec::new()
    } else {
        name.split_terminator('.')
            .map(|x| x.to_string())
            .collect()
    })
}

fn deny_blank(content: &str) -> Result<&str, Error> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        Err(Error::EmptyTag)
    } else {
        Ok(trimmed)
    }
}

//FIXME: These tests are mainly to guide the removal of panics from the parser (turning them
// into `Error`s instead). It would be good to make them more robust and make assertions on
// the tokens & partials returned.
#[cfg(test)]
mod tests {
    use super::*;

    pub fn parse(input: &str) -> Result<(Vec<Token>, Vec<String>), Error> {
        let input = &mut input.chars();
        let parser = Parser::new(input, "{{", "}}");
        parser.parse()
    }

    pub fn assert_parse(input: &str) -> (Vec<Token>, Vec<String>) {
        parse(input).unwrap_or_else(|_| panic!("Failed to parse: {}", input))
    }

    #[test]
    fn empty_input() {
        assert_parse("");
    }

    #[test]
    fn empty_tag() {
        assert_eq!(parse("{{}}"), Err(Error::EmptyTag));
    }

    #[test]
    fn whitespace_only_tag() {
        assert_eq!(parse("{{ }}"), Err(Error::EmptyTag));
    }

    #[test]
    fn bad_closing_tag() {
        assert_eq!(parse("{{hello}?"), Err(Error::BadClosingTag('?', '}')))
    }

    #[test]
    fn unclosed_tag() {
        assert_eq!(parse("{{hi"), Err(Error::UnclosedTag))
    }

    mod sections {
        use super::*;

        #[test]
        fn sanity() {
            assert_parse("{{#people}} Hi {{name}}! {{/people}}");
        }

        #[test]
        fn unclosed() {
            assert_eq!(parse("{{#world}}hi"), Err(Error::UnclosedSection("world".into())))
        }

        #[test]
        fn unclosed_nested_with_wrong_closing_tag() {
            assert_eq!(
                parse("{{#universe}} {{#world}} {{/universe}}"),
                Err(Error::UnclosedSection("world".into()))
            )
        }

        #[test]
        fn unclosed_nested() {
            assert_eq!(
                parse("{{#universe}} {{#world}}"),
                Err(Error::UnclosedSection("world".into()))
            )
        }

        #[test]
        fn unclosed_with_path() {
            assert_eq!(
                parse("{{#universe}} {{#world.and.stuff}} {{/universe}}"),
                Err(Error::UnclosedSection("world.and.stuff".into()))
            )
        }

        #[test]
        fn early_close() {
            assert_eq!(parse("{{/world}}"), Err(Error::EarlySectionClose("world".into())))
        }
    }

    mod inverted {
        use super::*;

        #[test]
        fn sanity() {
            assert_parse("{{^people}} No people! {{/people}}");
        }

        #[test]
        fn unclosed() {
            assert_eq!(parse("{{^world}}hi"), Err(Error::UnclosedSection("world".into())))
        }

        #[test]
        fn unclosed_nested_with_wrong_closing_tag() {
            assert_eq!(
                parse("{{#universe}} {{^world}} {{/universe}}"),
                Err(Error::UnclosedSection("world".into()))
            );

            assert_eq!(
                parse("{{^universe}} {{^world}} {{/universe}}"),
                Err(Error::UnclosedSection("world".into()))
            )
        }

        #[test]
        fn unclosed_nested() {
            assert_eq!(
                parse("{{#universe}} {{^world}}"),
                Err(Error::UnclosedSection("world".into()))
            )
        }

        #[test]
        fn unclosed_with_path() {
            assert_eq!(
                parse("{{#universe}} {{^world.and.stuff}} {{/universe}}"),
                Err(Error::UnclosedSection("world.and.stuff".into()))
            )
        }
    }

    mod set_delimeter {
        use super::*;

        #[test]
        fn sanity() {
            assert_parse("{{=<% %>=}}");
        }

        #[test]
        fn closing_tag_is_whitespace() {
            assert_eq!(parse("{{=<% =}}"), Err(Error::MissingSetDelimeterClosingTag))
        }

        #[test]
        fn missing_closing_tag() {
            assert_eq!(parse("{{=<%=}}"), Err(Error::MissingSetDelimeterClosingTag))
        }

        #[test]
        fn missing_closing_equals() {
            assert_eq!(parse("{{=<% %>}}"), Err(Error::InvalidSetDelimeterSyntax))
        }
    }

    #[test]
    fn unbalanced_unescape() {
        // use the set delimiter tag to change the brace type. Currently this error will
        // not trigger with "{{{ }}"
        let input = "{{=<% %>=}} <%{ %>";
        assert_eq!(parse(input), Err(Error::UnbalancedUnescapeTag))
    }
}
