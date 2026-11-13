//! The app's side of the host channel (docs/RUNTIME.md "The host channel"): what whoever embeds a `GameSession`
//! does with the messages the game posts (`GameSession::take_posts`) and what it sends back
//! (`GameSession::host_messages`). The session only carries the messages; sockets, a HUD and the input devices
//! belong to the app. Here: the services a window takes its answers from (`HostServices`: the `threert-render`
//! binary's are its sockets and HTTP, src/net.rs), and the input message built from a window's key and pointer events
//! (`InputTracker`), with no window system in it.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// What an app offers the game beyond the picture: it is handed every message the game posts and asked once a frame
/// for what it has to say.
pub trait HostServices {
    /// A message the game posted (`{ "t": ..., ... }`). One the services do not know is not theirs: ignored.
    fn post(&mut self, message: &Value);
    /// The messages for the game since the last call, in order.
    fn poll(&mut self) -> Vec<Value>;
}

/// An app without services: posts go nowhere, nothing comes back.
pub struct NoServices;

impl HostServices for NoServices {
    fn post(&mut self, _: &Value) {}
    fn poll(&mut self) -> Vec<Value> {
        vec![]
    }
}

/// `input.look` is an angle (packages/gameplay/src/input.ts): radians a count of mouse motion at sensitivity 1 (a
/// point of a macOS pointer, a CSS pixel in a browser). 1,000 counts turn 143 degrees.
pub const LOOK_RADIANS: f32 = 0.0025;

/// The `launch` message: how the app opened the game.
pub fn launch_message(params: Option<&Value>) -> Value {
    json!({ "t": "launch", "params": params.filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({})) })
}

/// A window's keys and pointer between two frames, and the `input` message of a frame.
#[derive(Default)]
pub struct InputTracker {
    /// Physical keys down (by the window system's own name for them), and how many of them hold each key name
    /// ("Shift" is either shift key).
    codes: BTreeMap<String, String>,
    held: BTreeMap<String, u32>,
    buttons: BTreeSet<u8>,
    pressed: Vec<String>,
    released: Vec<String>,
    look: (f32, f32),
    wheel: f32,
    pointer: (f32, f32),
    pub captured: bool,
}

impl InputTracker {
    pub fn new() -> InputTracker {
        InputTracker { pointer: (0.5, 0.5), ..Default::default() }
    }

    /// A key went down or up: `code` is the physical key, `name` what game code calls it.
    pub fn key(&mut self, code: &str, name: &str, down: bool) {
        if down {
            if self.codes.insert(code.to_string(), name.to_string()).is_none() {
                let n = self.held.entry(name.to_string()).or_default();
                *n += 1;
                if *n == 1 {
                    self.pressed.push(name.to_string());
                }
            }
        } else if let Some(name) = self.codes.remove(code) {
            if let Some(n) = self.held.get_mut(&name) {
                *n -= 1;
                if *n == 0 {
                    self.held.remove(&name);
                    self.released.push(name);
                }
            }
        }
    }

    /// A mouse button (0 left, 1 middle, 2 right) went down or up.
    pub fn button(&mut self, n: u8, down: bool) {
        if down && self.buttons.insert(n) {
            self.pressed.push(format!("Mouse{n}"));
        } else if !down && self.buttons.remove(&n) {
            self.released.push(format!("Mouse{n}"));
        }
    }

    /// The pointer moved by (dx, dy) counts while captured: look, at `sensitivity`.
    pub fn motion(&mut self, dx: f32, dy: f32, sensitivity: f32) {
        self.look.0 += dx * LOOK_RADIANS * sensitivity;
        self.look.1 += dy * LOOK_RADIANS * sensitivity;
    }

    pub fn wheel(&mut self, notches: f32) {
        self.wheel += notches;
    }

    /// Where the pointer is in the picture (0..1 from the top left).
    pub fn pointer(&mut self, u: f32, v: f32) {
        self.pointer = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    }

    /// The window lost the keyboard: everything held is let go (its key-up events go elsewhere).
    pub fn release_all(&mut self) {
        self.codes.clear();
        self.released.extend(std::mem::take(&mut self.held).into_keys());
        self.released.extend(std::mem::take(&mut self.buttons).into_iter().map(|n| format!("Mouse{n}")));
    }

    /// The frame's `input` message; what lasts a frame (edges, look, wheel) starts again.
    pub fn message(&mut self) -> Value {
        let (dx, dy) = std::mem::take(&mut self.look);
        let (u, v) = if self.captured { (0.5, 0.5) } else { self.pointer };
        json!({
            "t": "input",
            "keys": self.held.keys().collect::<Vec<_>>(),
            "pressed": std::mem::take(&mut self.pressed),
            "released": std::mem::take(&mut self.released),
            "buttons": self.buttons.iter().collect::<Vec<_>>(),
            "u": u, "v": v, "dx": dx, "dy": dy,
            "wheel": std::mem::take(&mut self.wheel),
            "captured": self.captured,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frames_input_message() {
        let mut i = InputTracker::new();
        i.key("KeyW", "W", true);
        i.key("KeyW", "W", true); // the window system's key repeat
        i.key("ShiftLeft", "Shift", true);
        i.key("ShiftRight", "Shift", true);
        i.key("Space", "Space", true);
        i.key("Space", "Space", false); // a tap between two frames
        i.button(0, true);
        i.motion(100.0, -40.0, 2.0);
        i.wheel(1.0);
        i.pointer(0.25, 2.0);
        let m = i.message();
        assert_eq!(m["keys"], json!(["Shift", "W"]));
        assert_eq!(m["pressed"], json!(["W", "Shift", "Space", "Mouse0"]));
        assert_eq!(m["released"], json!(["Space"]));
        assert_eq!(m["buttons"], json!([0]));
        assert_eq!((m["u"].as_f64(), m["v"].as_f64()), (Some(0.25), Some(1.0)));
        assert!((m["dx"].as_f64().unwrap() - 0.5).abs() < 1e-6 && (m["dy"].as_f64().unwrap() + 0.2).abs() < 1e-6, "{m}");
        assert_eq!(m["wheel"], json!(1.0));
        // The next frame: still held, nothing new.
        i.key("ShiftLeft", "Shift", false); // the other shift key still holds it
        i.captured = true;
        let m = i.message();
        assert_eq!(m["keys"], json!(["Shift", "W"]));
        assert_eq!((m["pressed"].clone(), m["released"].clone()), (json!([]), json!([])));
        assert_eq!((m["dx"].as_f64(), m["wheel"].as_f64(), m["u"].as_f64(), m["captured"].as_bool()), (Some(0.0), Some(0.0), Some(0.5), Some(true)));
        i.key("ShiftRight", "Shift", false);
        i.key("KeyQ", "Q", false); // an up without its down (it went down in another window)
        assert_eq!(i.message()["released"], json!(["Shift"]));
        // Focus lost: all let go.
        i.release_all();
        let m = i.message();
        assert_eq!((m["keys"].clone(), m["buttons"].clone(), m["released"].clone()), (json!([]), json!([]), json!(["W", "Mouse0"])));
    }

    #[test]
    fn the_launch_message() {
        assert_eq!(launch_message(None), json!({ "t": "launch", "params": {} }));
        assert_eq!(launch_message(Some(&json!({ "room": "K7Q4MZ" }))), json!({ "t": "launch", "params": { "room": "K7Q4MZ" } }));
        assert_eq!(launch_message(Some(&json!("x")))["params"], json!({}));
    }
}
