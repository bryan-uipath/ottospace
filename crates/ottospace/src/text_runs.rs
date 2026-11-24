//! A text whose runs are made when it is laid out, from the style it inherits there: its
//! highlights over that style, and some ranges in another font family (a reply's inline code in
//! the monospace one). One text, so the code wraps with its words and its box runs on unbroken.

use std::ops::Range;

use gpui::{
    AnyElement, App, Bounds, ElementId, GlobalElementId, HighlightStyle, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, StyledText, TextRun, TextStyle, Window, prelude::*,
};

/// What wraps the text once its runs are set: itself, or a clickable text around it.
type Wrap = Box<dyn FnOnce(StyledText) -> AnyElement>;

pub(crate) struct Runs {
    styled: Option<StyledText>,
    text: SharedString,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    family: Option<(SharedString, Vec<Range<usize>>)>,
    wrap: Option<Wrap>,
    built: Option<AnyElement>,
}

impl Runs {
    /// `styled` (made of `text`, its layout already handed out), its `styles` (sorted, apart),
    /// and `ranges` of it in `family`; `wrap` makes the element drawn of it.
    pub(crate) fn new(
        styled: StyledText,
        text: SharedString,
        styles: Vec<(Range<usize>, HighlightStyle)>,
        family: Option<(SharedString, Vec<Range<usize>>)>,
        wrap: impl FnOnce(StyledText) -> AnyElement + 'static,
    ) -> Self {
        Runs { styled: Some(styled), text, styles, family, wrap: Some(Box::new(wrap)), built: None }
    }
}

/// The runs of a text `len` bytes long over `base`: `styles` (sorted, apart) laid over it, and
/// `ranges` in `family`. Each run ends where a style or a range does.
pub(crate) fn runs(
    len: usize,
    base: &TextStyle,
    styles: &[(Range<usize>, HighlightStyle)],
    family: Option<(&SharedString, &[Range<usize>])>,
) -> Vec<TextRun> {
    let mut cuts: Vec<usize> = vec![0, len];
    cuts.extend(styles.iter().flat_map(|(r, _)| [r.start, r.end]));
    if let Some((_, ranges)) = family {
        cuts.extend(ranges.iter().flat_map(|r| [r.start, r.end]));
    }
    cuts.retain(|at| *at <= len);
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .map(|w| {
            let (start, end) = (w[0], w[1]);
            let mut style = base.clone();
            if let Some((_, highlight)) = styles.iter().find(|(r, _)| r.start <= start && end <= r.end) {
                style = style.highlight(*highlight);
            }
            if let Some((name, ranges)) = family
                && ranges.iter().any(|r| r.start <= start && end <= r.end)
            {
                style.font_family = name.clone();
            }
            style.to_run(end - start)
        })
        .collect()
}

impl IntoElement for Runs {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Runs {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        if let (Some(styled), Some(wrap)) = (self.styled.take(), self.wrap.take()) {
            let base = window.text_style();
            let family = self.family.as_ref().map(|(name, ranges)| (name, ranges.as_slice()));
            let runs = runs(self.text.len(), &base, &self.styles, family);
            self.built = Some(wrap(styled.with_runs(runs)));
        }
        let built = self.built.as_mut().expect("built at its first layout");
        (built.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(built) = self.built.as_mut() {
            built.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(built) = self.built.as_mut() {
            built.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{FontWeight, hsla};

    use super::*;

    /// A run ends where a style or a monospace range does; a range in the family keeps its
    /// style's weight and colour; the text is covered once, in order.
    #[test]
    fn runs_cut_at_styles_and_family_ranges() {
        let base = TextStyle::default();
        let bold = HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() };
        let boxed = HighlightStyle { background_color: Some(hsla(0., 0., 0.2, 1.)), ..Default::default() };
        // "see `a(b, c)` and **x**": code at 4..11, bold at 16..17.
        let styles = vec![(4..11, boxed), (16..17, bold)];
        let mono = SharedString::from("Menlo");
        let runs = runs(17, &base, &styles, Some((&mono, &[4..11])));
        let lens: Vec<usize> = runs.iter().map(|r| r.len).collect();
        assert_eq!(lens, [4, 7, 5, 1]);
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), 17);
        assert_eq!(runs[1].font.family, mono);
        assert!(runs[1].background_color.is_some() && runs[0].background_color.is_none());
        assert_ne!(runs[0].font.family, mono);
        assert_eq!(runs[3].font.weight, FontWeight::BOLD);
    }
}
