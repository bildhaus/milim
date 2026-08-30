//! Computer-use tools: screen capture + mouse/keyboard control.
//!
//! This is the "computer use" layer - the agent can see the screen (screenshot)
//! and drive the real mouse/keyboard (enigo). It is **off by default**: every
//! tool checks a shared gate that the GUI's "Computer" toggle flips, so the
//! agent can't move your mouse unless you explicitly opt in.
//!
//! Compiled only with the `computer-use` feature. To fully close the loop a
//! vision model must *see* the screenshot; today the tool saves a PNG and
//! returns its path + dimensions (a vision model can be pointed at it).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

#[cfg(not(target_os = "macos"))]
use enigo::Keyboard;
#[cfg(any(not(target_os = "macos"), test))]
use enigo::Key;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Mouse, Settings};
use milim_core::{Error, Result};
use milim_tools::{Tool, ToolEffect};

/// Shared on/off switch for the whole computer-use layer.
pub type ComputerGate = Arc<AtomicBool>;

fn arg_i32(args: &Value, key: &str) -> Result<i32> {
    let value = args
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::InvalidRequest(format!("missing integer argument: {key}")))?;
    i32::try_from(value)
        .map_err(|_| Error::InvalidRequest(format!("{key} is outside the supported range")))
}

fn ensure_enabled(gate: &ComputerGate) -> Result<()> {
    if gate.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err(Error::InvalidRequest(
            "Computer Use is disabled - enable the 'Computer' toggle in the toolbar first".into(),
        ))
    }
}

fn new_enigo() -> Result<Enigo> {
    Enigo::new(&Settings::default()).map_err(|e| Error::Other(format!("input init failed: {e}")))
}

pub(crate) fn type_text_input(text: &str) -> Result<usize> {
    #[cfg(target_os = "macos")]
    {
        macos_type_text(text)?;
        Ok(text.len())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut e = new_enigo()?;
        e.text(text)
            .map_err(|e| Error::Other(format!("type_text: {e}")))?;
        Ok(text.len())
    }
}

fn press_key_input(name: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos_press_key(name)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let key =
            map_key(name).ok_or_else(|| Error::InvalidRequest(format!("unknown key: {name}")))?;
        let mut e = new_enigo()?;
        e.key(key, Direction::Click)
            .map_err(|e| Error::Other(format!("key_press: {e}")))
    }
}

/// All computer-use tools bound to the shared gate, saving captures to `dir`.
pub fn computer_tools(gate: ComputerGate, captures_dir: PathBuf) -> Vec<Arc<dyn Tool>> {
    let _ = std::fs::create_dir_all(&captures_dir);
    vec![
        Arc::new(ScreenshotTool {
            gate: gate.clone(),
            dir: captures_dir,
            counter: AtomicU64::new(0),
        }),
        Arc::new(MouseMoveTool { gate: gate.clone() }),
        Arc::new(MouseClickTool { gate: gate.clone() }),
        Arc::new(TypeTextTool { gate: gate.clone() }),
        Arc::new(KeyTool { gate: gate.clone() }),
        Arc::new(ScrollTool { gate }),
    ]
}

/// Capture a monitor to a PNG and return its path + dimensions.
pub struct ScreenshotTool {
    gate: ComputerGate,
    dir: PathBuf,
    counter: AtomicU64,
}
#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot"
    }
    fn description(&self) -> &str {
        "Capture a screenshot of the screen. Returns the saved file path, dimensions, and the image itself so a vision model can see what's on screen. Optional 'monitor' index (default 0)."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"monitor":{"type":"integer","description":"monitor index, default 0"}}})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let monitor = args.get("monitor").and_then(Value::as_i64).unwrap_or(0);
        let idx = usize::try_from(monitor)
            .map_err(|_| Error::InvalidRequest("monitor must be non-negative".into()))?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let path = self
            .dir
            .join(format!("shot-{}-{n}.png", std::process::id()));
        let path2 = path.clone();
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || capture(idx, &path2, &gate))
            .await
            .map_err(|e| Error::Other(format!("capture task failed: {e}")))?
    }
}

fn capture(idx: usize, path: &std::path::Path, gate: &ComputerGate) -> Result<Value> {
    use base64::Engine;

    let monitors =
        xcap::Monitor::all().map_err(|e| Error::Other(format!("enumerate monitors: {e}")))?;
    let monitor = monitors
        .into_iter()
        .nth(idx)
        .ok_or_else(|| Error::InvalidRequest(format!("no monitor at index {idx}")))?;
    let shot = monitor
        .capture_image()
        .map_err(|e| Error::Other(format!("capture failed: {e}")))?;
    ensure_enabled(gate)?;
    let (w, h) = (shot.width(), shot.height());

    // Full-resolution PNG to disk for reference.
    shot.save(path)
        .map_err(|e| Error::Other(format!("save PNG failed: {e}")))?;
    if let Some(dir) = path.parent() {
        prune_captures(dir, path, 50);
    }

    // Downscaled PNG (max 1568px - the practical vision-model cap) as a base64
    // payload. The agent loop strips this `image` field out of the visible
    // result and forwards it to the model as an image message.
    const MAX_DIM: u32 = 1568;
    let dynimg = image::DynamicImage::ImageRgba8(shot);
    let dynimg = if w > MAX_DIM || h > MAX_DIM {
        dynimg.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Triangle)
    } else {
        dynimg
    };
    let mut bytes = std::io::Cursor::new(Vec::new());
    dynimg
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| Error::Other(format!("encode PNG failed: {e}")))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());

    Ok(json!({
        "path": path.to_string_lossy(),
        "width": w,
        "height": h,
        "image": { "mime": "image/png", "data": data },
    }))
}

fn prune_captures(dir: &std::path::Path, current: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("png"))
        .filter(|entry| entry.path() != current)
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(modified, _)| *modified);
    let remove = files.len().saturating_sub(keep.saturating_sub(1));
    for (_, path) in files.into_iter().take(remove) {
        let _ = std::fs::remove_file(path);
    }
}

/// Move the mouse to absolute screen coordinates.
pub struct MouseMoveTool {
    gate: ComputerGate,
}
#[async_trait]
impl Tool for MouseMoveTool {
    fn name(&self) -> &str {
        "mouse_move"
    }
    fn description(&self) -> &str {
        "Move the mouse cursor to absolute screen pixel coordinates (x, y)."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"x":{"type":"integer"},"y":{"type":"integer"}},"required":["x","y"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let (x, y) = (arg_i32(&args, "x")?, arg_i32(&args, "y")?);
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || -> Result<Value> {
            ensure_enabled(&gate)?;
            let mut e = new_enigo()?;
            e.move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| Error::Other(format!("move_mouse: {e}")))?;
            Ok(json!({ "moved": [x, y] }))
        })
        .await
        .map_err(|e| Error::Other(format!("input task failed: {e}")))?
    }
}

/// Click a mouse button (optionally moving to x,y first).
pub struct MouseClickTool {
    gate: ComputerGate,
}
#[async_trait]
impl Tool for MouseClickTool {
    fn name(&self) -> &str {
        "mouse_click"
    }
    fn description(&self) -> &str {
        "Click a mouse button at the current position, or at (x, y) if given. button: left|right|middle (default left)."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "x":{"type":"integer"},"y":{"type":"integer"},
            "button":{"type":"string","enum":["left","right","middle"]}
        }})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let position = match (args.get("x"), args.get("y")) {
            (None, None) => None,
            (Some(_), Some(_)) => Some((arg_i32(&args, "x")?, arg_i32(&args, "y")?)),
            _ => {
                return Err(Error::InvalidRequest(
                    "mouse_click requires both x and y when either is provided".into(),
                ))
            }
        };
        let button = map_button(args.get("button").and_then(Value::as_str).unwrap_or("left"))?;
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || -> Result<Value> {
            ensure_enabled(&gate)?;
            let mut e = new_enigo()?;
            if let Some((x, y)) = position {
                e.move_mouse(x, y, Coordinate::Abs)
                    .map_err(|e| Error::Other(format!("move_mouse: {e}")))?;
            }
            e.button(button, Direction::Click)
                .map_err(|e| Error::Other(format!("click: {e}")))?;
            Ok(json!({ "clicked": true }))
        })
        .await
        .map_err(|e| Error::Other(format!("input task failed: {e}")))?
    }
}

/// Type a string of text at the current focus.
pub struct TypeTextTool {
    gate: ComputerGate,
}
#[async_trait]
impl Tool for TypeTextTool {
    fn name(&self) -> &str {
        "type_text"
    }
    fn description(&self) -> &str {
        "Type a string of text into the currently focused field."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("missing string argument: text".into()))?
            .to_string();
        let len = text.len();
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || -> Result<Value> {
            ensure_enabled(&gate)?;
            type_text_input(&text)?;
            Ok(json!({ "typed": len }))
        })
        .await
        .map_err(|e| Error::Other(format!("input task failed: {e}")))?
    }
}

/// Press a single named key (enter, tab, escape, arrows, or a character).
pub struct KeyTool {
    gate: ComputerGate,
}
#[async_trait]
impl Tool for KeyTool {
    fn name(&self) -> &str {
        "key_press"
    }
    fn description(&self) -> &str {
        "Press a key: enter, tab, escape, space, backspace, delete, up, down, left, right, home, end, pageup, pagedown, or a single character."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let name = args
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("missing string argument: key".into()))?
            .to_string();
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || -> Result<Value> {
            ensure_enabled(&gate)?;
            press_key_input(&name)?;
            Ok(json!({ "pressed": name }))
        })
        .await
        .map_err(|e| Error::Other(format!("input task failed: {e}")))?
    }
}

/// Scroll vertically (positive = down) or horizontally.
pub struct ScrollTool {
    gate: ComputerGate,
}
#[async_trait]
impl Tool for ScrollTool {
    fn name(&self) -> &str {
        "scroll"
    }
    fn description(&self) -> &str {
        "Scroll the mouse wheel. 'amount' lines (positive = down), 'axis': vertical|horizontal (default vertical)."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "amount":{"type":"integer"},
            "axis":{"type":"string","enum":["vertical","horizontal"]}
        },"required":["amount"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.gate)?;
        let amount = arg_i32(&args, "amount")?;
        let axis = match args
            .get("axis")
            .and_then(Value::as_str)
            .unwrap_or("vertical")
        {
            "vertical" => Axis::Vertical,
            "horizontal" => Axis::Horizontal,
            other => {
                return Err(Error::InvalidRequest(format!(
                    "unknown scroll axis: {other}"
                )))
            }
        };
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || -> Result<Value> {
            ensure_enabled(&gate)?;
            let mut e = new_enigo()?;
            e.scroll(amount, axis)
                .map_err(|e| Error::Other(format!("scroll: {e}")))?;
            Ok(json!({ "scrolled": amount }))
        })
        .await
        .map_err(|e| Error::Other(format!("input task failed: {e}")))?
    }
}

fn map_button(name: &str) -> Result<Button> {
    match name.trim().to_lowercase().as_str() {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        other => Err(Error::InvalidRequest(format!(
            "unknown mouse button: {other}"
        ))),
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn map_key(name: &str) -> Option<Key> {
    let n = name.trim().to_lowercase();
    Some(match n.as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        _ => {
            let mut chars = n.chars();
            let c = chars.next()?;
            if chars.next().is_none() {
                Key::Unicode(c)
            } else {
                return None;
            }
        }
    })
}

#[cfg(target_os = "macos")]
fn macos_type_text(text: &str) -> Result<()> {
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| Error::Other("input event source failed".into()))?;
    for ch in text.chars() {
        let text = ch.to_string();
        let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| Error::Other("key down event failed".into()))?;
        down.set_string(&text);
        down.post(CGEventTapLocation::HID);
        let up = CGEvent::new_keyboard_event(source.clone(), 0, false)
            .map_err(|_| Error::Other("key up event failed".into()))?;
        up.set_string(&text);
        up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_press_key(name: &str) -> Result<()> {
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let n = name.trim().to_lowercase();
    if let Some(code) = macos_key_code(&n) {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| Error::Other("input event source failed".into()))?;
        let down = CGEvent::new_keyboard_event(source.clone(), code, true)
            .map_err(|_| Error::Other("key down event failed".into()))?;
        down.post(CGEventTapLocation::HID);
        let up = CGEvent::new_keyboard_event(source, code, false)
            .map_err(|_| Error::Other("key up event failed".into()))?;
        up.post(CGEventTapLocation::HID);
        return Ok(());
    }
    if n.chars().count() == 1 {
        return macos_type_text(&n);
    }
    Err(Error::InvalidRequest(format!("unknown key: {name}")))
}

#[cfg(target_os = "macos")]
fn macos_key_code(name: &str) -> Option<core_graphics::event::CGKeyCode> {
    use core_graphics::event::KeyCode;

    Some(match name {
        "enter" | "return" => KeyCode::RETURN,
        "tab" => KeyCode::TAB,
        "escape" | "esc" => KeyCode::ESCAPE,
        "space" => KeyCode::SPACE,
        "backspace" => KeyCode::DELETE,
        "delete" | "del" => KeyCode::FORWARD_DELETE,
        "up" => KeyCode::UP_ARROW,
        "down" => KeyCode::DOWN_ARROW,
        "left" => KeyCode::LEFT_ARROW,
        "right" => KeyCode::RIGHT_ARROW,
        "home" => KeyCode::HOME,
        "end" => KeyCode::END,
        "pageup" => KeyCode::PAGE_UP,
        "pagedown" => KeyCode::PAGE_DOWN,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn temp_capture_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "milim-computer-use-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn registers_expected_computer_use_tools() {
        let gate = Arc::new(AtomicBool::new(false));
        let dir = temp_capture_dir();
        let tools = computer_tools(gate, dir.clone());
        let mut names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "key_press",
                "mouse_click",
                "mouse_move",
                "screenshot",
                "scroll",
                "type_text"
            ]
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disabled_gate_blocks_all_computer_use_tools_before_side_effects() {
        let gate = Arc::new(AtomicBool::new(false));
        let dir = temp_capture_dir();
        let tools = computer_tools(gate, dir.clone());
        let cases = [
            ("screenshot", json!({ "monitor": 0 })),
            ("mouse_move", json!({ "x": 1, "y": 1 })),
            ("mouse_click", json!({})),
            ("type_text", json!({ "text": "not typed" })),
            ("key_press", json!({ "key": "enter" })),
            ("scroll", json!({ "amount": 1 })),
        ];

        for (name, args) in cases {
            let tool = tools
                .iter()
                .find(|tool| tool.name() == name)
                .unwrap_or_else(|| panic!("missing computer-use tool: {name}"));
            let err = block_on(tool.invoke(args)).unwrap_err().to_string();
            assert!(
                err.contains("Computer Use is disabled"),
                "{name} returned unexpected error: {err}"
            );
        }

        let captures = std::fs::read_dir(&dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(captures, 0, "disabled screenshot must not write captures");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn computer_use_key_mapping_accepts_common_windows_navigation_keys() {
        for key in [
            "enter",
            "tab",
            "escape",
            "space",
            "backspace",
            "delete",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
            "pageup",
            "pagedown",
            "a",
        ] {
            assert!(map_key(key).is_some(), "{key} should map to an enigo key");
        }
        assert!(map_key("not-a-key").is_none());
    }

    #[test]
    fn rejects_wrapped_coordinates_and_unknown_actions() {
        assert!(arg_i32(&json!({"x": i64::MAX}), "x").is_err());
        assert!(map_button("left").is_ok());
        assert!(map_button("sideways").is_err());
    }
}
