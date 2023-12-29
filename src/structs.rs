use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub(crate) struct Instance {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) matcher: WindowMatcher,
    pub(crate) window_delay: Option<u64>,
    pub(crate) geometry: WindowGeometry,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct WindowGeometry {
    pub(crate) width: String,
    pub(crate) height: String,
}

impl WindowGeometry {
    /// Get the dimensions of the instance based on the screen dimensions.
    pub(crate) fn get_dimensions(&self, screen_width: u16, screen_height: u16) -> (u32, u32) {
        let width: u32 = if self.width.ends_with('%') {
            let width_pct = self
                .width
                .strip_suffix('%')
                .unwrap()
                .parse::<f64>()
                .expect("invalid width")
                / 100.0;
            (screen_width as f64 * width_pct) as u32
        } else {
            self.width.parse().expect("invalid width")
        };
        let height: u32 = if self.height.ends_with('%') {
            let height_pct = self
                .height
                .strip_suffix('%')
                .unwrap()
                .parse::<f64>()
                .expect("invalid height")
                / 100.0;
            (screen_height as f64 * height_pct) as u32
        } else {
            self.height.parse().expect("invalid height")
        };
        (width, height)
    }
}

pub(crate) struct Context {
    pub matcher: WindowMatcher,
    pub window_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) enum WindowMatcher {
    ProcessId(Option<u32>),
    WmClass(String),
}
