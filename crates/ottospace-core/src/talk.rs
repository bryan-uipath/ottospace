//! The right drawer: one conversation at a time, beside the workspace and in no tab. Today a
//! conversation is a task's agent in a terminal the shell hosts; what the drawer shows of it
//! is the shell's to draw. See `docs/ai/task-conversation.md`.

use serde::Serialize;

use crate::tasks::TaskId;

/// The drawer's width in points when nobody has dragged it, and the least and most it takes.
pub const WIDTH: f32 = 480.;
pub const MIN_WIDTH: f32 = 320.;
pub const MAX_WIDTH: f32 = 1600.;

/// The conversation the drawer is on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Talk {
    /// Whose it is. A session that is no task's will be a second kind of subject here.
    pub task: TaskId,
    /// False: closed to the right rail. The agent goes on; the rail's button opens it again.
    pub open: bool,
    /// The keyboard is in it: keys go to the agent, as in an app pane they go to the app.
    pub keys: bool,
}

/// A width asked for (a drag of the drawer's edge), as one it may have.
pub fn width(asked: f32) -> f32 {
    if asked.is_finite() { asked.clamp(MIN_WIDTH, MAX_WIDTH) } else { WIDTH }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_width_is_kept_within_bounds() {
        let asked = [0., 400., 5000., f32::NAN, f32::INFINITY, -1.];
        assert_eq!(asked.map(width), [MIN_WIDTH, 400., MAX_WIDTH, WIDTH, WIDTH, MIN_WIDTH]);
    }
}
