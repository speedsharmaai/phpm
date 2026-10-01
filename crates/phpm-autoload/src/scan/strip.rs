//! `php_strip_whitespace()`: PHP's own lexer with comments dropped and
//! whitespace runs collapsed to one space. Only the token boundaries that
//! change the output are reproduced; everything else is copied through.

// php-src: Zend/zend_language_scanner.l, Zend/zend_highlight.c zend_strip

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Initial,
    Scripting,
    LookingForProperty,
    DoubleQuotes,
    Backquote,
    Heredoc,
    Nowdoc,
    EndHeredoc,
    LookingForVarname,
    VarOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Whitespace,
    Comment,
    EndHeredoc,
    Text,
    End,
}

#[derive(Debug)]
struct Label {
    label: Vec<u8>,
    indentation: usize,
}

fn is_label_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_label_char(c: u8) -> bool {
    is_label_start(c) || c.is_ascii_digit()
}

fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

const OPERATORS: [&[u8]; 34] = [
    b"...", b"**=", b"<=>", b"===", b"!==", b"<<=", b">>=", b"??=", b"?->", b"**", b"==", b"!=",
    b"<>", b"<=", b">=", b"+=", b"-=", b"*=", b"/=", b".=", b"%=", b"&=", b"|=", b"^=", b"||",
    b"&&", b"<<", b">>", b"++", b"--", b"->", b"=>", b"::", b"??",
];

const CASTS: [&[u8]; 12] = [
    b"int", b"integer", b"double", b"float", b"real", b"string", b"binary", b"array", b"object",
    b"bool", b"boolean", b"unset",
];

struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
    state: State,
    stack: Vec<State>,
    heredocs: Vec<Label>,
    short_tags: bool,
}

impl Lexer<'_> {
    fn at(&self, i: usize) -> u8 {
        self.s.get(i).copied().unwrap_or(0)
    }

    fn starts_with_ci(&self, i: usize, word: &[u8]) -> bool {
        self.s
            .get(i..i + word.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(word))
    }

    fn label_end(&self, i: usize) -> usize {
        if !is_label_start(self.at(i)) || i >= self.s.len() {
            return i;
        }
        let mut j = i + 1;
        while j < self.s.len() && is_label_char(self.s[j]) {
            j += 1;
        }
        j
    }

    fn pop(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }

    fn push(&mut self, state: State) {
        self.stack.push(self.state);
        self.state = state;
    }

    fn token(&mut self, kind: Kind, end: usize) -> (Kind, usize, usize) {
        let start = self.pos;
        self.pos = end;
        (kind, start, end)
    }

    fn next(&mut self) -> (Kind, usize, usize) {
        loop {
            if self.pos >= self.s.len() && self.state != State::EndHeredoc {
                return (Kind::End, self.pos, self.pos);
            }
            let result = match self.state {
                State::Initial => Some(self.initial()),
                State::Scripting => Some(self.scripting()),
                State::LookingForProperty => self.looking_for_property(),
                State::DoubleQuotes | State::Backquote | State::Heredoc => {
                    Some(self.interpolated())
                }
                State::Nowdoc => Some(self.nowdoc()),
                State::EndHeredoc => Some(self.end_heredoc()),
                State::LookingForVarname => self.looking_for_varname(),
                State::VarOffset => Some(self.var_offset()),
            };
            if let Some(token) = result {
                return token;
            }
        }
    }

    fn open_tag_at(&self, i: usize) -> bool {
        self.at(i + 1) == b'?'
            && (self.short_tags
                || self.at(i + 2) == b'='
                || (self.starts_with_ci(i + 2, b"php")
                    && (i + 5 == self.s.len()
                        || matches!(self.at(i + 5), b' ' | b'\t' | b'\n' | b'\r'))))
    }

    fn initial(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        if self.at(p) == b'<' && self.at(p + 1) == b'?' {
            if self.at(p + 2) == b'=' {
                self.state = State::Scripting;
                return self.token(Kind::Text, p + 3);
            }
            if self.starts_with_ci(p + 2, b"php") {
                let after = p + 5;
                let end = match self.at(after) {
                    b' ' | b'\t' | b'\n' => Some(after + 1),
                    b'\r' => Some(if self.at(after + 1) == b'\n' {
                        after + 2
                    } else {
                        after + 1
                    }),
                    _ if after == self.s.len() => Some(after),
                    _ => None,
                };
                if let Some(end) = end {
                    self.state = State::Scripting;
                    return self.token(Kind::Text, end);
                }
                if self.short_tags {
                    self.state = State::Scripting;
                    return self.token(Kind::Text, p + 2);
                }
            } else if self.short_tags {
                self.state = State::Scripting;
                return self.token(Kind::Text, p + 2);
            }
        }
        let mut i = p + 1;
        while i < self.s.len() {
            match self.s[i..].iter().position(|&c| c == b'<') {
                None => {
                    i = self.s.len();
                    break;
                }
                Some(off) => {
                    let lt = i + off;
                    if self.open_tag_at(lt) {
                        i = lt;
                        break;
                    }
                    i = lt + 1;
                }
            }
        }
        self.token(Kind::Text, i.min(self.s.len()))
    }

    fn line_comment_end(&self, mut i: usize) -> usize {
        while i < self.s.len() {
            match self.s[i] {
                b'\r' | b'\n' => return i,
                b'?' if self.at(i + 1) == b'>' => return i,
                _ => i += 1,
            }
        }
        i
    }

    fn block_comment_end(&self, mut i: usize) -> usize {
        while i < self.s.len() {
            let c = self.s[i];
            i += 1;
            if c == b'*' && self.at(i) == b'/' {
                return (i + 1).min(self.s.len());
            }
        }
        self.s.len()
    }

    /// `/*` comment, `/**` + whitespace doc comment, `#` or `//` comment.
    fn comment_at(&self, p: usize) -> Option<usize> {
        match (self.at(p), self.at(p + 1)) {
            (b'#', _) => Some(self.line_comment_end(p + 1)),
            (b'/', b'/') => Some(self.line_comment_end(p + 2)),
            (b'/', b'*') => {
                let start = if self.at(p + 2) == b'*' && is_whitespace(self.at(p + 3)) {
                    p + 4
                } else {
                    p + 2
                };
                Some(self.block_comment_end(start))
            }
            _ => None,
        }
    }

    fn whitespace_end(&self, mut i: usize) -> usize {
        while i < self.s.len() && is_whitespace(self.s[i]) {
            i += 1;
        }
        i
    }

    /// `{WHITESPACE_OR_COMMENTS}+` with the strict comment forms of the
    /// `yield from` rule.
    fn strict_gap_end(&self, start: usize) -> Option<usize> {
        let mut i = start;
        loop {
            let c = self.at(i);
            if i < self.s.len() && is_whitespace(c) {
                i = self.whitespace_end(i);
            } else if c == b'/' && self.at(i + 1) == b'*' {
                let mut j = i + 2;
                loop {
                    if j >= self.s.len() || self.s[j] == 0 {
                        return (i > start).then_some(i);
                    }
                    if self.s[j] == b'*' && self.at(j + 1) == b'/' {
                        i = j + 2;
                        break;
                    }
                    j += 1;
                }
            } else if (c == b'/' && self.at(i + 1) == b'/') || (c == b'#' && self.at(i + 1) != b'[')
            {
                let mut j = i + if c == b'#' { 1 } else { 2 };
                while j < self.s.len() && !matches!(self.s[j], b'\n' | b'\r' | 0) {
                    j += 1;
                }
                if !matches!(self.at(j), b'\n' | b'\r') || j >= self.s.len() {
                    return (i > start).then_some(i);
                }
                i = j + 1;
            } else {
                return (i > start).then_some(i);
            }
        }
    }

    fn cast_end(&self, p: usize) -> Option<usize> {
        let skip = |mut i: usize| {
            while matches!(self.at(i), b' ' | b'\t') && i < self.s.len() {
                i += 1;
            }
            i
        };
        let i = skip(p + 1);
        let word = self.label_end(i);
        let name = self.s.get(i..word)?;
        if !CASTS.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            return None;
        }
        let close = skip(word);
        (self.at(close) == b')' && close < self.s.len()).then_some(close + 1)
    }

    /// `<<<` start: (token end, label, nowdoc).
    fn heredoc_start(&self, p: usize) -> Option<(usize, Vec<u8>, bool)> {
        if !self.s[p..].starts_with(b"<<<") {
            return None;
        }
        let mut i = p + 3;
        while matches!(self.at(i), b' ' | b'\t') && i < self.s.len() {
            i += 1;
        }
        let quote = match self.at(i) {
            q @ (b'\'' | b'"') => {
                i += 1;
                Some(q)
            }
            _ => None,
        };
        let end = self.label_end(i);
        if end == i {
            return None;
        }
        let label = self.s[i..end].to_vec();
        let mut j = end;
        if let Some(q) = quote {
            if self.at(j) != q || j >= self.s.len() {
                return None;
            }
            j += 1;
        }
        let j = match self.at(j) {
            b'\n' if j < self.s.len() => j + 1,
            b'\r' if j < self.s.len() => {
                if self.at(j + 1) == b'\n' {
                    j + 2
                } else {
                    j + 1
                }
            }
            _ => return None,
        };
        Some((j, label, quote == Some(b'\'')))
    }

    /// Whether `label` closes the heredoc at `i`, after the indentation.
    fn closes_at(&self, i: usize, label: &[u8]) -> bool {
        label.len() < self.s.len().saturating_sub(i)
            && self.s[i..].starts_with(label)
            && !is_label_char(self.at(i + label.len()))
    }

    fn indentation_end(&self, mut i: usize) -> usize {
        while i < self.s.len() && matches!(self.s[i], b' ' | b'\t') {
            i += 1;
        }
        i
    }

    fn scripting(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        let c = self.s[p];
        let next = self.at(p + 1);
        if is_whitespace(c) {
            let end = self.whitespace_end(p);
            return self.token(Kind::Whitespace, end);
        }
        if c == b'#' && next == b'[' {
            return self.token(Kind::Text, p + 2);
        }
        if let Some(end) = self.comment_at(p) {
            return self.token(Kind::Comment, end);
        }
        match c {
            b'?' if next == b'>' => {
                let mut end = p + 2;
                if self.at(end) == b'\r' && end < self.s.len() {
                    end += 1;
                    if self.at(end) == b'\n' {
                        end += 1;
                    }
                } else if self.at(end) == b'\n' && end < self.s.len() {
                    end += 1;
                }
                self.state = State::Initial;
                self.token(Kind::Text, end)
            }
            b'?' if next == b'-' && self.at(p + 2) == b'>' => {
                self.push(State::LookingForProperty);
                self.token(Kind::Text, p + 3)
            }
            b'?' if next == b'?' => {
                let end = if self.at(p + 2) == b'=' { p + 3 } else { p + 2 };
                self.token(Kind::Text, end)
            }
            b'-' if next == b'>' => {
                self.push(State::LookingForProperty);
                self.token(Kind::Text, p + 2)
            }
            b'\'' => {
                let mut i = p + 1;
                while i < self.s.len() {
                    match self.s[i] {
                        b'\'' => {
                            i += 1;
                            return self.token(Kind::Text, i);
                        }
                        b'\\' if i + 1 < self.s.len() => i += 2,
                        _ => i += 1,
                    }
                }
                self.token(Kind::Text, self.s.len())
            }
            b'"' => {
                let mut i = p + 1;
                while i < self.s.len() {
                    match self.s[i] {
                        b'"' => return self.token(Kind::Text, i + 1),
                        b'$' if is_label_start(self.at(i + 1)) && i + 1 < self.s.len()
                            || self.at(i + 1) == b'{' =>
                        {
                            break;
                        }
                        b'{' if self.at(i + 1) == b'$' => break,
                        b'\\' => i += 2,
                        _ => i += 1,
                    }
                }
                self.state = State::DoubleQuotes;
                self.token(Kind::Text, p + 1)
            }
            b'`' => {
                self.state = State::Backquote;
                self.token(Kind::Text, p + 1)
            }
            b'<' => {
                if let Some((end, label, nowdoc)) = self.heredoc_start(p) {
                    self.state = if nowdoc {
                        State::Nowdoc
                    } else {
                        State::Heredoc
                    };
                    let indent_end = self.indentation_end(end);
                    let mut heredoc = Label {
                        label,
                        indentation: 0,
                    };
                    if indent_end < self.s.len() && self.closes_at(indent_end, &heredoc.label) {
                        heredoc.indentation = indent_end - end;
                        self.state = State::EndHeredoc;
                    }
                    self.heredocs.push(heredoc);
                    return self.token(Kind::Text, end);
                }
                let end = match (next, self.at(p + 2)) {
                    (b'<', b'=') => p + 3,
                    (b'<', _) => p + 2,
                    _ => p + 1,
                };
                self.token(Kind::Text, end.min(self.s.len()))
            }
            b'-' if next == b'-' => self.token(Kind::Text, p + 2),
            b'{' => {
                self.push(State::Scripting);
                self.token(Kind::Text, p + 1)
            }
            b'}' => {
                self.pop();
                self.token(Kind::Text, p + 1)
            }
            b'(' => {
                let end = self.cast_end(p).unwrap_or(p + 1);
                self.token(Kind::Text, end)
            }
            b'\\' if is_label_start(next) && p + 1 < self.s.len() => {
                let end = self.name_end(p + 1);
                self.token(Kind::Text, end)
            }
            b'$' if is_label_start(next) && p + 1 < self.s.len() => {
                let end = self.label_end(p + 1);
                self.token(Kind::Text, end)
            }
            c if is_label_start(c) => {
                let end = self.label_end(p);
                if self.s[p..end].eq_ignore_ascii_case(b"yield")
                    && let Some(gap) = self.strict_gap_end(end)
                    && self.starts_with_ci(gap, b"from")
                    && gap + 4 < self.s.len()
                    && !is_label_char(self.s[gap + 4])
                {
                    return self.token(Kind::Text, gap + 4);
                }
                let end = self.name_end(p);
                self.token(Kind::Text, end)
            }
            _ => {
                let end = self
                    .number_end(p)
                    .or_else(|| {
                        OPERATORS
                            .iter()
                            .find(|op| self.s[p..].starts_with(op))
                            .map(|op| p + op.len())
                    })
                    .unwrap_or(p + 1);
                self.token(Kind::Text, end)
            }
        }
    }

    /// A label with any `\label` parts that follow it.
    fn name_end(&self, p: usize) -> usize {
        let mut end = self.label_end(p);
        while self.at(end) == b'\\' && is_label_start(self.at(end + 1)) && end + 1 < self.s.len() {
            end = self.label_end(end + 1);
        }
        end
    }

    fn digits_end(&self, i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if i >= self.s.len() || !digit(self.s[i]) {
            return None;
        }
        let mut j = i + 1;
        loop {
            while j < self.s.len() && digit(self.s[j]) {
                j += 1;
            }
            if self.at(j) == b'_' && j + 1 < self.s.len() && digit(self.s[j + 1]) {
                j += 1;
            } else {
                return Some(j);
            }
        }
    }

    /// The longest number token (`LNUM`, `DNUM`, `EXPONENT_DNUM`, `HNUM`,
    /// `BNUM`, `ONUM`) at `p`.
    fn number_end(&self, p: usize) -> Option<usize> {
        let dec = |c: u8| c.is_ascii_digit();
        let mut best: Option<usize> = None;
        let mut take = |end: Option<usize>| {
            if let Some(e) = end {
                best = Some(best.map_or(e, |b: usize| b.max(e)));
            }
        };
        let int = self.digits_end(p, dec);
        take(int);
        let after_int = int.unwrap_or(p);
        let dnum = if self.at(after_int) == b'.' && after_int < self.s.len() {
            match (int, self.digits_end(after_int + 1, dec)) {
                (_, Some(frac)) => Some(frac),
                (Some(_), None) => Some(after_int + 1),
                (None, None) => None,
            }
        } else {
            None
        };
        take(dnum);
        for mantissa in [int, dnum].into_iter().flatten() {
            if matches!(self.at(mantissa), b'e' | b'E') && mantissa < self.s.len() {
                let sign = usize::from(matches!(self.at(mantissa + 1), b'+' | b'-'));
                take(self.digits_end(mantissa + 1 + sign, dec));
            }
        }
        if self.at(p) == b'0' {
            let radix: Option<fn(u8) -> bool> = match self.at(p + 1) {
                b'x' | b'X' => Some(|c: u8| c.is_ascii_hexdigit()),
                b'b' | b'B' => Some(|c: u8| c == b'0' || c == b'1'),
                b'o' | b'O' => Some(|c: u8| (b'0'..=b'7').contains(&c)),
                _ => None,
            };
            if let Some(radix) = radix {
                take(self.digits_end(p + 2, radix));
            }
        }
        best
    }

    fn looking_for_property(&mut self) -> Option<(Kind, usize, usize)> {
        let p = self.pos;
        let c = self.s[p];
        if is_whitespace(c) {
            let end = self.whitespace_end(p);
            return Some(self.token(Kind::Whitespace, end));
        }
        if c == b'-' && self.at(p + 1) == b'>' {
            return Some(self.token(Kind::Text, p + 2));
        }
        if c == b'?' && self.at(p + 1) == b'-' && self.at(p + 2) == b'>' {
            return Some(self.token(Kind::Text, p + 3));
        }
        if is_label_start(c) {
            let end = self.label_end(p);
            self.pop();
            return Some(self.token(Kind::Text, end));
        }
        if let Some(end) = self.comment_at(p) {
            return Some(self.token(Kind::Comment, end));
        }
        self.pop();
        None
    }

    fn looking_for_varname(&mut self) -> Option<(Kind, usize, usize)> {
        let p = self.pos;
        let end = self.label_end(p);
        self.pop();
        self.push(State::Scripting);
        if end > p && matches!(self.at(end), b'[' | b'}') && end < self.s.len() {
            return Some(self.token(Kind::Text, end));
        }
        None
    }

    fn var_offset(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        match self.s[p] {
            b']' => {
                self.pop();
                self.token(Kind::Text, p + 1)
            }
            b' ' | b'\n' | b'\r' | b'\t' | b'\\' | b'\'' | b'#' => {
                self.pop();
                self.token(Kind::Text, p)
            }
            b'$' if is_label_start(self.at(p + 1)) && p + 1 < self.s.len() => {
                let end = self.label_end(p + 1);
                self.token(Kind::Text, end)
            }
            c if is_label_start(c) => {
                let end = self.label_end(p);
                self.token(Kind::Text, end)
            }
            _ => self.token(Kind::Text, p + 1),
        }
    }

    /// Double quotes, backquotes and heredoc bodies.
    fn interpolated(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        let c = self.s[p];
        let next = self.at(p + 1);
        if c == b'{' && next == b'$' {
            self.push(State::Scripting);
            return self.token(Kind::Text, p + 1);
        }
        if c == b'$' && next == b'{' {
            self.push(State::LookingForVarname);
            return self.token(Kind::Text, p + 2);
        }
        if c == b'$' && is_label_start(next) && p + 1 < self.s.len() {
            let end = self.label_end(p + 1);
            let arrow = usize::from(self.at(end) == b'?');
            if self.at(end + arrow) == b'-'
                && self.at(end + arrow + 1) == b'>'
                && is_label_start(self.at(end + arrow + 2))
                && end + arrow + 2 < self.s.len()
            {
                self.push(State::LookingForProperty);
            } else if self.at(end) == b'[' && end < self.s.len() {
                self.push(State::VarOffset);
            }
            return self.token(Kind::Text, end);
        }
        match self.state {
            State::DoubleQuotes if c == b'"' => {
                self.state = State::Scripting;
                self.token(Kind::Text, p + 1)
            }
            State::Backquote if c == b'`' => {
                self.state = State::Scripting;
                self.token(Kind::Text, p + 1)
            }
            State::Heredoc => self.heredoc_body(),
            _ => {
                let close = if self.state == State::DoubleQuotes {
                    b'"'
                } else {
                    b'`'
                };
                let mut i = p + 1;
                if c == b'\\' && i < self.s.len() {
                    i += 1;
                }
                while i < self.s.len() {
                    let ch = self.s[i];
                    i += 1;
                    match ch {
                        _ if ch == close => {
                            i -= 1;
                            break;
                        }
                        b'$' if is_label_start(self.at(i)) && i < self.s.len()
                            || self.at(i) == b'{' =>
                        {
                            i -= 1;
                            break;
                        }
                        b'{' if self.at(i) == b'$' => {
                            i -= 1;
                            break;
                        }
                        b'\\' if i < self.s.len() => i += 1,
                        _ => {}
                    }
                }
                self.token(Kind::Text, i.min(self.s.len()))
            }
        }
    }

    fn heredoc_body(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        let label = self
            .heredocs
            .last()
            .map(|h| h.label.clone())
            .unwrap_or_default();
        let mut i = p;
        while i < self.s.len() {
            let ch = self.s[i];
            i += 1;
            match ch {
                b'\r' | b'\n' => {
                    if ch == b'\r' && self.at(i) == b'\n' {
                        i += 1;
                    }
                    let indent_end = self.indentation_end(i);
                    if indent_end >= self.s.len() {
                        return self.token(Kind::Text, self.s.len());
                    }
                    if is_label_start(self.at(indent_end)) && self.closes_at(indent_end, &label) {
                        if let Some(h) = self.heredocs.last_mut() {
                            h.indentation = indent_end - i;
                        }
                        self.state = State::EndHeredoc;
                        return self.token(Kind::Text, i);
                    }
                    i = indent_end;
                }
                b'$' if (is_label_start(self.at(i)) && i < self.s.len()) || self.at(i) == b'{' => {
                    return self.token(Kind::Text, i - 1);
                }
                b'{' if self.at(i) == b'$' => return self.token(Kind::Text, i - 1),
                b'\\' if i < self.s.len() && !matches!(self.s[i], b'\n' | b'\r') => i += 1,
                _ => {}
            }
        }
        self.token(Kind::Text, self.s.len())
    }

    fn nowdoc(&mut self) -> (Kind, usize, usize) {
        let p = self.pos;
        let label = self
            .heredocs
            .last()
            .map(|h| h.label.clone())
            .unwrap_or_default();
        let mut i = p;
        while i < self.s.len() {
            let ch = self.s[i];
            i += 1;
            if matches!(ch, b'\r' | b'\n') {
                if ch == b'\r' && self.at(i) == b'\n' {
                    i += 1;
                }
                let indent_end = self.indentation_end(i);
                if indent_end >= self.s.len() {
                    return self.token(Kind::Text, self.s.len());
                }
                if is_label_start(self.at(indent_end)) && self.closes_at(indent_end, &label) {
                    if let Some(h) = self.heredocs.last_mut() {
                        h.indentation = indent_end - i;
                    }
                    self.state = State::EndHeredoc;
                    return self.token(Kind::Text, i);
                }
                i = indent_end;
            }
        }
        self.token(Kind::Text, self.s.len())
    }

    fn end_heredoc(&mut self) -> (Kind, usize, usize) {
        let heredoc = self.heredocs.pop();
        let len = heredoc.map_or(0, |h| h.indentation + h.label.len());
        self.state = State::Scripting;
        let end = (self.pos + len).min(self.s.len());
        self.token(Kind::EndHeredoc, end)
    }
}

fn skip_shebang(s: &[u8]) -> usize {
    if !s.starts_with(b"#!") {
        return 0;
    }
    if let Some(n) = s.iter().position(|&c| c == b'\n') {
        return n + 1;
    }
    s.iter().rposition(|&c| c == b'\r').map_or(0, |r| r + 1)
}

/// `php_strip_whitespace()` as the PHP CLI running Composer computes it:
/// a leading `#!` line skipped, short open tags as configured.
pub fn strip_whitespace(source: &[u8], short_tags: bool) -> Vec<u8> {
    let mut lexer = Lexer {
        s: source,
        pos: skip_shebang(source),
        state: State::Initial,
        stack: Vec::new(),
        heredocs: Vec::new(),
        short_tags,
    };
    let mut out = Vec::with_capacity(source.len());
    let mut prev_space = false;
    loop {
        let (kind, start, end) = lexer.next();
        match kind {
            Kind::End => break,
            Kind::Whitespace => {
                if !prev_space {
                    out.push(b' ');
                    prev_space = true;
                }
            }
            Kind::Comment => {}
            Kind::EndHeredoc => {
                out.extend_from_slice(&source[start..end]);
                let (next, s2, e2) = lexer.next();
                if next != Kind::Whitespace {
                    out.extend_from_slice(&source[s2..e2]);
                }
                out.push(b'\n');
                prev_space = true;
            }
            Kind::Text => {
                out.extend_from_slice(&source[start..end]);
                prev_space = false;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_whitespace;

    fn strip(s: &str) -> String {
        String::from_utf8(strip_whitespace(s.as_bytes(), false)).unwrap()
    }

    #[test]
    fn drops_comments_and_collapses_whitespace() {
        assert_eq!(strip("<?php  // c\nclass  A {}\n"), "<?php  class A {} ");
        assert_eq!(
            strip("<?php\n/** doc */\n# hash\nclass A{}"),
            "<?php\n class A{}"
        );
        assert_eq!(strip("<?php /* unterminated"), "<?php ");
        assert_eq!(strip("<?php #[Attr]\nclass A{}"), "<?php #[Attr] class A{}");
        assert_eq!(strip("#!/usr/bin/env php\n<?php echo 1;"), "<?php echo 1;");
    }

    #[test]
    fn keeps_strings_and_inline_html() {
        assert_eq!(
            strip("<html><?php $a = '/* x */';?>\n<b>"),
            "<html><?php $a = '/* x */';?>\n<b>"
        );
        assert_eq!(
            strip("<?php $a = \"// $b /* {$c /* x */ } \";"),
            "<?php $a = \"// $b /* {$c } \";"
        );
        assert_eq!(strip("<?php // a ?> b"), "<?php ?> b");
        assert_eq!(strip("<? class A {}"), "<? class A {}");
        assert_eq!(strip("<?= $x ?>"), "<?= $x ?>");
    }

    #[test]
    fn heredocs_follow_the_strip_rules() {
        assert_eq!(
            strip("<?php $a = <<<EOT\n  /* x */ {$b}\n  EOT;\nclass A{}"),
            "<?php $a = <<<EOT\n  /* x */ {$b}\n  EOT;\nclass A{}"
        );
        assert_eq!(
            strip("<?php $a = <<<'EOT'\nclass X\nEOT\n;"),
            "<?php $a = <<<'EOT'\nclass X\nEOT\n;"
        );
        assert_eq!(strip("<?php f(<<<A\nA, 1);"), "<?php f(<<<A\nA,\n1);");
    }

    #[test]
    fn odd_tokens_are_kept_whole() {
        assert_eq!(strip("<?php $x = (  int  )$y;"), "<?php $x = (  int  )$y;");
        assert_eq!(
            strip("<?php yield /* c */ from $g;"),
            "<?php yield /* c */ from $g;"
        );
        assert_eq!(strip("<?php $a ??= $b ?? $c;"), "<?php $a ??= $b ?? $c;");
        assert_eq!(strip("<?php $o -> /* c */ p;"), "<?php $o -> p;");
        assert_eq!(
            strip("<?php \"${a}\" . \"$a[0] $b->c\";"),
            "<?php \"${a}\" . \"$a[0] $b->c\";"
        );
    }

    #[test]
    fn matches_php_on_rarer_tokens() {
        let cases = [
            (
                "<?php `ls {$a} $b->c ${d} \\\\` ;",
                "<?php `ls {$a} $b->c ${d} \\\\` ;",
            ),
            (
                "<?php \"$a[ x] $a[$b] $a[1]\" ;",
                "<?php \"$a[ x] $a[$b] $a[1]\" ;",
            ),
            (
                "<?php \"${ a } ${a[1]} {$a  ->  b}\";",
                "<?php \"${ a } ${a[1]} {$a -> b}\";",
            ),
            (
                "<?php $x = 0x1F + 0b101 + 0o17 + 1_000 + 1.5e3 + .5 + 1. + 1e-3;",
                "<?php $x = 0x1F + 0b101 + 0o17 + 1_000 + 1.5e3 + .5 + 1. + 1e-3;",
            ),
            (
                "<?php $x = ( object )$y . (float)$z . ( foo )$w;",
                "<?php $x = ( object )$y . (float)$z . ( foo )$w;",
            ),
            ("<?php $s = 'unterminated", "<?php $s = 'unterminated"),
            (
                "<?php $s = \"unterminated $x",
                "<?php $s = \"unterminated $x",
            ),
            ("<?php $a = <<<EOT\nno end", "<?php $a = <<<EOT\nno end"),
            ("<?php $a = <<<'EOT'\nno end", "<?php $a = <<<'EOT'\nno end"),
            (
                "<?php $a = <<<\"EOT\"\r\n  x \\$y {$z}\r\n  EOT;",
                "<?php $a = <<<\"EOT\"\r\n  x \\$y {$z}\r\n  EOT;\n",
            ),
            (
                "<?php $a = <<<EOT\nEOTX\nEOT;",
                "<?php $a = <<<EOT\nEOTX\nEOT;\n",
            ),
            (
                "<?php yield\nfrom\n$x; yield fromage;",
                "<?php yield\nfrom $x; yield fromage;",
            ),
            (
                "<?php $a->b->c?->d; $a-> #x\n b;",
                "<?php $a->b->c?->d; $a-> b;",
            ),
            (
                "<?php \\Foo\\Bar::baz(); namespace\\x();",
                "<?php \\Foo\\Bar::baz(); namespace\\x();",
            ),
            (
                "<?php $a = 1 <=> 2 ** 3 .= 4 ?? 5;",
                "<?php $a = 1 <=> 2 ** 3 .= 4 ?? 5;",
            ),
            (
                "<?php { { } } } ?>tail<?php }",
                "<?php { { } } } ?>tail<?php }",
            ),
            (
                "<?php \"{$a['x']}\" . \"$a->b->c\" . \"$a?->b\";",
                "<?php \"{$a['x']}\" . \"$a->b->c\" . \"$a?->b\";",
            ),
            (
                "<?php echo <<<A\n  {$x}\n  A . <<<B\nB;",
                "<?php echo <<<A\n  {$x}\n  A\n. <<<B\nB;\n",
            ),
            ("<?php # comment ?>html", "<?php ?>html"),
            ("<?php /** doc */ /**x*/", "<?php  "),
        ];
        for (source, expected) in cases {
            assert_eq!(strip(source), expected, "{source:?}");
        }
    }
}
