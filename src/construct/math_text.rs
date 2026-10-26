//! Math (text) as pandoc reads it.
//!
//! This follows commonmark-hs `Commonmark.Extensions.Math` (`parseMath`,
//! `pDollarsMath`), the reader pandoc runs for CommonMark and GFM with
//! `tex_math_dollars`:
//!
//! * `$$` opens display math, closed by the next `$$` outside braces; a single
//!   `$` inside it is content
//! * `$` opens inline math unless whitespace follows it, closed by the next `$`
//!   outside braces; it fails when its content ends with whitespace or a digit
//!   follows the closer (`$20,000 and $30,000` is not math)
//! * a backslash takes the next byte; braces nest, at most 1000 deep, and a
//!   closing brace outside any fails
//! * a failed opener is one `$` of text: the next `$` may open (`$$x$` is `$`
//!   and the math `x`)
//!
//! It replaces the micromark rules of [raw (text)][raw_text] for `$`, which
//! have none of these. Twin: Glyph `web/src/syntax/glyph/math.ts`.
//!
//! [raw_text]: crate::construct::raw_text

use crate::event::Name;
use crate::state::{Name as StateName, State};
use crate::tokenizer::Tokenizer;

const MAX_DEPTH: usize = 1000;

/// Start of math (text).
///
/// ```markdown
/// > | $a$
///     ^
/// ```
pub fn start(tokenizer: &mut Tokenizer) -> State {
    tokenizer.enter(Name::MathText);
    tokenizer.enter(Name::MathTextSequence);
    tokenizer.consume();
    State::Next(StateName::MathTextOpened)
}

/// After the first `$`.
///
/// ```markdown
/// > | $a$
///      ^
/// > | $$a$$
///      ^
/// ```
pub fn opened(tokenizer: &mut Tokenizer) -> State {
    if tokenizer.current == Some(b'$') {
        tokenizer.tokenize_state.size = 2;
        tokenizer.consume();
        tokenizer.exit(Name::MathTextSequence);
        return State::Next(StateName::MathTextBetween);
    }
    if !tokenizer.parse_state.options.math_text_single_dollar
        || matches!(tokenizer.current, None | Some(b'\t' | b'\n' | b' '))
    {
        return nok(tokenizer);
    }
    tokenizer.tokenize_state.size = 1;
    tokenizer.exit(Name::MathTextSequence);
    State::Retry(StateName::MathTextBetween)
}

/// Between something and something else.
///
/// ```markdown
/// > | $a$
///      ^^
/// ```
pub fn between(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        None => nok(tokenizer),
        Some(b'\n') => {
            tokenizer.enter(Name::LineEnding);
            tokenizer.consume();
            tokenizer.exit(Name::LineEnding);
            tokenizer.tokenize_state.seen = true;
            State::Next(StateName::MathTextBetween)
        }
        Some(b'$') if tokenizer.tokenize_state.size_b == 0 => {
            tokenizer.enter(Name::MathTextSequence);
            tokenizer.consume();
            if tokenizer.tokenize_state.size == 2 {
                State::Next(StateName::MathTextCloseDisplay)
            } else {
                State::Next(StateName::MathTextCloseInline)
            }
        }
        _ => {
            tokenizer.enter(Name::MathTextData);
            State::Retry(StateName::MathTextData)
        }
    }
}

/// In data.
///
/// ```markdown
/// > | $a$
///      ^
/// ```
pub fn data(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        None | Some(b'\n') => {
            tokenizer.exit(Name::MathTextData);
            State::Retry(StateName::MathTextBetween)
        }
        Some(b'$') if tokenizer.tokenize_state.size_b == 0 => {
            tokenizer.exit(Name::MathTextData);
            State::Retry(StateName::MathTextBetween)
        }
        Some(b'\\') => {
            tokenizer.tokenize_state.seen = false;
            tokenizer.consume();
            State::Next(StateName::MathTextEscape)
        }
        Some(b'{') => {
            tokenizer.tokenize_state.size_b += 1;
            if tokenizer.tokenize_state.size_b > MAX_DEPTH {
                return nok(tokenizer);
            }
            tokenizer.tokenize_state.seen = false;
            tokenizer.consume();
            State::Next(StateName::MathTextData)
        }
        Some(b'}') => {
            if tokenizer.tokenize_state.size_b == 0 {
                return nok(tokenizer);
            }
            tokenizer.tokenize_state.size_b -= 1;
            tokenizer.tokenize_state.seen = false;
            tokenizer.consume();
            State::Next(StateName::MathTextData)
        }
        Some(byte) => {
            tokenizer.tokenize_state.seen = matches!(byte, b'\t' | b' ');
            tokenizer.consume();
            State::Next(StateName::MathTextData)
        }
    }
}

/// After a backslash: the next byte is content.
///
/// ```markdown
/// > | $a\$b$
///        ^
/// ```
pub fn escape(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        None | Some(b'\n') => State::Retry(StateName::MathTextData),
        Some(byte) => {
            tokenizer.tokenize_state.seen = matches!(byte, b'\t' | b' ');
            tokenizer.consume();
            State::Next(StateName::MathTextData)
        }
    }
}

/// After the `$` closing inline math.
///
/// ```markdown
/// > | $a$
///        ^
/// ```
pub fn close_inline(tokenizer: &mut Tokenizer) -> State {
    if tokenizer.tokenize_state.seen || matches!(tokenizer.current, Some(b'0'..=b'9')) {
        return nok(tokenizer);
    }
    tokenizer.exit(Name::MathTextSequence);
    tokenizer.exit(Name::MathText);
    reset(tokenizer);
    State::Ok
}

/// After a `$` in display math: a second one closes it, else it is content.
///
/// ```markdown
/// > | $$a$$
///         ^
/// ```
pub fn close_display(tokenizer: &mut Tokenizer) -> State {
    if tokenizer.current == Some(b'$') {
        tokenizer.consume();
        tokenizer.exit(Name::MathTextSequence);
        tokenizer.exit(Name::MathText);
        reset(tokenizer);
        return State::Ok;
    }
    tokenizer.exit(Name::MathTextSequence);
    let len = tokenizer.events.len();
    tokenizer.events[len - 2].name = Name::MathTextData;
    tokenizer.events[len - 1].name = Name::MathTextData;
    tokenizer.tokenize_state.seen = false;
    State::Retry(StateName::MathTextBetween)
}

fn nok(tokenizer: &mut Tokenizer) -> State {
    reset(tokenizer);
    State::Nok
}

fn reset(tokenizer: &mut Tokenizer) {
    tokenizer.tokenize_state.size = 0;
    tokenizer.tokenize_state.size_b = 0;
    tokenizer.tokenize_state.seen = false;
}
