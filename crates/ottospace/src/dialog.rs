//! What ottospace's floating panels share: the new-task box, the picker, the key reference.

use gpui::{BoxShadow, black, point, px};

/// Corner radius of a panel, and of what sits inside one.
pub const RADIUS: f32 = 13.;
pub const INNER_RADIUS: f32 = 8.;

/// Two layers: a tight dark one that lifts the edge, a wide soft one under the panel.
pub fn shadow() -> Vec<BoxShadow> {
    let layer = |opacity, y, blur| BoxShadow {
        color: black().opacity(opacity),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    };
    vec![layer(0.55, 2., 6.), layer(0.45, 18., 48.)]
}
