//! A paragraph reads the same whatever container closed before it (CommonMark
//! §4.8 / §4.10 / §5.2: a line that cannot interrupt a paragraph continues it).

use markdown::{to_html_with_options, CompileOptions, Options};
use pretty_assertions::assert_eq;

/// Lines that cannot interrupt a paragraph.
const CANNOT_INTERRUPT: &[&str] = &[
    "2. two",
    "10) ten",
    "-",
    "*",
    "1.",
    "<my-tag>",
    "</my-tag>",
    "<a href=\"x\">",
    "    indented",
    "[d]: /url",
    "\t\ttab",
];

const CLOSED_BEFORE: &[&str] = &["- a\n\n", "* a\n\n", "1. a\n\n", "> a\n\n", "- a\n  - b\n\n", "> - a\n\n", "- > a\n\n"];

fn html(s: &str) -> String {
    let o = Options {
        compile: CompileOptions { allow_dangerous_html: true, ..Default::default() },
        ..Options::gfm()
    };
    to_html_with_options(s, &o).unwrap()
}

#[test]
fn a_closed_container_does_not_change_the_paragraph_after_it() {
    let mut bad = Vec::new();
    for c in CLOSED_BEFORE {
        for x in CANNOT_INTERRUPT {
            let tail = format!("para\n{x}\n");
            let got = html(&format!("{c}{tail}"));
            let want = format!("{}{}", html(c), html(&tail));
            if got != want {
                bad.push(format!("{:?}\n  got  {got:?}\n  want {want:?}", format!("{c}{tail}")));
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
}

/// HTML that only its tag makes a block (CommonMark kind 7) cannot interrupt a
/// paragraph, on a lazy line too (commonmark.js `blockStarts` HTML: kind 7 needs a
/// tip that is not a paragraph): the line continues the paragraph inside its
/// container, as if written inside it.
#[test]
fn a_lazy_complete_html_line_continues_the_paragraph() {
    let mut bad = Vec::new();
    for (open, inside) in [("- b\n", "  "), ("1. b\n", "   "), ("> b\n", "> "), ("- a\n- b\n", "  "), ("> - b\n", ">   ")] {
        for x in ["<my-tag>", "</my-tag>", "<a href=\"x\">", "<a>"] {
            let got = html(&format!("{open}{x}\nstill\n"));
            let want = html(&format!("{open}{inside}{x}\n{inside}still\n"));
            if got != want {
                bad.push(format!("{:?}\n  got  {got:?}\n  want {want:?}", format!("{open}{x}\nstill\n")));
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
}
