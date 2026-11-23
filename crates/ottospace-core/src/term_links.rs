//! Links in a terminal's text: a URL printed as text (wrapped across rows too), or an OSC 8
//! hyperlink. Pure: the shell gives the rows around a cell, this finds the link on it, and
//! `links::target` judges what may open (`http`, `https`).

use crate::links::{self, Target};

pub const USERS_ALONE: &str = "only the user's ⌘-click opens a terminal's link";

/// The longest URL read; longer ones are not links.
const MAX_URL: usize = 2048;

/// One grid row: a character per column (`'\0'`: a wide character's second half), each column's
/// OSC 8 target if it has one, and whether the row wraps into the next.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub cells: Vec<char>,
    pub targets: Vec<Option<String>>,
    pub wraps: bool,
}

/// A link: what opens, the cells it covers (`(row, first column, last column)`, rows of the
/// slice given), and whether its text is not what opens (an OSC 8 link's: shown before it opens).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub url: String,
    pub cells: Vec<(usize, usize, usize)>,
    pub differs: bool,
}

/// Rows to read above and below a cell, `cols` wide: a URL that reaches past them is longer
/// than `MAX_URL` even with wide characters, so a cut one is never opened.
pub fn reach(cols: usize) -> usize {
    2 * MAX_URL / cols.max(1) + 2
}

/// The link on cell `(row, col)` of `rows`, if one there opens.
pub fn link_at(rows: &[Row], row: usize, col: usize) -> Option<Link> {
    if row >= rows.len() {
        return None;
    }
    // The line `row` is part of: rows joined where one wraps into the next.
    let first = (0..row).rev().take_while(|r| rows[*r].wraps).last().unwrap_or(row);
    let last = (row..rows.len()).find(|r| !rows[*r].wraps).unwrap_or(rows.len().saturating_sub(1));
    let mut flat: Vec<(usize, usize, char, Option<&str>)> = Vec::new();
    for (r, line) in rows.iter().enumerate().take(last + 1).skip(first) {
        for (c, ch) in line.cells.iter().enumerate().filter(|(_, ch)| **ch != '\0') {
            flat.push((r, c, *ch, line.targets.get(c).and_then(|t| t.as_deref())));
        }
    }
    // A wide character's second half is the character.
    let at = flat.iter().rposition(|(r, c, ..)| *r == row && *c <= col)?;
    let wide = |i: usize| rows[flat[i].0].cells.get(flat[i].1 + 1) == Some(&'\0');
    if flat[at].1 != col && !(flat[at].1 + 1 == col && wide(at)) {
        return None;
    }
    let (start, end, url, differs) = match flat[at].3 {
        // OSC 8: the run of cells with its target; that target opens, whatever the text says.
        Some(target) => {
            let start = flat[..at].iter().rposition(|cell| cell.3 != Some(target)).map_or(0, |i| i + 1);
            let end = flat[at..].iter().position(|cell| cell.3 != Some(target)).map_or(flat.len(), |i| at + i);
            let text: String = flat[start..end].iter().map(|cell| cell.2).collect();
            (start, end, target.to_string(), text.trim() != target)
        }
        None => {
            let chars: Vec<char> = flat.iter().map(|cell| cell.2).collect();
            let (start, end) = url_around(&chars, at)?;
            (start, end, chars[start..end].iter().collect(), false)
        }
    };
    let Some(Target::Web(url)) = links::target(&url, None) else { return None };
    let mut cells: Vec<(usize, usize, usize)> = Vec::new();
    for (i, (r, c, ..)) in flat.iter().enumerate().take(end).skip(start) {
        let c_end = if wide(i) { c + 1 } else { *c };
        match cells.last_mut() {
            Some((row, _, to)) if row == r => *to = c_end,
            _ => cells.push((*r, *c, c_end)),
        }
    }
    Some(Link { url, cells, differs })
}

/// The span of a plain `http://` or `https://` URL in `chars` that covers index `at`. It ends at a
/// space or a character no URL has unquoted; trailing punctuation and a closing bracket it did not
/// open are left out (`(see https://a.com/x).` → `https://a.com/x`).
fn url_around(chars: &[char], at: usize) -> Option<(usize, usize)> {
    let lower: Vec<char> = chars.iter().map(char::to_ascii_lowercase).collect();
    let starts = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| lower.get(i + k) == Some(&c));
    let stops = |c: char| c.is_whitespace() || c.is_control() || "<>\"`{}|^\\⟨⟩".contains(c);
    let start = (0..=at).rev().find(|i| starts(*i, "http://") || starts(*i, "https://"))?;
    let mut end = start + chars[start..].iter().position(|c| stops(*c)).unwrap_or(chars.len() - start);
    loop {
        let text = &chars[start..end];
        let count = |c: char| text.iter().filter(|t| **t == c).count();
        match text.last() {
            Some('.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*') => end -= 1,
            Some(')') if count('(') < count(')') => end -= 1,
            Some(']') if count('[') < count(']') => end -= 1,
            _ => break,
        }
    }
    (at < end && end - start <= MAX_URL).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows `cols` wide from text, wrapped where it runs past them; `\n` ends a line.
    fn rows(text: &str, cols: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        for line in text.split('\n') {
            let chars: Vec<char> = line.chars().collect();
            let pieces: Vec<&[char]> = if chars.is_empty() { vec![&[]] } else { chars.chunks(cols).collect() };
            let count = pieces.len();
            for (i, piece) in pieces.into_iter().enumerate() {
                let cells = piece.to_vec();
                rows.push(Row { targets: vec![None; cells.len()], cells, wraps: i + 1 < count });
            }
        }
        rows
    }

    fn url(rows: &[Row], row: usize, col: usize) -> Option<String> {
        link_at(rows, row, col).map(|link| link.url)
    }

    #[test]
    fn a_url_in_text_is_found_without_the_punctuation_after_it() {
        let cases = [
            ("see https://example.com/a?b=1#c.", "https://example.com/a?b=1#c"),
            ("(see https://example.com/x).", "https://example.com/x"),
            ("https://en.example.org/wiki/A_(b)", "https://en.example.org/wiki/A_(b)"),
            ("[link](https://example.com/y)", "https://example.com/y"),
            ("'http://example.com:8080/z',", "http://example.com:8080/z"),
            ("<https://example.com/q>", "https://example.com/q"),
            ("\"https://example.com/w\"", "https://example.com/w"),
            ("HTTPS://Example.com/k!", "HTTPS://Example.com/k"),
        ];
        for (text, want) in cases {
            let rows = rows(text, 200);
            let at = text.find(['h', 'H']).unwrap_or(0) + 2;
            assert_eq!(url(&rows, 0, at).as_deref(), Some(want), "{text:?}");
        }
    }

    #[test]
    fn only_a_cell_of_the_url_is_on_it() {
        let rows = rows("go to https://example.com now", 80);
        assert_eq!(url(&rows, 0, 2), None, "before it");
        assert_eq!(url(&rows, 0, 6).as_deref(), Some("https://example.com"));
        assert_eq!(url(&rows, 0, 25), None, "the space after it");
        assert_eq!(url(&rows, 0, 79), None, "past the text");
        assert_eq!(url(&rows, 3, 0), None, "past the rows");
        assert_eq!(link_at(&rows, 0, 24).map(|link| link.cells), Some(vec![(0, 6, 24)]));
    }

    #[test]
    fn a_wrapped_url_is_one_and_a_new_line_ends_it() {
        let text = "x https://example.com/a/long/path y";
        let rows = rows(text, 12);
        let want = Some("https://example.com/a/long/path".to_string());
        assert_eq!(url(&rows, 0, 4), want);
        assert_eq!(url(&rows, 2, 0), want, "its last row");
        let cells = link_at(&rows, 1, 3).map(|link| link.cells);
        assert_eq!(cells, Some(vec![(0, 2, 11), (1, 0, 11), (2, 0, 8)]));
        // Not wrapped: the next row is another line.
        let rows = super::tests::rows("https://example.com/a\n/b", 40);
        assert_eq!(url(&rows, 1, 1), None);
        assert_eq!(url(&rows, 0, 3).as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn only_web_pages_are_links() {
        for text in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@example.com",
            "ftp://example.com",
            "https://user@evil.example.com/",
            "https://",
            "myapp://open",
        ] {
            let rows = rows(text, 80);
            assert_eq!(url(&rows, 0, 3), None, "{text:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert_eq!(url(&rows(&long, 80), 0, 3), None, "too long");
    }

    #[test]
    fn an_osc_8_link_opens_its_target_and_says_when_the_text_differs() {
        let mut rows = rows("click here or https://example.com/t", 80);
        for c in 6..10 {
            rows[0].targets[c] = Some("https://example.com/real".into());
        }
        let link = link_at(&rows, 0, 7).expect("a link");
        assert_eq!(
            (link.url.as_str(), link.cells.as_slice(), link.differs),
            ("https://example.com/real", &[(0, 6, 9)][..], true)
        );
        assert_eq!(link_at(&rows, 0, 3), None, "outside its run");
        // Its text is its target: nothing to warn of.
        for c in 14..35 {
            rows[0].targets[c] = Some("https://example.com/t".into());
        }
        assert_eq!(link_at(&rows, 0, 20).map(|l| l.differs), Some(false));
        // A target that may not open is no link, whatever its text.
        for c in 6..10 {
            rows[0].targets[c] = Some("file:///etc/passwd".into());
        }
        assert_eq!(link_at(&rows, 0, 7), None);
    }

    #[test]
    fn a_wide_characters_two_columns_are_one_cell() {
        let cells: Vec<char> = "日\0 https://example.com/日\0 x".chars().collect();
        let rows = vec![Row { targets: vec![None; cells.len()], cells, wraps: false }];
        let link = link_at(&rows, 0, 4).expect("a link");
        assert_eq!((link.url.as_str(), link.cells.as_slice()), ("https://example.com/日", &[(0, 3, 24)][..]));
        assert_eq!(url(&rows, 0, 24).as_deref(), Some("https://example.com/日"), "its second half");
        assert_eq!(url(&rows, 0, 1), None);
    }

    #[test]
    fn a_url_cut_at_the_rows_read_is_too_long_to_open() {
        // `reach` rows either side of a cell hold more than a URL may have, even at half a row's characters.
        for cols in [1, 7, 80, 300] {
            assert!(reach(cols) * cols / 2 > MAX_URL, "{cols}");
        }
    }
}
