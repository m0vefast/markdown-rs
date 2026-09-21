//! Attention (emphasis, strong, optionally GFM strikethrough) occurs in the
//! [text][] content type.
//!
//! ## Grammar
//!
//! Attention sequences form with the following BNF
//! (<small>see [construct][crate::construct] for character groups</small>):
//!
//! ```bnf
//! attention_sequence ::= 1*'*' | 1*'_'
//! gfm_attention_sequence ::= 1*'~'
//! ```
//!
//! Sequences are matched together to form attention based on which character
//! they contain, how long they are, and what character occurs before and after
//! each sequence.
//! Otherwise they are turned into data.
//!
//! ## HTML
//!
//! When asterisk/underscore sequences match, and two markers can be “taken”
//! from them, they together relate to the `<strong>` element in HTML.
//! When one marker can be taken, they relate to the `<em>` element.
//! See [*§ 4.5.2 The `em` element*][html-em] and
//! [*§ 4.5.3 The `strong` element*][html-strong] in the HTML spec for more
//! info.
//!
//! When tilde sequences match, they together relate to the `<del>` element in
//! HTML.
//! See [*§ 4.7.2 The `del` element*][html-del] in the HTML spec for more info.
//!
//! ## Recommendation
//!
//! It is recommended to use asterisks for emphasis/strong attention when
//! writing markdown.
//!
//! There are some small differences in whether sequences can open and/or close
//! based on whether they are formed with asterisks or underscores.
//! Because underscores also frequently occur in natural language inside words,
//! while asterisks typically never do, `CommonMark` prohibits underscore
//! sequences from opening or closing when *inside* a word.
//!
//! Because asterisks can be used to form the most markdown constructs, using
//! them has the added benefit of making it easier to gloss over markdown: you
//! can look for asterisks to find syntax while not worrying about other
//! characters.
//!
//! For strikethrough attention, it is recommended to use two markers.
//! While `github.com` allows single tildes too, it technically prohibits it in
//! their spec.
//!
//! ## Tokens
//!
//! * [`Emphasis`][Name::Emphasis]
//! * [`EmphasisSequence`][Name::EmphasisSequence]
//! * [`EmphasisText`][Name::EmphasisText]
//! * [`GfmStrikethrough`][Name::GfmStrikethrough]
//! * [`GfmStrikethroughSequence`][Name::GfmStrikethroughSequence]
//! * [`GfmStrikethroughText`][Name::GfmStrikethroughText]
//! * [`Strong`][Name::Strong]
//! * [`StrongSequence`][Name::StrongSequence]
//! * [`StrongText`][Name::StrongText]
//!
//! > 👉 **Note**: while parsing, [`AttentionSequence`][Name::AttentionSequence]
//! > is used, which is later compiled away.
//!
//! ## References
//!
//! * [`attention.js` in `micromark`](https://github.com/micromark/micromark/blob/main/packages/micromark-core-commonmark/dev/lib/attention.js)
//! * [`micromark-extension-gfm-strikethrough`](https://github.com/micromark/micromark-extension-gfm-strikethrough)
//! * [*§ 6.2 Emphasis and strong emphasis* in `CommonMark`](https://spec.commonmark.org/0.31/#emphasis-and-strong-emphasis)
//! * [*§ 6.5 Strikethrough (extension)* in `GFM`](https://github.github.com/gfm/#strikethrough-extension-)
//!
//! [text]: crate::construct::text
//! [html-em]: https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-em-element
//! [html-strong]: https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-strong-element
//! [html-del]: https://html.spec.whatwg.org/multipage/edits.html#the-del-element

use crate::event::{Event, Kind, Name, Point};
use crate::resolve::Name as ResolveName;
use crate::state::{Name as StateName, State};
use crate::subtokenize::Subresult;
use crate::tokenizer::Tokenizer;
use crate::util::char::{
    after_index as char_after_index, before_index as char_before_index, classify_opt,
    Kind as CharacterKind,
};
use alloc::{collections::BTreeMap, vec, vec::Vec};

/// Attentention sequence that we can take markers from.
#[derive(Debug)]
struct Sequence {
    /// Marker as a byte (`u8`) used in this sequence.
    marker: u8,
    /// We track whether sequences are in balanced events, and where those
    /// events start, so that one attention doesn’t start in say, one link, and
    /// end in another.
    stack: Vec<usize>,
    /// The index into events where this sequence’s `Enter` currently resides.
    index: usize,
    /// The (shifted) point where this sequence starts.
    start_point: Point,
    /// The (shifted) point where this sequence end.
    end_point: Point,
    /// The number of markers we can still use.
    size: usize,
    /// Whether this sequence can open attention.
    open: bool,
    /// Whether this sequence can close attention.
    close: bool,
}

/// At start of attention.
///
/// ```markdown
/// > | **
///     ^
/// ```
pub fn start(tokenizer: &mut Tokenizer) -> State {
    // Emphasis/strong:
    if (tokenizer.parse_state.options.constructs.attention
        && matches!(tokenizer.current, Some(b'*' | b'_')))
        // GFM strikethrough:
        || (tokenizer.parse_state.options.constructs.gfm_strikethrough && tokenizer.current == Some(b'~'))
    {
        tokenizer.tokenize_state.marker = tokenizer.current.unwrap();
        tokenizer.enter(Name::AttentionSequence);
        State::Retry(StateName::AttentionInside)
    } else {
        State::Nok
    }
}

/// In sequence.
///
/// ```markdown
/// > | **
///     ^^
/// ```
pub fn inside(tokenizer: &mut Tokenizer) -> State {
    if tokenizer.current == Some(tokenizer.tokenize_state.marker) {
        tokenizer.consume();
        State::Next(StateName::AttentionInside)
    } else {
        tokenizer.exit(Name::AttentionSequence);
        tokenizer.register_resolver(ResolveName::Attention);
        tokenizer.tokenize_state.marker = 0;
        State::Ok
    }
}

/// The sequences that may still open, and where the backward walk for a closer
/// looks among them.
///
/// Each set is a disjoint-set forest over positions shifted by one (node `0` is
/// “none”): a sequence that can no longer open — spent, or closed off by a match
/// around it — never can again, so its node points down and a walk jumps every
/// such run in amortized constant time. A spent sequence stays in place instead
/// of being removed from the list: each removal shifted everything after it.
///
/// A closer only matches an opener with its marker and its stack, so it walks its
/// own group alone. And once a closer of some kind finds no opener, nothing below
/// it in its group opens for that kind until one of those sequences changes size,
/// which only a match does (`bottom`, lowered in `resized`) — the bound
/// CommonMark’s reference parser keeps as `openers_bottom`. Without these, every
/// closer rescanned every opener that could never match it.
struct Openers {
    all: Vec<usize>,
    group: Vec<usize>,
    place: Vec<usize>,
    members: Vec<Vec<usize>>,
    forests: Vec<Vec<usize>>,
    bottom: Vec<[usize; 6]>,
}

/// The root of `node` in `forest`, halving the path to it.
fn root(forest: &mut [usize], mut node: usize) -> usize {
    while forest[node] != node {
        let grand = forest[forest[node]];
        forest[node] = grand;
        node = grand;
    }
    node
}

/// The kind of closer a sequence is, as far as which openers it matches: whether
/// it can open, and its size — modulo three for the rule of three, or exactly for
/// strikethrough.
fn closer_kind(sequence: &Sequence) -> usize {
    let size = if sequence.marker == b'~' {
        sequence.size.min(3) - 1
    } else {
        sequence.size % 3
    };
    usize::from(sequence.open) * 3 + size
}

impl Openers {
    fn new(sequences: &[Sequence]) -> Self {
        let mut groups: BTreeMap<(u8, Option<usize>), usize> = BTreeMap::new();
        let mut all = Vec::with_capacity(sequences.len() + 1);
        let mut group = Vec::with_capacity(sequences.len());
        let mut place = Vec::with_capacity(sequences.len());
        let mut members: Vec<Vec<usize>> = vec![];
        all.push(0);
        for (index, sequence) in sequences.iter().enumerate() {
            all.push(if sequence.open { index + 1 } else { index });
            let fresh = members.len();
            let id = *groups
                .entry((sequence.marker, sequence.stack.last().copied()))
                .or_insert(fresh);
            if id == fresh {
                members.push(vec![]);
            }
            group.push(id);
            place.push(members[id].len());
            members[id].push(index);
        }
        let forests = members
            .iter()
            .map(|member| {
                let mut forest = Vec::with_capacity(member.len() + 1);
                forest.push(0);
                for (at, index) in member.iter().enumerate() {
                    forest.push(if sequences[*index].open { at + 1 } else { at });
                }
                forest
            })
            .collect();
        let bottom = vec![[0; 6]; members.len()];
        Openers {
            all,
            group,
            place,
            members,
            forests,
            bottom,
        }
    }

    /// The nearest sequence before `index` that may still open.
    fn before(&mut self, index: usize) -> Option<usize> {
        root(&mut self.all, index).checked_sub(1)
    }

    /// The nearest sequence of `close`’s group before `index` that may still open
    /// for a closer of `kind`.
    fn opener_before(&mut self, close: usize, index: usize, kind: usize) -> Option<usize> {
        let id = self.group[close];
        let at = root(&mut self.forests[id], self.place[index]).checked_sub(1)?;
        (at >= self.bottom[id][kind]).then(|| self.members[id][at])
    }

    /// Nothing before `close` in its group opens for a closer of `kind`.
    fn exhausted(&mut self, close: usize, kind: usize) {
        self.bottom[self.group[close]][kind] = self.place[close];
    }

    /// The sequence at `index` changed size: it may open for closers it could not.
    fn resized(&mut self, index: usize) {
        let at = self.place[index];
        for bottom in &mut self.bottom[self.group[index]] {
            *bottom = (*bottom).min(at);
        }
    }

    /// The sequence at `index` can no longer open.
    fn close(&mut self, index: usize) {
        self.all[index + 1] = index;
        let at = self.place[index];
        self.forests[self.group[index]][at + 1] = at;
    }
}

/// Resolve sequences.
pub fn resolve(tokenizer: &mut Tokenizer) -> Option<Subresult> {
    // Find all sequences, gather info about them.
    let mut sequences = get_sequences(tokenizer);
    let mut openers = Openers::new(&sequences);

    // Now walk through them and match them.
    let mut close = 0;

    while close < sequences.len() {
        let sequence_close = &sequences[close];
        let mut next_index = close + 1;

        // Find a sequence that can close.
        if sequence_close.close {
            let kind = closer_kind(sequence_close);
            let mut candidate = openers.opener_before(close, close, kind);
            let mut matched = false;

            // Now walk back to find an opener.
            while let Some(open) = candidate {
                candidate = openers.opener_before(close, open, kind);

                let sequence_open = &sequences[open];

                // An opener matching our closer:
                if sequence_open.open
                    && sequence_close.marker == sequence_open.marker
                    && sequence_close.stack == sequence_open.stack
                {
                    // If the opening can close or the closing can open,
                    // and the close size *is not* a multiple of three,
                    // but the sum of the opening and closing size *is*
                    // multiple of three, then **don’t** match.
                    if (sequence_open.close || sequence_close.open)
                        && sequence_close.size % 3 != 0
                        && (sequence_open.size + sequence_close.size) % 3 == 0
                    {
                        continue;
                    }

                    // For GFM strikethrough:
                    // * both sequences must have the same size
                    // * more than 2 markers don’t work
                    // * one marker is prohibited by the spec, but supported by GH
                    if sequence_close.marker == b'~'
                        && (sequence_close.size != sequence_open.size
                            || sequence_close.size > 2
                            || sequence_close.size == 1
                                && !tokenizer.parse_state.options.gfm_strikethrough_single_tilde)
                    {
                        continue;
                    }

                    // We found a match!
                    next_index = match_sequences(tokenizer, &mut sequences, &mut openers, open, close);
                    matched = true;

                    break;
                }
            }

            if !matched {
                openers.exhausted(close, kind);
            }
        }

        close = next_index;
    }

    // Mark remaining sequences as data.
    let mut index = 0;
    while index < sequences.len() {
        let sequence = &sequences[index];
        if sequence.size > 0 {
            tokenizer.events[sequence.index].name = Name::Data;
            tokenizer.events[sequence.index + 1].name = Name::Data;
        }
        index += 1;
    }

    tokenizer.map.consume(&mut tokenizer.events);
    None
}

/// Get sequences.
fn get_sequences(tokenizer: &mut Tokenizer) -> Vec<Sequence> {
    let mut index = 0;
    let mut stack = vec![];
    let mut sequences = vec![];

    while index < tokenizer.events.len() {
        let enter = &tokenizer.events[index];

        if enter.name == Name::AttentionSequence {
            if enter.kind == Kind::Enter {
                let end = index + 1;
                let exit = &tokenizer.events[end];

                let marker = tokenizer.parse_state.bytes[enter.point.index];
                let before_char = char_before_index(tokenizer.parse_state.bytes, enter.point.index);
                let before = classify_opt(before_char);
                let after_char = char_after_index(tokenizer.parse_state.bytes, exit.point.index);
                let after = classify_opt(after_char);
                let open = after == CharacterKind::Other
                    || (after == CharacterKind::Punctuation && before != CharacterKind::Other)
                    // For regular attention markers (not strikethrough), the
                    // other attention markers can be used around them
                    || (marker != b'~' && matches!(after_char, Some('*' | '_')))
                    || (marker != b'~' && tokenizer.parse_state.options.constructs.gfm_strikethrough && matches!(after_char, Some('~')));
                let close = before == CharacterKind::Other
                    || (before == CharacterKind::Punctuation && after != CharacterKind::Other)
                    || (marker != b'~' && matches!(before_char, Some('*' | '_')))
                    || (marker != b'~'
                        && tokenizer.parse_state.options.constructs.gfm_strikethrough
                        && matches!(before_char, Some('~')));

                sequences.push(Sequence {
                    index,
                    stack: stack.clone(),
                    start_point: enter.point.clone(),
                    end_point: exit.point.clone(),
                    size: exit.point.index - enter.point.index,
                    open: if marker == b'_' {
                        open && (before != CharacterKind::Other || !close)
                    } else {
                        open
                    },
                    close: if marker == b'_' {
                        close && (after != CharacterKind::Other || !open)
                    } else {
                        close
                    },
                    marker,
                });
            }
        } else if enter.kind == Kind::Enter {
            stack.push(index);
        } else {
            stack.pop();
        }

        index += 1;
    }

    sequences
}

/// Match two sequences.
#[allow(clippy::too_many_lines)]
fn match_sequences(
    tokenizer: &mut Tokenizer,
    sequences: &mut [Sequence],
    openers: &mut Openers,
    open: usize,
    close: usize,
) -> usize {
    // Where to move to next.
    // Stay on this closing sequence for the next iteration: it
    // might close more things.
    // It’s changed if the closing sequence is spent.
    let mut next = close;

    // Number of markers to use from the sequence.
    let take = if sequences[open].size > 1 && sequences[close].size > 1 {
        2
    } else {
        1
    };

    // We’re *on* a closing sequence, with a matching opening
    // sequence.
    // Now we make sure that we can’t have misnested attention:
    //
    // ```html
    // <em>a <strong>b</em> c</strong>
    // ```
    //
    // Do that by marking everything between it as no longer
    // possible to open anything.
    // Theoretically we should mark as `close: false` too, but
    // we don’t look for closers backwards, so it’s not needed.
    // Only sequences that may still open have anything to lose.
    let mut between = openers.before(close);

    while let Some(index) = between.filter(|&index| index > open) {
        sequences[index].open = false;
        openers.close(index);
        between = openers.before(index);
    }

    let (group_name, seq_name, text_name) = if sequences[open].marker == b'~' {
        (
            Name::GfmStrikethrough,
            Name::GfmStrikethroughSequence,
            Name::GfmStrikethroughText,
        )
    } else if take == 1 {
        (Name::Emphasis, Name::EmphasisSequence, Name::EmphasisText)
    } else {
        (Name::Strong, Name::StrongSequence, Name::StrongText)
    };
    let open_index = sequences[open].index;
    let close_index = sequences[close].index;
    let open_exit = sequences[open].end_point.clone();
    let close_enter = sequences[close].start_point.clone();

    // No need to worry about `VS`, because sequences are only actual characters.
    sequences[open].size -= take;
    sequences[close].size -= take;
    sequences[open].end_point.column -= take;
    sequences[open].end_point.index -= take;
    sequences[close].start_point.column += take;
    sequences[close].start_point.index += take;

    // Opening.
    tokenizer.map.add_before(
        // Add after the current sequence (it might remain).
        open_index + 2,
        0,
        vec![
            Event {
                kind: Kind::Enter,
                name: group_name.clone(),
                point: sequences[open].end_point.clone(),
                link: None,
            },
            Event {
                kind: Kind::Enter,
                name: seq_name.clone(),
                point: sequences[open].end_point.clone(),
                link: None,
            },
            Event {
                kind: Kind::Exit,
                name: seq_name.clone(),
                point: open_exit.clone(),
                link: None,
            },
            Event {
                kind: Kind::Enter,
                name: text_name.clone(),
                point: open_exit,
                link: None,
            },
        ],
    );
    // Closing.
    tokenizer.map.add(
        close_index,
        0,
        vec![
            Event {
                kind: Kind::Exit,
                name: text_name,
                point: close_enter.clone(),
                link: None,
            },
            Event {
                kind: Kind::Enter,
                name: seq_name.clone(),
                point: close_enter,
                link: None,
            },
            Event {
                kind: Kind::Exit,
                name: seq_name,
                point: sequences[close].start_point.clone(),
                link: None,
            },
            Event {
                kind: Kind::Exit,
                name: group_name,
                point: sequences[close].start_point.clone(),
                link: None,
            },
        ],
    );

    // Spend closing sequence if fully used.
    if sequences[close].size == 0 {
        sequences[close].open = false;
        sequences[close].close = false;
        openers.close(close);
        tokenizer.map.add(close_index, 2, vec![]);
        next += 1;
    } else {
        openers.resized(close);
        // Shift remaining closing sequence forward.
        // Do it here because a sequence can open and close different
        // other sequences, and the remainder can be on any side or
        // somewhere in the middle.
        tokenizer.events[close_index].point = sequences[close].start_point.clone();
    }

    if sequences[open].size == 0 {
        sequences[open].open = false;
        sequences[open].close = false;
        openers.close(open);
        tokenizer.map.add(open_index, 2, vec![]);
    } else {
        openers.resized(open);
        tokenizer.events[open_index + 1].point = sequences[open].end_point.clone();
    }

    next
}
